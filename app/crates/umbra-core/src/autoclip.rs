//! Auto-clipping. Detectors (such as [`crate::cs2`]) report what happens in a game as
//! [`GameEvent`]s on a bounded channel, never waiting: a full queue drops the event. The
//! [`Policy`] groups events into moments: a round in games with rounds, else events close
//! together. When a moment is over it decides whether it makes a clip (enough kills, or an
//! event set to make one), pads it, and keeps any event from ending up in two clips. The
//! controller runs it on a thread of its own and saves clips through the same path as the
//! clip hotkey.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::mpsc::{SyncSender, TrySendError};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::settings::AutoClipPrefs;

/// Events waiting for the policy.
pub const QUEUE: usize = 64;

/// How long a round's events wait for the end of the round, after the last of them, before
/// they are decided on anyway (the game closed or stopped reporting).
const ROUND_WAIT: f64 = 180.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EventKind {
    Kill,
    Headshot,
    Knife,
    Zeus,
    /// One bullet, two kills.
    Collateral,
    /// A kill while flashed.
    BlindKill,
    Death,
    RoundWin,
    /// Not a moment but a marker: the round in [`GameEvent::round`] is over, so what was seen
    /// of it is decided on.
    RoundDone,
}

impl EventKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Kill => "kill",
            Self::Headshot => "headshot",
            Self::Knife => "knife kill",
            Self::Zeus => "Zeus kill",
            Self::Collateral => "collateral",
            Self::BlindKill => "blind kill",
            Self::Death => "death",
            Self::RoundWin => "round win",
            Self::RoundDone => "round over",
        }
    }
}

/// Special kills in the order a clip's name lists them, with their names there.
const SPECIAL: [(EventKind, &str); 4] = [
    (EventKind::Knife, "Knife kill"),
    (EventKind::Zeus, "Zeus kill"),
    (EventKind::Collateral, "Collateral"),
    (EventKind::BlindKill, "Blind kill"),
];

#[derive(Clone, Debug)]
pub struct GameEvent {
    pub kind: EventKind,
    /// When the detector saw it.
    pub at: Instant,
    /// 0 to 1: how sure the detector is.
    pub confidence: f32,
    /// The detector, such as `cs2-gsi`.
    pub source: &'static str,
    /// The round it happened in, in games with rounds: a round's events make one clip,
    /// decided when the round is over. `None` merges events close together instead.
    pub round: Option<u32>,
    /// Details (the map, kills this round, the weapon), kept in the clip's tags.
    pub metadata: Vec<(String, String)>,
}

/// Hands an event to the policy without waiting. False when it was dropped.
pub fn send(events: &SyncSender<GameEvent>, event: GameEvent) -> bool {
    match events.try_send(event) {
        Ok(()) => true,
        Err(TrySendError::Full(event)) => {
            log::warn!(
                "auto-clip: queue full, dropped a {} from {}",
                event.kind.as_str(),
                event.source
            );
            false
        }
        Err(TrySendError::Disconnected(_)) => false,
    }
}

/// A clip the policy decided on. Times are seconds on the policy's own clock.
#[derive(Clone, Debug)]
pub struct ClipPlan {
    pub from: f64,
    pub to: f64,
    /// The round it is of, in games with rounds.
    pub round: Option<u32>,
    /// What is in it, with when each happened.
    pub events: Vec<(f64, GameEvent)>,
}

impl ClipPlan {
    fn count(&self, kind: EventKind) -> usize {
        self.events.iter().filter(|(_, e)| e.kind == kind).count()
    }

