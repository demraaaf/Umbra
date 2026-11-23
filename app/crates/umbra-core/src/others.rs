//! "Other apps": every program that plays sound except the game, the apps that have their
//! own tracks, and Umbra. Windows can leave only one program out of a loopback capture, so
//! this track is not one capture: each other program is captured on its own and the
//! captures are summed. An app with its own track is therefore never inside "Other apps",
//! and taking its track out of a clip takes it out completely.
//!
//! About once a second the open audio sessions are listed and captures are started for new
//! programs and stopped for ones that went away or became the game. A program that plays a
//! sound for the very first time can lose up to about a second of it here. The Windows system
//! sounds session belongs to no program and is not captured.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::align::{CHUNK, SAMPLE_RATE};
use crate::audio::{AudioSource, TrackWorker};
use crate::clock::Clock;
use crate::process::{ProcessEntry, list_processes};

/// How far behind the clock the summed track is emitted, in chunks (about 210 ms): late
/// enough that every capture has delivered its part of a chunk.
const DELAY_CHUNKS: u64 = 10;
/// How often the list of programs is refreshed.
const POLL: Duration = Duration::from_secs(1);

/// Which processes to capture, each as a whole tree, so that together they hold every
/// sound except the excluded ones exactly once.
///
/// `sessions` are the processes with an audio session. A process is left out when it is in
/// the tree of Umbra (`me`) or of the game, or when it or a parent is one of the `except`
/// programs. A process that started one of those (Steam starting the game, Explorer starting
/// most things) cannot be captured as a tree without them and is left out too. Of what is
/// left, a process is dropped when a parent of it is already captured.
pub fn choose(
    sessions: &[u32],
    processes: &[ProcessEntry],
    me: u32,
    game: u32,
    except: &[String],
) -> Vec<u32> {
    let parent: HashMap<u32, u32> = processes.iter().map(|p| (p.pid, p.parent_pid)).collect();
    let exe: HashMap<u32, &str> = processes.iter().map(|p| (p.pid, p.exe.as_str())).collect();
    // A process and its parents, nearest first. Bounded, since process ids are reused and
    // a stale parent id can form a loop.
    let lineage = |pid: u32| -> Vec<u32> {
        let mut chain = vec![pid];
        let mut current = pid;
        while let Some(&up) = parent.get(&current) {
            if up == 0 || up == current || chain.contains(&up) || chain.len() > 64 {
                break;
            }
            chain.push(up);
            current = up;
        }
        chain
    };
    let is_except = |pid: u32| {
        exe.get(&pid)
            .is_some_and(|e| except.iter().any(|x| x.eq_ignore_ascii_case(e)))
    };
    let excluded = |pid: u32| {
        lineage(pid)
            .iter()
            .any(|&p| p == me || (game != 0 && p == game) || is_except(p))
    };
    // Processes whose trees must not be captured whole: parents of anything excluded.
    let mut shielding: HashSet<u32> = HashSet::new();
    for p in processes {
        let root_of_excluded = p.pid == me || (game != 0 && p.pid == game) || is_except(p.pid);
        if root_of_excluded {
            shielding.extend(lineage(p.pid).into_iter().skip(1));
        }
    }

    let candidates: Vec<u32> = sessions
        .iter()
        .copied()
        .filter(|&pid| parent.contains_key(&pid))
        .filter(|&pid| !excluded(pid) && !shielding.contains(&pid))
        .collect();
    let set: HashSet<u32> = candidates.iter().copied().collect();
    let mut chosen: Vec<u32> = candidates
        .into_iter()
        .filter(|&pid| !lineage(pid).iter().skip(1).any(|p| set.contains(p)))
        .collect();
    chosen.sort_unstable();
    chosen.dedup();
    chosen
}

