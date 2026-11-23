//! Counter-Strike 2's Game State Integration: the game itself posts its state as JSON to a
//! web address named in a config file in its `cfg` folder. Umbra listens on 127.0.0.1 only
//! and turns the changes into kills (with knife, Zeus, collateral and blind kills),
//! headshots, deaths, round wins and the ends of rounds. Valve made it for tournament
//! overlays and coaching tools; nothing touches the game's process.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, TcpListener};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::SyncSender;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use serde_json::Value;

use crate::autoclip::{self, EventKind, GameEvent};

pub const SOURCE: &str = "cs2-gsi";
pub const CONFIG_NAME: &str = "gamestate_integration_umbra.cfg";
const STEAM_APP_ID: u32 = 730;
/// The game posts a few kilobytes; anything far bigger is not from it.
const MAX_BODY: usize = 256 * 1024;
const MAX_HEAD: u64 = 16 * 1024;
/// How flashed (0 to 255) a kill must be to count as a blind kill.
const BLIND: i64 = 100;

/// Listens for the game's posts until dropped.
pub struct Cs2Gsi {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Cs2Gsi {
    /// `modes` are the game modes clipped (see `settings::CS2_MODES`).
    pub fn start(
        port: u16,
        token: String,
        modes: Vec<String>,
        events: SyncSender<GameEvent>,
        log: Arc<autoclip::Log>,
    ) -> Result<Self> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port))
            .with_context(|| format!("cannot listen on 127.0.0.1:{port}"))?;
        // Checked for new connections every 100 ms, so dropping it never waits long.
        listener.set_nonblocking(true)?;
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = stop.clone();
        let tracker = Tracker::new(modes);
        let thread = std::thread::Builder::new()
            .name("umbra-cs2-gsi".into())
            .spawn(move || serve(listener, &token, tracker, &events, &log, &thread_stop))?;
        Ok(Self {
            stop,
            thread: Some(thread),
        })
    }
}

impl Drop for Cs2Gsi {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn serve(
    listener: TcpListener,
    token: &str,
    mut tracker: Tracker,
    events: &SyncSender<GameEvent>,
    log: &autoclip::Log,
    stop: &AtomicBool,
) {
    let (mut heard, mut wrong_token, mut no_weapons) = (false, false, false);
    let mut place: Option<(String, String)> = None;
    while !stop.load(Ordering::Relaxed) {
        let stream = match listener.accept() {
            Ok((stream, _)) => stream,
            Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(100));
                continue;
            }
            Err(err) => {
                log::warn!("CS2: accept failed: {err}");
                std::thread::sleep(Duration::from_secs(1));
                continue;
            }
        };
        // The game keeps the connection for its next post. An idle one is closed after a
        // second, so stopping never waits longer; the game simply connects again.
        let _ = stream.set_nonblocking(false);
        let _ = stream.set_read_timeout(Some(Duration::from_secs(1)));
        let mut reader = BufReader::new(&stream);
        while !stop.load(Ordering::Relaxed) {
            let Ok(Some(body)) = read_request(&mut reader) else {
                break;
            };
            let at = Instant::now();
            if (&stream)
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
                .is_err()
            {
                break;
            }
            let Ok(state) = serde_json::from_slice::<Value>(&body) else {
                continue;
            };
            let text = |path: &str| state.pointer(path).and_then(Value::as_str);
            if text("/auth/token") != Some(token) {
                if !std::mem::replace(&mut wrong_token, true) {
                    log.add(
                        None,
                        "CS2 posts with another token: install the config file again and restart CS2",
                    );
                }
                continue;
            }
            if !std::mem::replace(&mut heard, true) {
                log.add(None, "CS2 is connected");
            }
            // The mode as the game names it, so the filter in Settings can be checked.
            let now = text("/map/name")
                .map(|map| (map.to_string(), text("/map/mode").unwrap_or("").to_string()));
            if now != place {
                if let Some((map, mode)) = &now {
                    let off = if tracker.clips(mode) {
                        ""
                    } else {
                        "; this mode is not clipped"
                    };
                    log.add(None, format!("CS2: {mode} on {map}{off}"));
                }
                place = now;
            }
            // A config file from before weapons were asked for.
            let alive = state
                .pointer("/player/state/health")
                .and_then(Value::as_i64)
                .is_some_and(|h| h > 0);
            if alive
                && text("/player/steamid") == text("/provider/steamid")
                && state.pointer("/player/weapons").is_none()
                && !std::mem::replace(&mut no_weapons, true)
            {
                log.add(
                    None,
                    "CS2 sends no weapons: install the config file again and restart CS2 to get knife, Zeus and collateral kills",
                );
            }
            for event in tracker.update(&state, at) {
                autoclip::send(events, event);
            }
        }
    }
}