    /// What happened, for the clip's name: `Triple kill`, `Knife kill`, `Ace, Collateral`.
    pub fn label(&self) -> String {
        let kills = self.count(EventKind::Kill);
        let headshots = self.count(EventKind::Headshot);
        let specials: Vec<&str> = SPECIAL
            .iter()
            .filter(|(kind, _)| self.count(*kind) > 0)
            .map(|(_, name)| *name)
            .collect();
        let mut parts: Vec<String> = Vec::new();
        match kills {
            0 if headshots == 1 => parts.push("Headshot".into()),
            0 if headshots > 1 => parts.push(format!("{headshots} headshots")),
            0 => {}
            // The special kill names it.
            1 if !specials.is_empty() => {}
            1 if headshots > 0 => parts.push("Headshot".into()),
            1 => parts.push("Kill".into()),
            2 => parts.push("Double kill".into()),
            3 => parts.push("Triple kill".into()),
            4 => parts.push("Quad kill".into()),
            5 if self.round.is_some() => parts.push("Ace".into()),
            n => parts.push(format!("{n} kills")),
        }
        parts.extend(specials.into_iter().map(String::from));
        if self.count(EventKind::RoundWin) > 0 {
            parts.push("Round win".into());
        }
        if parts.is_empty() && self.count(EventKind::Death) > 0 {
            parts.push("Death".into());
        }
        parts.join(", ")
    }

    /// What the clip's file records about it (shown in its properties).
    pub fn tags(&self) -> Vec<(String, String)> {
        let mut sources: Vec<&str> = self.events.iter().map(|(_, e)| e.source).collect();
        sources.dedup();
        // ponytail: offsets are from the asked-for start; the file starts on the keyframe
        // before it (up to one keyframe interval earlier). Exact if markers ever need it.
        let times: Vec<String> = self
            .events
            .iter()
            .map(|(t, e)| format!("{} {:.1}", e.kind.as_str(), t - self.from))
            .collect();
        let mut tags = vec![
            ("event".to_string(), self.label()),
            ("event_count".into(), self.events.len().to_string()),
            ("event_source".into(), sources.join(", ")),
            ("event_times".into(), times.join(", ")),
        ];
        // The newest value of each detail wins (kills this round grows with each kill).
        for (_, event) in &self.events {
            for (key, value) in &event.metadata {
                let key = format!("event_{key}");
                match tags.iter_mut().find(|(k, _)| *k == key) {
                    Some(tag) => tag.1 = value.clone(),
                    None => tags.push((key, value.clone())),
                }
            }
        }
        tags
    }
}

/// What the policy made of a moment.
#[derive(Debug)]
pub enum Decision {
    Save(ClipPlan),
    /// No clip: what the moment was, and why.
    Skip {
        label: String,
        why: String,
    },
}

/// Decides which moments become clips. Pure: it is told the time, so tests can drive it.
pub struct Policy {
    pub prefs: AutoClipPrefs,
    /// The longest clip the replay buffer can give.
    pub max_secs: f64,
    /// Events of the moment not decided on yet.
    open: Vec<(f64, GameEvent)>,
    /// The round they are of.
    round: Option<u32>,
    /// When that round was over.
    done_at: Option<f64>,
    /// End of the last clip saved: an event before it is in that clip already.
    saved_to: f64,
    decided: Vec<Decision>,
}

type Moment = [(f64, EventKind)];

impl Policy {
    pub fn new(prefs: AutoClipPrefs, max_secs: f64) -> Self {
        Self {
            prefs,
            max_secs,
            open: Vec::new(),
            round: None,
            done_at: None,
            saved_to: f64::NEG_INFINITY,
            decided: Vec::new(),
        }
    }

    /// Kinds switched on make clips, and their padding counts. The others only add to the name
    /// of a clip something else made.
    fn makes_clips(&self, kind: EventKind) -> bool {
        self.prefs.rule(kind).enabled
    }