/// Sums the captures of the other programs and hands out one chunk at a time.
pub(crate) fn run(
    game: Arc<AtomicU32>,
    except: Vec<String>,
    clock: Clock,
    stop: Arc<AtomicBool>,
    on_chunk: &mut (dyn FnMut(u64, &[f32]) + Send),
) {
    struct Sums {
        /// Chunks before this were handed out; late parts of them are dropped.
        next: u64,
        chunks: BTreeMap<u64, Vec<f32>>,
    }
    let sums = Arc::new(Mutex::new(Sums {
        next: 0,
        chunks: BTreeMap::new(),
    }));
    let mut workers: HashMap<u32, TrackWorker> = HashMap::new();
    let mut next_poll = Instant::now();
    let zeros = vec![0.0f32; CHUNK * 2];
    let me = std::process::id();

    while !stop.load(Ordering::Relaxed) {
        if Instant::now() >= next_poll {
            next_poll = Instant::now() + POLL;
            match crate::devices::audio_session_pids() {
                Ok(sessions) => {
                    let wanted: HashSet<u32> = choose(
                        &sessions,
                        &list_processes(),
                        me,
                        game.load(Ordering::Relaxed),
                        &except,
                    )
                    .into_iter()
                    .collect();
                    workers.retain(|pid, _| wanted.contains(pid));
                    for pid in wanted {
                        if workers.contains_key(&pid) {
                            continue;
                        }
                        let sums = sums.clone();
                        let worker = TrackWorker::spawn(
                            AudioSource::ProcessTree { pid },
                            clock,
                            Box::new(move |index, samples| {
                                let mut s = sums.lock().unwrap();
                                if index < s.next {
                                    return;
                                }
                                let sum = s
                                    .chunks
                                    .entry(index)
                                    .or_insert_with(|| vec![0.0; CHUNK * 2]);
                                for (total, sample) in sum.iter_mut().zip(samples) {
                                    *total += sample;
                                }
                            }),
                        );
                        match worker {
                            Ok(worker) => {
                                workers.insert(pid, worker);
                            }
                            Err(err) => log::warn!("cannot capture process {pid}: {err:#}"),
                        }
                    }
                }
                Err(err) => log::warn!("cannot list audio sessions: {err:#}"),
            }
        }

        // Hand out every chunk the captures have had time to fill.
        let now = (clock.now_secs().max(0.0) * SAMPLE_RATE as f64 / CHUNK as f64) as u64;
        let ready = now.saturating_sub(DELAY_CHUNKS);
        let batch: Vec<(u64, Option<Vec<f32>>)> = {
            let mut s = sums.lock().unwrap();
            let mut out = Vec::new();
            while s.next <= ready {
                let index = s.next;
                out.push((index, s.chunks.remove(&index)));
                s.next += 1;
            }
            out
        };
        for (index, chunk) in batch {
            on_chunk(index, chunk.as_deref().unwrap_or(&zeros));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(pid: u32, parent_pid: u32, exe: &str) -> ProcessEntry {
        ProcessEntry {
            pid,
            parent_pid,
            exe: exe.into(),
        }
    }

    #[test]
    fn leaves_out_umbra_the_game_and_apps_with_their_own_tracks() {
        let processes = vec![
            p(1, 0, "explorer.exe"),
            p(10, 1, "Spotify.exe"),
            p(11, 10, "Spotify.exe"),
            p(20, 1, "Discord.exe"),
            p(21, 20, "Discord.exe"),
            p(30, 1, "steam.exe"),
            p(31, 30, "game.exe"),
            p(40, 1, "chrome.exe"),
            p(41, 40, "chrome.exe"),
            p(50, 1, "umbra.exe"),
            p(51, 50, "msedgewebview2.exe"),
            p(60, 1, "vlc.exe"),
        ];
        // Sessions: Spotify's child, Discord's child, Steam itself, the game, a Chrome
        // child, Umbra's webview, VLC and Explorer.
        let sessions = [11, 21, 30, 31, 41, 51, 60, 1];
        let chosen = choose(
            &sessions,
            &processes,
            50,
            31,
            &["Spotify.exe".into(), "Discord.exe".into()],
        );
        // Steam and Explorer started excluded programs, so their trees can't be captured whole.
        assert_eq!(chosen, vec![41, 60]);
    }

    #[test]
    fn a_child_is_not_captured_twice() {
        let processes = vec![
            p(1, 0, "explorer.exe"),
            p(40, 1, "app.exe"),
            p(41, 40, "helper.exe"),
        ];
        assert_eq!(choose(&[40, 41], &processes, 99, 0, &[]), vec![40]);
    }

    #[test]
    fn without_a_game_only_umbra_and_the_listed_apps_are_left_out() {
        let processes = vec![p(5, 0, "wininit.exe"), p(6, 5, "a.exe"), p(7, 5, "b.exe")];
        assert_eq!(
            choose(&[6, 7], &processes, 99, 0, &["b.exe".into()]),
            vec![6]
        );
    }
}