/// Reads one HTTP request and returns its body; `None` when the connection closed.
fn read_request(reader: &mut impl BufRead) -> std::io::Result<Option<Vec<u8>>> {
    let mut head = reader.by_ref().take(MAX_HEAD);
    let mut line = String::new();
    if head.read_line(&mut line)? == 0 {
        return Ok(None);
    }
    let mut length = 0;
    loop {
        line.clear();
        if head.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            if name.trim().eq_ignore_ascii_case("content-length") {
                length = value.trim().parse().map_err(|_| invalid("bad length"))?;
            }
        }
    }
    if length > MAX_BODY {
        return Err(invalid("body too big"));
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    Ok(Some(body))
}

fn invalid(why: &str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, why)
}

/// The name Settings gives a game mode as the game reports it (see `settings::CS2_MODES`).
// ponytail: only competitive, casual and deathmatch are confirmed names; the others are
// CS:GO's. The log shows what the game sends, so a wrong one is easy to spot and fix here.
fn mode_group(mode: &str) -> &'static str {
    match mode {
        "competitive" | "premier" => "competitive",
        "scrimcomp2v2" | "wingman" => "wingman",
        "casual" => "casual",
        "deathmatch" => "deathmatch",
        "gungameprogressive" | "armsrace" => "armsrace",
        _ => "other",
    }
}

/// The weapon in hand: its name, type and bullets in the magazine.
fn active_weapon(state: &Value) -> Option<(&str, &str, Option<i64>)> {
    let weapon = state
        .pointer("/player/weapons")?
        .as_object()?
        .values()
        .find(|w| matches!(w["state"].as_str(), Some("active" | "reloading")))?;
    Some((
        weapon["name"].as_str().unwrap_or(""),
        weapon["type"].as_str().unwrap_or(""),
        weapon["ammo_clip"].as_i64(),
    ))
}

/// Turns the game's posts into events by comparing each with the one before.
#[derive(Default)]
pub struct Tracker {
    /// The game modes clipped.
    modes: Vec<String>,
    /// The map the counts are from; another map (or the menu) starts them over.
    map: Option<String>,
    /// Rounds seen since Umbra started listening; a round's events carry its number.
    round: u32,
    kills: Option<i64>,
    deaths: Option<i64>,
    /// Headshot kills this round.
    headshots: Option<i64>,
    team: Option<String>,
    round_phase: Option<String>,
    /// The weapon in hand at the last post and the bullets in it, to tell a collateral.
    weapon: Option<(String, i64)>,
    /// The game was showing someone else (you are dead and watching a teammate): what
    /// changed meanwhile, such as a grenade thrown before dying, happened at a moment not
    /// known, so it is taken in without an event.
    watching_other: bool,
}

impl Tracker {
    pub fn new(modes: Vec<String>) -> Self {
        Self {
            modes,
            ..Self::default()
        }
    }

    /// Whether the game mode the game reports as `mode` is clipped.
    pub fn clips(&self, mode: &str) -> bool {
        self.modes.iter().any(|m| m == mode_group(mode))
    }