    /// Start and end of a clip of `events`, padded by each kind's rule: from the events that
    /// make clips, or all of them when none does.
    fn span(&self, events: &Moment) -> (f64, f64) {
        let shaping = events.iter().any(|(_, k)| self.makes_clips(*k));
        events
            .iter()
            .filter(|(_, k)| !shaping || self.makes_clips(*k))
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(from, to), (t, k)| {
                let rule = self.prefs.rule(*k);
                (
                    from.min(t - rule.pre_secs as f64),
                    to.max(t + rule.post_secs as f64),
                )
            })
    }

    /// When a moment is decided on: a round once it is over and its last event has had its
    /// padding (or long after its last event), else once the last event has had its padding
    /// and the merge window has passed.
    fn due(&self, events: &Moment) -> f64 {
        let last = events
            .iter()
            .map(|(t, _)| *t)
            .fold(f64::NEG_INFINITY, f64::max);
        let padded = self.span(events).1;
        match (self.round, self.done_at) {
            (Some(_), Some(done)) => padded.max(done),
            (Some(_), None) => last + ROUND_WAIT,
            (None, _) => padded.max(last + self.prefs.merge_secs as f64),
        }
    }

    fn open_kinds(&self) -> Vec<(f64, EventKind)> {
        self.open.iter().map(|(t, e)| (*t, e.kind)).collect()
    }

    /// When the open moment is decided on, if there is one.
    pub fn next_deadline(&self) -> Option<f64> {
        (!self.open.is_empty()).then(|| self.due(&self.open_kinds()))
    }

    /// Events in the open moment.
    pub fn open_events(&self) -> usize {
        self.open.len()
    }

    /// Takes an event seen at `t`. One that ends the open moment (another round, or past its
    /// time) has it decided on first; [`take`](Self::take) hands out what was decided. An
    /// event left out comes back as `Err` with why.
    pub fn push(&mut self, t: f64, event: GameEvent, now: f64) -> Result<(), &'static str> {
        if !self.prefs.enabled {
            return Err("auto-clip is off");
        }
        if event.kind == EventKind::RoundDone {
            if !self.open.is_empty() && self.round.is_some() && event.round == self.round {
                self.done_at = Some(t);
            }
            return Ok(());
        }
        if event.confidence < self.prefs.rule(event.kind).min_confidence {
            return Err("below the minimum confidence");
        }
        if t <= self.saved_to {
            return Err("already in a saved clip");
        }
        if let Some(due) = self.next_deadline() {
            let mut with = self.open_kinds();
            with.push((t, event.kind));
            // Without rounds, a chain of events longer than the buffer is cut into clips.
            let too_long =
                self.round.is_none() && self.due(&with) - self.span(&with).0 > self.max_secs;
            if event.round != self.round || t > due || too_long {
                // Ends where this event starts, so it is in one clip only.
                self.close(t, now);
            }
        }
        if self.open.is_empty() {
            self.round = event.round;
        }
        self.open.push((t, event));
        Ok(())
    }

    /// Decides on the open moment once it is due, and hands out everything decided.
    pub fn take(&mut self, now: f64) -> Vec<Decision> {
        if self.next_deadline().is_some_and(|due| due <= now) {
            self.close(now, now);
        }
        std::mem::take(&mut self.decided)
    }

    /// Decides on the open moment: a clip ending by `limit` and starting no earlier than the
    /// buffer reaches at `now`, or why not.
    fn close(&mut self, limit: f64, now: f64) {
        self.done_at = None;
        let events = std::mem::take(&mut self.open);
        let kinds: Vec<_> = events.iter().map(|(t, e)| (*t, e.kind)).collect();
        let (from, to) = self.span(&kinds);
        let to = to.min(limit).min(now);
        let from = from.max(now - self.max_secs).min(to);
        let mut plan = ClipPlan {
            from,
            to,
            round: self.round,
            events,
        };
        let kills = kinds.iter().filter(|(_, k)| *k == EventKind::Kill).count();
        let needed = self.prefs.min_kills.max(1) as usize;
        let makes_clip = (self.prefs.kill.enabled && kills >= needed)
            || kinds
                .iter()
                .any(|(_, k)| *k != EventKind::Kill && self.makes_clips(*k));
        let why = if !makes_clip {
            Some(if self.prefs.kill.enabled && kills > 0 {
                format!("{kills} of the {needed} kills a clip needs")
            } else {
                "nothing in it is set to make a clip".into()
            })
        } else if to - from < 1.0 {
            Some(
                "it already left the replay buffer; a longer clip length under Clip keeps it"
                    .into(),
            )
        } else {
            None
        };
        match why {
            None => {
                self.saved_to = to;
                // Something long after (a death) is not in the clip, so not in its name.
                plan.events
                    .retain(|(t, _)| (from - 0.01..=to + 0.01).contains(t));
                self.decided.push(Decision::Save(plan));
            }
            Some(why) => self.decided.push(Decision::Skip {
                label: plan.label(),
                why,
            }),
        }
    }
}