    pub fn update(&mut self, state: &Value, at: Instant) -> Vec<GameEvent> {
        let text = |path: &str| state.pointer(path).and_then(Value::as_str);
        let number = |path: &str| state.pointer(path).and_then(Value::as_i64);
        let event = |kind, round, metadata| GameEvent {
            kind,
            at,
            confidence: 1.0,
            source: SOURCE,
            round,
            metadata,
        };
        let mut found = Vec::new();

        let map = text("/map/name");
        if map != self.map.as_deref() {
            // What was open of the last map is over.
            if self.map.is_some() {
                found.push(event(EventKind::RoundDone, Some(self.round), Vec::new()));
            }
            *self = Self {
                modes: std::mem::take(&mut self.modes),
                map: map.map(str::to_string),
                round: self.round + 1,
                ..Self::default()
            };
        }
        let mode = text("/map/mode").unwrap_or("");
        // Warmup kills count for nothing, nor do modes that are not clipped.
        let live = map.is_some() && text("/map/phase") != Some("warmup") && self.clips(mode);
        let phase = text("/round/phase");
        let entering = |p: &str| phase == Some(p) && self.round_phase.as_deref() != Some(p);
        let (starts, ends) = (entering("live"), entering("over"));
        if starts {
            self.round += 1;
        }
        // Deathmatch and Arms Race have no rounds to speak of: events close together merge.
        let round = (!matches!(mode_group(mode), "deathmatch" | "armsrace")).then_some(self.round);

        // `player` is whoever the game shows, which is someone else while you are dead.
        let me = text("/provider/steamid");
        let shown = text("/player/steamid");
        if shown.is_some() && shown == me {
            let kills = number("/player/match_stats/kills");
            let deaths = number("/player/match_stats/deaths");
            let headshots = number("/player/state/round_killhs");
            let weapon = active_weapon(state);
            if live && !self.watching_other {
                // A count that went down (a suicide, a new match) is no event.
                let rose = |before: Option<i64>, now: Option<i64>| match (before, now) {
                    (Some(before), Some(now)) if now > before => (now - before).min(5) as usize,
                    _ => 0,
                };
                let (name, kind, bullets) = weapon.unwrap_or_default();
                let details = || {
                    let mut details = Vec::new();
                    if let Some(map) = map {
                        details.push(("map".to_string(), map.to_string()));
                    }
                    if let Some(kills) = number("/player/state/round_kills") {
                        details.push(("round_kills".into(), kills.to_string()));
                    }
                    if !name.is_empty() {
                        let short = name.strip_prefix("weapon_").unwrap_or(name);
                        details.push(("weapon".into(), short.to_string()));
                    }
                    details
                };
                let new_kills = rose(self.kills, kills);
                let knife = kind == "Knife" || name.contains("knife") || name.contains("bayonet");
                let zeus = name.contains("taser");
                let blind = number("/player/state/flashed").is_some_and(|f| f >= BLIND);
                // Two kills for one bullet out of the same magazine.
                let collateral = new_kills >= 2
                    && bullets.is_some_and(|now| {
                        self.weapon
                            .as_ref()
                            .is_some_and(|(before, had)| before == name && had - now == 1)
                    });
                for _ in 0..new_kills {
                    found.push(event(EventKind::Kill, round, details()));
                    for (special, yes) in [
                        (EventKind::Knife, knife),
                        (EventKind::Zeus, zeus),
                        (EventKind::BlindKill, blind),
                    ] {
                        if yes {
                            found.push(event(special, round, details()));
                        }
                    }
                }
                if collateral {
                    found.push(event(EventKind::Collateral, round, details()));
                }
                for _ in 0..rose(self.headshots, headshots) {
                    found.push(event(EventKind::Headshot, round, details()));
                }
                for _ in 0..rose(self.deaths, deaths) {
                    found.push(event(EventKind::Death, round, details()));
                }
            }
            self.kills = kills;
            self.deaths = deaths;
            self.headshots = headshots;
            self.weapon = weapon.and_then(|(name, _, bullets)| Some((name.to_string(), bullets?)));
            self.team = text("/player/team").map(str::to_string);
            self.watching_other = false;
        } else if shown.is_some() {
            self.watching_other = true;
        }

        // After the kills: the one that ends a round comes with its end.
        if ends {
            let won = text("/round/win_team")
                .zip(self.team.as_deref())
                .is_some_and(|(winner, team)| winner.eq_ignore_ascii_case(team));
            if live && won {
                found.push(event(EventKind::RoundWin, round, Vec::new()));
            }
            if round.is_some() {
                found.push(event(EventKind::RoundDone, round, Vec::new()));
            }
        }
        self.round_phase = phase.map(str::to_string);
        found
    }
}

/// The config file that makes the game post to Umbra.
pub fn config_text(port: u16, token: &str) -> String {
    format!(
        r#""Umbra auto-clip"
{{
    "uri"       "http://127.0.0.1:{port}/"
    "timeout"   "1.0"
    "buffer"    "0.1"
    "throttle"  "0.1"
    "heartbeat" "30.0"
    "auth"
    {{
        "token" "{token}"
    }}
    "data"
    {{
        "provider"           "1"
        "map"                "1"
        "round"              "1"
        "player_id"          "1"
        "player_state"       "1"
        "player_match_stats" "1"
        "player_weapons"     "1"
    }}
}}
"#
    )
}

/// The game's `cfg` folder, found through the Steam libraries.
pub fn config_dir() -> Option<PathBuf> {
    crate::gamedb::steam_app_dir(STEAM_APP_ID)
        .map(|game| game.join(r"game\csgo\cfg"))
        .filter(|dir| dir.is_dir())
}