/// One line of the auto-clip log in Settings: an event as it came in, or what was done.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogEntry {
    pub unix_ms: u64,
    pub kind: Option<EventKind>,
    pub confidence: Option<f32>,
    pub source: Option<String>,
    pub message: String,
}

impl LogEntry {
    /// About `event` (stamped when it was seen) or, without one, about now.
    pub fn new(event: Option<&GameEvent>, message: impl Into<String>) -> Self {
        let when = SystemTime::now() - event.map_or(Default::default(), |e| e.at.elapsed());
        Self {
            unix_ms: when
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_millis() as u64),
            kind: event.map(|e| e.kind),
            confidence: event.map(|e| e.confidence),
            source: event.map(|e| e.source.to_string()),
            message: message.into(),
        }
    }
}

/// The newest entries of the auto-clip log, kept for Settings. Each is also written to
/// Umbra's log file and handed to `on_entry`, which shows it live.
pub struct Log {
    entries: Mutex<VecDeque<LogEntry>>,
    on_entry: Box<dyn Fn(&LogEntry) + Send + Sync>,
}

const LOG_LENGTH: usize = 200;

impl Log {
    pub fn new(on_entry: impl Fn(&LogEntry) + Send + Sync + 'static) -> Self {
        Self {
            entries: Mutex::new(VecDeque::new()),
            on_entry: Box::new(on_entry),
        }
    }

    pub fn add(&self, event: Option<&GameEvent>, message: impl Into<String>) {
        let entry = LogEntry::new(event, message);
        match event {
            Some(e) => log::info!(
                "auto-clip: {} from {} ({:.2}): {}",
                e.kind.as_str(),
                e.source,
                e.confidence,
                entry.message
            ),
            None => log::info!("auto-clip: {}", entry.message),
        }
        {
            let mut entries = self.entries.lock().unwrap();
            if entries.len() == LOG_LENGTH {
                entries.pop_front();
            }
            entries.push_back(entry.clone());
        }
        (self.on_entry)(&entry);
    }