/// Writes the config file into the game's `cfg` folder. The game reads it when it starts.
pub fn install_config(port: u16, token: &str) -> Result<PathBuf> {
    let dir = config_dir()
        .ok_or_else(|| anyhow!("Counter-Strike 2 was not found in your Steam libraries"))?;
    let path = dir.join(CONFIG_NAME);
    std::fs::write(&path, config_text(port, token))
        .with_context(|| format!("cannot write {}", path.display()))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use EventKind::*;
    use serde_json::json;

    const ME: &str = "76561198000000001";
    const MATE: &str = "76561198000000002";
    const AK: (&str, &str, Option<i64>) = ("weapon_ak47", "Rifle", Some(30));

    struct Post {
        shown: &'static str,
        map_phase: &'static str,
        mode: &'static str,
        kills: i64,
        deaths: i64,
        headshots: i64,
        round_phase: &'static str,
        winner: Option<&'static str>,
        weapon: (&'static str, &'static str, Option<i64>),
        flashed: i64,
    }

    impl Default for Post {
        fn default() -> Self {
            Self {
                shown: ME,
                map_phase: "live",
                mode: "competitive",
                kills: 0,
                deaths: 0,
                headshots: 0,
                round_phase: "live",
                winner: None,
                weapon: AK,
                flashed: 0,
            }
        }
    }

    impl Post {
        fn json(&self) -> Value {
            let mut round = json!({ "phase": self.round_phase });
            if let Some(winner) = self.winner {
                round["win_team"] = json!(winner);
            }
            let (name, kind, bullets) = self.weapon;
            json!({
                "provider": { "name": "Counter-Strike: Global Offensive", "appid": 730, "steamid": ME },
                "map": { "name": "de_dust2", "mode": self.mode, "phase": self.map_phase, "round": 3 },
                "round": round,
                "player": {
                    "steamid": self.shown,
                    "team": "CT",
                    "state": {
                        "health": 100,
                        "flashed": self.flashed,
                        "round_kills": self.kills,
                        "round_killhs": self.headshots
                    },
                    "match_stats": { "kills": self.kills, "deaths": self.deaths },
                    "weapons": {
                        "weapon_0": { "name": "weapon_knife", "type": "Knife", "state": "holstered" },
                        "weapon_1": { "name": name, "type": kind, "state": "active", "ammo_clip": bullets }
                    }
                },
                "auth": { "token": "t" }
            })
        }
    }

    fn every_mode() -> Tracker {
        Tracker::new(crate::settings::CS2_MODES.map(String::from).to_vec())
    }

    /// The kinds found, without the ends of rounds.
    fn kinds(tracker: &mut Tracker, post: Post) -> Vec<EventKind> {
        tracker
            .update(&post.json(), Instant::now())
            .into_iter()
            .map(|e| e.kind)
            .filter(|k| *k != RoundDone)
            .collect()
    }

    #[test]
    fn kills_and_headshots_are_counted_from_the_change() {
        let mut t = every_mode();
        assert!(kinds(&mut t, Post::default()).is_empty());
        assert_eq!(
            kinds(
                &mut t,
                Post {
                    kills: 1,
                    ..Post::default()
                }
            ),
            [Kill]
        );
        assert_eq!(
            kinds(
                &mut t,
                Post {
                    kills: 3,
                    headshots: 1,
                    ..Post::default()
                }
            ),
            [Kill, Kill, Headshot]
        );
        let found = t.update(
            &Post {
                kills: 4,
                headshots: 1,
                ..Post::default()
            }
            .json(),
            Instant::now(),
        );
        for detail in [
            ("round_kills", "4"),
            ("map", "de_dust2"),
            ("weapon", "ak47"),
        ] {
            assert!(
                found[0]
                    .metadata
                    .contains(&(detail.0.into(), detail.1.into())),
                "{detail:?}"
            );
        }
        // A suicide takes a kill away; the next kill still counts.
        assert!(
            kinds(
                &mut t,
                Post {
                    kills: 3,
                    ..Post::default()
                }
            )
            .is_empty()
        );
        assert_eq!(
            kinds(
                &mut t,
                Post {
                    kills: 4,
                    ..Post::default()
                }
            ),
            [Kill]
        );
    }

    #[test]
    fn the_first_post_is_only_a_starting_point() {
        let mut t = every_mode();
        assert!(
            kinds(
                &mut t,
                Post {
                    kills: 12,
                    deaths: 4,
                    ..Post::default()
                }
            )
            .is_empty()
        );
    }

    #[test]
    fn a_teammate_watched_after_dying_makes_no_events() {
        let mut t = every_mode();
        kinds(&mut t, Post::default());
        assert_eq!(
            kinds(
                &mut t,
                Post {
                    deaths: 1,
                    ..Post::default()
                }
            ),
            [Death]
        );
        assert!(
            kinds(
                &mut t,
                Post {
                    shown: MATE,
                    kills: 9,
                    deaths: 9,
                    ..Post::default()
                }
            )
            .is_empty()
        );
        // Back to yourself with a kill made by your grenade meanwhile: taken in quietly.
        assert!(
            kinds(
                &mut t,
                Post {
                    kills: 1,
                    deaths: 1,
                    ..Post::default()
                }
            )
            .is_empty()
        );
        assert_eq!(
            kinds(
                &mut t,
                Post {
                    kills: 2,
                    deaths: 1,
                    ..Post::default()
                }
            ),
            [Kill]
        );
    }

    #[test]
    fn warmup_and_a_new_map_count_for_nothing() {
        let mut t = every_mode();
        kinds(
            &mut t,
            Post {
                map_phase: "warmup",
                ..Post::default()
            },
        );
        assert!(
            kinds(
                &mut t,
                Post {
                    map_phase: "warmup",
                    kills: 5,
                    ..Post::default()
                }
            )
            .is_empty()
        );
        let mut other_map = Post {
            kills: 2,
            ..Post::default()
        }
        .json();
        other_map["map"]["name"] = json!("de_mirage");
        let found: Vec<_> = t
            .update(&other_map, Instant::now())
            .into_iter()
            .map(|e| e.kind)
            .collect();
        // Only the end of what was open on the last map.
        assert_eq!(found, [RoundDone]);
    }

    #[test]
    fn special_kills_come_with_their_kill() {
        let mut t = every_mode();
        let knife = ("weapon_knife_karambit", "Knife", None);
        kinds(
            &mut t,
            Post {
                weapon: knife,
                ..Post::default()
            },
        );
        assert_eq!(
            kinds(
                &mut t,
                Post {
                    kills: 1,
                    weapon: knife,
                    ..Post::default()
                }
            ),
            [Kill, Knife]
        );
        let zeus = ("weapon_taser", "StackableItem", Some(1));
        kinds(
            &mut t,
            Post {
                kills: 1,
                weapon: zeus,
                ..Post::default()
            },
        );
        assert_eq!(
            kinds(
                &mut t,
                Post {
                    kills: 2,
                    weapon: (zeus.0, zeus.1, Some(0)),
                    ..Post::default()
                }
            ),
            [Kill, Zeus]
        );
        assert_eq!(
            kinds(
                &mut t,
                Post {
                    kills: 3,
                    flashed: 220,
                    ..Post::default()
                }
            ),
            [Kill, BlindKill]
        );
        assert_eq!(
            kinds(
                &mut t,
                Post {
                    kills: 4,
                    flashed: 40,
                    ..Post::default()
                }
            ),
            [Kill]
        );
    }

    #[test]
    fn a_collateral_is_two_kills_for_one_bullet() {
        let mut t = every_mode();
        let awp = |bullets| ("weapon_awp", "SniperRifle", Some(bullets));
        kinds(
            &mut t,
            Post {
                weapon: awp(5),
                ..Post::default()
            },
        );
        assert_eq!(
            kinds(
                &mut t,
                Post {
                    kills: 2,
                    weapon: awp(4),
                    ..Post::default()
                }
            ),
            [Kill, Kill, Collateral]
        );
        // Two bullets: two kills, nothing more.
        assert_eq!(
            kinds(
                &mut t,
                Post {
                    kills: 4,
                    weapon: awp(2),
                    ..Post::default()
                }
            ),
            [Kill, Kill]
        );
    }

    #[test]
    fn a_round_win_is_reported_once_whatever_the_case() {
        let mut t = every_mode();
        kinds(&mut t, Post::default());
        let won = || Post {
            round_phase: "over",
            winner: Some("ct"),
            ..Post::default()
        };
        assert_eq!(kinds(&mut t, won()), [RoundWin]);
        assert!(kinds(&mut t, won()).is_empty());
        kinds(&mut t, Post::default());
        let lost = Post {
            round_phase: "over",
            winner: Some("T"),
            ..Post::default()
        };
        assert!(kinds(&mut t, lost).is_empty());
    }

    #[test]
    fn events_carry_their_round_and_its_end_comes_last() {
        let mut t = every_mode();
        let update = |t: &mut Tracker, post: Post| -> Vec<(EventKind, Option<u32>)> {
            t.update(&post.json(), Instant::now())
                .into_iter()
                .map(|e| (e.kind, e.round))
                .collect()
        };
        update(
            &mut t,
            Post {
                round_phase: "freezetime",
                ..Post::default()
            },
        );
        let first = update(
            &mut t,
            Post {
                kills: 1,
                ..Post::default()
            },
        );
        let round = first[0].1;
        assert!(round.is_some());
        // The kill that ends the round, in the same post as its end.
        assert_eq!(
            update(
                &mut t,
                Post {
                    kills: 2,
                    round_phase: "over",
                    winner: Some("CT"),
                    ..Post::default()
                }
            ),
            [(Kill, round), (RoundWin, round), (RoundDone, round)]
        );
        // A kill after the end still belongs to that round; the next round has another number.
        assert_eq!(
            update(
                &mut t,
                Post {
                    kills: 3,
                    round_phase: "over",
                    ..Post::default()
                }
            ),
            [(Kill, round)]
        );
        update(
            &mut t,
            Post {
                kills: 3,
                round_phase: "freezetime",
                ..Post::default()
            },
        );
        let next = update(
            &mut t,
            Post {
                kills: 4,
                ..Post::default()
            },
        );
        assert_ne!(next[0].1, round);
    }

    #[test]
    fn modes_without_rounds_merge_by_time_and_modes_left_out_are_quiet() {
        let mut t = every_mode();
        let dm = |kills| Post {
            mode: "deathmatch",
            kills,
            ..Post::default()
        };
        t.update(&dm(0).json(), Instant::now());
        let found = t.update(&dm(1).json(), Instant::now());
        assert_eq!((found[0].kind, found[0].round), (Kill, None));

        let mut t = Tracker::new(vec!["competitive".into()]);
        assert!(t.clips("premier") && !t.clips("deathmatch") && !t.clips("custom"));
        kinds(&mut t, dm(0));
        assert!(kinds(&mut t, dm(1)).is_empty());
    }

    #[test]
    fn requests_are_read_one_after_another() {
        let raw = b"POST / HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: 2\r\n\r\n{}\
POST / HTTP/1.1\r\ncontent-length: 4\r\n\r\nnull";
        let mut reader = BufReader::new(&raw[..]);
        assert_eq!(read_request(&mut reader).unwrap().unwrap(), b"{}");
        assert_eq!(read_request(&mut reader).unwrap().unwrap(), b"null");
        assert!(read_request(&mut reader).unwrap().is_none());
        let big = format!(
            "POST / HTTP/1.1\r\nContent-Length: {}\r\n\r\n",
            MAX_BODY + 1
        );
        assert!(read_request(&mut BufReader::new(big.as_bytes())).is_err());
    }

    #[test]
    fn a_post_reaches_the_event_queue() {
        let (tx, rx) = std::sync::mpsc::sync_channel(8);
        let log = Arc::new(autoclip::Log::new(|_| {}));
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let modes = vec!["competitive".to_string()];
        let gsi = Cs2Gsi::start(port, "t".into(), modes, tx, log.clone()).unwrap();
        let mut client = std::net::TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
        for kills in [0, 1] {
            let body = Post {
                kills,
                ..Post::default()
            }
            .json()
            .to_string();
            write!(
                client,
                "POST / HTTP/1.1\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
            let mut reply = [0u8; 38];
            client.read_exact(&mut reply).unwrap();
            assert!(reply.starts_with(b"HTTP/1.1 200 OK"));
        }
        let event = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!((event.kind, event.source), (Kill, SOURCE));
        drop(gsi);
        let messages: Vec<_> = log.entries().into_iter().map(|e| e.message).collect();
        assert!(
            messages.contains(&"CS2 is connected".to_string()),
            "{messages:?}"
        );
        assert!(
            messages.contains(&"CS2: competitive on de_dust2".to_string()),
            "{messages:?}"
        );
    }

    #[test]
    fn the_config_names_the_port_token_and_weapons() {
        let text = config_text(47380, "abc123");
        assert!(text.contains("\"http://127.0.0.1:47380/\""));
        assert!(text.contains("\"token\" \"abc123\""));
        assert!(text.contains("\"player_weapons\"     \"1\""));
    }
}