    pub fn entries(&self) -> Vec<LogEntry> {
        self.entries.lock().unwrap().iter().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use EventKind::*;

    fn event(kind: EventKind, round: Option<u32>, confidence: f32) -> GameEvent {
        GameEvent {
            kind,
            at: Instant::now(),
            confidence,
            source: "test",
            round,
            metadata: Vec::new(),
        }
    }

    /// The defaults, switched on: every kill, knife, Zeus and collateral kills make clips,
    /// 8 s before and 3 s after, merged within 10 s without rounds.
    fn on() -> AutoClipPrefs {
        AutoClipPrefs {
            enabled: true,
            ..AutoClipPrefs::default()
        }
    }

    /// With a 60 s buffer.
    fn policy() -> Policy {
        Policy::new(on(), 60.0)
    }

    /// Feeds `events` (time, kind, round) in order, waking at each deadline the way the
    /// controller's thread does, and returns every decision with when it was made.
    fn run(policy: &mut Policy, events: &[(f64, EventKind, Option<u32>)]) -> Vec<(f64, Decision)> {
        let mut decided = Vec::new();
        let mut collect = |at: f64, policy: &mut Policy| {
            decided.extend(policy.take(at).into_iter().map(|d| (at, d)));
        };
        for &(t, kind, round) in events {
            while let Some(due) = policy.next_deadline().filter(|d| *d < t) {
                collect(due, policy);
            }
            let _ = policy.push(t, event(kind, round, 1.0), t);
            collect(t, policy);
        }
        while let Some(due) = policy.next_deadline() {
            collect(due, policy);
        }
        decided
    }

    fn saved(decided: &[(f64, Decision)]) -> Vec<(f64, &ClipPlan)> {
        decided
            .iter()
            .filter_map(|(at, d)| match d {
                Decision::Save(plan) => Some((*at, plan)),
                Decision::Skip { .. } => None,
            })
            .collect()
    }

    fn skipped(decided: &[(f64, Decision)]) -> Vec<(&str, &str)> {
        decided
            .iter()
            .filter_map(|(_, d)| match d {
                Decision::Skip { label, why } => Some((label.as_str(), why.as_str())),
                Decision::Save(_) => None,
            })
            .collect()
    }

    #[test]
    fn one_kill_is_padded_and_saved_after_the_merge_window() {
        let decided = run(&mut policy(), &[(100.0, Kill, None)]);
        let saved = saved(&decided);
        assert_eq!(saved.len(), 1);
        let (when, plan) = saved[0];
        assert_eq!(when, 110.0);
        assert_eq!((plan.from, plan.to), (92.0, 103.0));
        assert_eq!(plan.label(), "Kill");
    }

    #[test]
    fn kills_close_together_make_one_clip() {
        let decided = run(
            &mut policy(),
            &[
                (100.0, Kill, None),
                (106.0, Kill, None),
                (114.0, Kill, None),
            ],
        );
        let saved = saved(&decided);
        assert_eq!(saved.len(), 1);
        let (when, plan) = saved[0];
        assert_eq!(when, 124.0);
        assert_eq!((plan.from, plan.to), (92.0, 117.0));
        assert_eq!(plan.label(), "Triple kill");
        assert!(plan.tags().contains(&("event_count".into(), "3".into())));
    }

    #[test]
    fn kills_far_apart_make_two_clips() {
        let decided = run(&mut policy(), &[(100.0, Kill, None), (111.0, Kill, None)]);
        let saved = saved(&decided);
        assert_eq!(saved.len(), 2);
        assert_eq!((saved[0].1.from, saved[0].1.to), (92.0, 103.0));
        assert_eq!((saved[1].1.from, saved[1].1.to), (103.0, 114.0));
    }

    #[test]
    fn a_round_is_one_clip_however_far_apart_its_kills() {
        let decided = run(
            &mut Policy::new(on(), 120.0),
            &[
                (100.0, Kill, Some(4)),
                (140.0, Kill, Some(4)),
                // The kill that ends the round comes with the round's end.
                (171.0, Kill, Some(4)),
                (171.0, RoundDone, Some(4)),
            ],
        );
        let saved = saved(&decided);
        assert_eq!(saved.len(), 1);
        let (when, plan) = saved[0];
        // Saved once the last kill has had its 3 s after.
        assert_eq!(when, 174.0);
        assert_eq!((plan.from, plan.to), (92.0, 174.0));
        assert_eq!(plan.label(), "Triple kill");
    }

    #[test]
    fn a_round_below_the_kills_needed_makes_no_clip() {
        let mut prefs = on();
        prefs.min_kills = 3;
        let mut policy = Policy::new(prefs, 120.0);
        let round = |n: u32, kills: &[f64]| {
            let mut events: Vec<_> = kills.iter().map(|&t| (t, Kill, Some(n))).collect();
            events.push((kills[kills.len() - 1] + 20.0, RoundDone, Some(n)));
            events
        };
        let mut events = round(1, &[10.0, 30.0]);
        events.extend(round(2, &[200.0, 210.0, 250.0]));
        let decided = run(&mut policy, &events);
        assert_eq!(
            skipped(&decided),
            [("Double kill", "2 of the 3 kills a clip needs")]
        );
        let saved = saved(&decided);
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].1.label(), "Triple kill");
    }

    #[test]
    fn five_kills_in_a_round_are_an_ace() {
        let mut events: Vec<_> = (0..5)
            .map(|i| (10.0 + i as f64 * 5.0, Kill, Some(1)))
            .collect();
        events.push((40.0, RoundDone, Some(1)));
        let decided = run(&mut policy(), &events);
        assert_eq!(saved(&decided)[0].1.label(), "Ace");
        // Without rounds, five kills are just that.
        let events: Vec<_> = (0..5)
            .map(|i| (10.0 + i as f64 * 5.0, Kill, None))
            .collect();
        let decided = run(&mut policy(), &events);
        assert_eq!(saved(&decided)[0].1.label(), "5 kills");
    }

    #[test]
    fn the_next_round_ends_the_one_before() {
        let decided = run(
            &mut Policy::new(on(), 120.0),
            &[
                (10.0, Kill, Some(1)),
                (90.0, Kill, Some(2)),
                (100.0, RoundDone, Some(2)),
            ],
        );
        let saved = saved(&decided);
        assert_eq!(saved.len(), 2);
        assert_eq!(saved[0].0, 90.0);
        assert_eq!(saved[0].1.round, Some(1));
        assert_eq!(saved[1].1.round, Some(2));
    }

    #[test]
    fn a_special_kill_makes_a_clip_below_the_kills_needed() {
        let mut prefs = on();
        prefs.min_kills = 3;
        let mut policy = Policy::new(prefs.clone(), 60.0);
        let events = [
            (10.0, Kill, Some(1)),
            (10.0, Knife, Some(1)),
            (30.0, RoundDone, Some(1)),
        ];
        let decided = run(&mut policy, &events);
        assert_eq!(saved(&decided)[0].1.label(), "Knife kill");
        // Switched off, it only names a clip something else made.
        prefs.knife.enabled = false;
        let decided = run(&mut Policy::new(prefs, 60.0), &events);
        assert!(saved(&decided).is_empty());
        assert_eq!(skipped(&decided)[0].0, "Knife kill");
    }

    #[test]
    fn special_kills_and_headshots_are_named() {
        let events = [
            (10.0, Kill, Some(1)),
            (10.0, Headshot, Some(1)),
            (14.0, Kill, Some(1)),
            (14.0, Kill, Some(1)),
            (14.0, Collateral, Some(1)),
            (20.0, RoundDone, Some(1)),
        ];
        let decided = run(&mut policy(), &events);
        assert_eq!(saved(&decided)[0].1.label(), "Triple kill, Collateral");
        let decided = run(&mut policy(), &[(5.0, Kill, None), (5.0, Headshot, None)]);
        assert_eq!(saved(&decided)[0].1.label(), "Headshot");
    }

    #[test]
    fn padding_follows_each_kind() {
        let mut prefs = on();
        prefs.round_win.enabled = true;
        prefs.round_win.pre_secs = 2.0;
        prefs.round_win.post_secs = 6.0;
        let mut policy = Policy::new(prefs, 60.0);
        let decided = run(&mut policy, &[(100.0, Kill, None), (104.0, RoundWin, None)]);
        let saved = saved(&decided);
        assert_eq!(saved.len(), 1);
        assert_eq!((saved[0].1.from, saved[0].1.to), (92.0, 110.0));
        assert_eq!(saved[0].1.label(), "Kill, Round win");
    }

    #[test]
    fn events_that_are_off_neither_make_nor_stretch_a_clip() {
        // A death long after the kill, with deaths off: not in the clip, not in its name.
        let decided = run(
            &mut policy(),
            &[
                (10.0, Kill, Some(1)),
                (50.0, Death, Some(1)),
                (60.0, RoundDone, Some(1)),
            ],
        );
        let plan = saved(&decided)[0].1;
        assert_eq!((plan.from, plan.to), (2.0, 13.0));
        assert_eq!(plan.label(), "Kill");
        let decided = run(&mut policy(), &[(10.0, Death, None)]);
        assert_eq!(
            skipped(&decided),
            [("Death", "nothing in it is set to make a clip")]
        );
    }

    #[test]
    fn unsure_events_and_auto_clip_off_are_left_out() {
        let mut policy = policy();
        policy.prefs.kill.min_confidence = 0.8;
        assert!(policy.push(1.0, event(Kill, None, 0.5), 1.0).is_err());
        assert!(policy.next_deadline().is_none());
        assert!(policy.push(1.0, event(Kill, None, 0.9), 1.0).is_ok());
        policy.prefs.enabled = false;
        assert!(policy.push(2.0, event(Kill, None, 1.0), 2.0).is_err());
    }

    #[test]
    fn an_event_already_in_a_clip_never_makes_another() {
        let mut policy = policy();
        policy.push(100.0, event(Kill, None, 1.0), 100.0).unwrap();
        let decided = policy.take(110.0);
        assert!(matches!(&decided[..], [Decision::Save(plan)] if plan.to == 103.0));
        // Seen by a second detector, late.
        assert_eq!(
            policy
                .push(102.5, event(Kill, None, 1.0), 110.5)
                .unwrap_err(),
            "already in a saved clip"
        );
        assert!(policy.next_deadline().is_none());
    }

    #[test]
    fn a_chain_longer_than_the_buffer_is_split() {
        let mut policy = Policy::new(on(), 30.0);
        let kills: Vec<_> = (0..8)
            .map(|i| (100.0 + i as f64 * 6.0, Kill, None))
            .collect();
        let decided = run(&mut policy, &kills);
        let saved = saved(&decided);
        assert!(saved.len() >= 2, "{} clips", saved.len());
        for (when, plan) in &saved {
            assert!(plan.to - plan.from <= 30.0, "{plan:?}");
            assert!(when - plan.from <= 30.0, "start left the buffer: {plan:?}");
        }
        // Every kill is in exactly one clip.
        let total: usize = saved.iter().map(|(_, p)| p.count(Kill)).sum();
        assert_eq!(total, 8);
        for pair in saved.windows(2) {
            assert!(pair[0].1.to <= pair[1].1.events[0].0);
        }
    }

    #[test]
    fn the_start_never_reaches_past_the_buffer() {
        let mut policy = Policy::new(on(), 30.0);
        let decided = run(
            &mut policy,
            &[
                (10.0, Kill, Some(1)),
                (80.0, Kill, Some(1)),
                (90.0, RoundDone, Some(1)),
            ],
        );
        let (when, plan) = saved(&decided)[0];
        assert_eq!(plan.from, when - 30.0);
        // The first kill fell out of the buffer, so the name counts what the clip shows.
        assert_eq!(plan.label(), "Kill");
    }

    #[test]
    fn a_round_that_left_the_buffer_is_skipped() {
        let mut policy = Policy::new(on(), 30.0);
        // The game stopped reporting: no end of the round, so it waits until long after.
        let decided = run(
            &mut policy,
            &[
                (10.0, Kill, Some(1)),
                (300.0, Kill, Some(9)),
                (301.0, RoundDone, Some(9)),
            ],
        );
        assert_eq!(saved(&decided).len(), 1, "only the last one: {decided:?}");
        assert!(
            skipped(&decided)[0].1.contains("replay buffer"),
            "{decided:?}"
        );
    }

    #[test]
    fn tags_keep_the_newest_detail() {
        let mut first = event(Kill, None, 1.0);
        first.metadata = vec![("round_kills".into(), "1".into())];
        let mut second = event(Kill, None, 1.0);
        second.metadata = vec![("round_kills".into(), "2".into())];
        let plan = ClipPlan {
            from: 0.0,
            to: 10.0,
            round: None,
            events: vec![(8.0, first), (9.0, second)],
        };
        let tags = plan.tags();
        assert!(tags.contains(&("event_round_kills".into(), "2".into())));
        assert!(tags.contains(&("event_times".into(), "kill 8.0, kill 9.0".into())));
    }
}
