//! What has already been announced, persisted to `state.json` on the Fly volume.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::io::Write;
use std::path::Path;

/// IDs remembered per source. Both pages we poll return far fewer than this.
const KEEP: usize = 200;
/// A live we never saw finish is forgotten after this long.
const LIVE_TTL: i64 = 7 * 24 * 3600;

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct State {
    /// `"threads:<user>"` / `"youtube:<channel>"` -> newest-last IDs already handled.
    #[serde(default)]
    pub seen: HashMap<String, VecDeque<String>>,
    /// `YouTube` video id -> live stream still being tracked (scheduled or on air).
    #[serde(default)]
    pub lives: HashMap<String, Live>,
    /// Deliveries remain here until Discord accepts them, including across restarts.
    #[serde(default)]
    pub pending: VecDeque<Delivery>,
    #[serde(default)]
    pub event_updates: VecDeque<EventUpdate>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Delivery {
    pub channel: u64,
    pub content: String,
    /// Only scheduled notices may create an event; startup baselines never enqueue these.
    #[serde(default)]
    pub event: Option<EventCreation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventCreation {
    pub video: String,
    pub title: String,
    pub at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventUpdate {
    pub event: u64,
    pub action: EventAction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EventAction {
    Reschedule(i64),
    Start,
    Complete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Live {
    pub first_seen: i64,
    pub scheduled: Option<i64>,
    pub started: Option<i64>,
    /// Discord scheduled event created for it, if creation succeeded.
    pub event_id: Option<u64>,
}

impl State {
    /// `Ok(None)` when the file does not exist yet: first boot, nothing is known.
    pub fn load(path: &Path) -> crate::Result<Option<State>> {
        match std::fs::read(path) {
            Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Write-then-rename so a crash mid-write cannot leave a truncated file.
    pub fn save(&self, path: &Path) -> crate::Result<()> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("json.tmp");
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(&serde_json::to_vec(self)?)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    pub fn is_seen(&self, key: &str, id: &str) -> bool {
        self.seen
            .get(key)
            .is_some_and(|ids| ids.iter().any(|s| s == id))
    }

    /// Records `id` under `key`; `true` if it was not known before.
    pub fn mark_seen(&mut self, key: &str, id: &str) -> bool {
        let ids = self.seen.entry(key.to_owned()).or_default();
        if ids.iter().any(|seen| seen == id) {
            return false;
        }
        ids.push_back(id.to_owned());
        if ids.len() > KEEP {
            ids.pop_front();
        }
        true
    }

    // ponytail: no "cancel event when the stream is deleted"; a live that never ends is just dropped after a week.
    pub fn prune_lives(&mut self, now: i64) {
        self.lives.retain(|_, l| {
            let since = l
                .first_seen
                .max(l.scheduled.unwrap_or(0))
                .max(l.started.unwrap_or(0));
            now.saturating_sub(since) < LIVE_TTL
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seen_is_bounded_and_roundtrips_through_disk() {
        let mut s = State::default();
        assert!(s.mark_seen("k", "a"));
        assert!(!s.mark_seen("k", "a"));
        for i in 0..KEEP {
            s.mark_seen("k", &i.to_string());
        }
        assert_eq!(s.seen["k"].len(), KEEP);
        assert!(!s.seen["k"].contains(&"a".to_owned()), "oldest id evicted");
        s.lives.insert(
            "v".into(),
            Live {
                first_seen: 0,
                scheduled: Some(5),
                started: None,
                event_id: Some(9),
            },
        );

        let path = std::env::temp_dir().join(format!("pibipu-state-{}.json", std::process::id()));
        s.save(&path).unwrap();
        s.pending.push_back(Delivery {
            channel: 1,
            content: "retry me".into(),
            event: None,
        });
        s.save(&path).unwrap(); // Replacing an existing state file must work on Windows too.
        let back = State::load(&path).unwrap().unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(back.seen["k"], s.seen["k"]);
        assert_eq!(back.lives["v"], s.lives["v"]);
        assert_eq!(back.pending, s.pending);
        assert!(
            State::load(&path).unwrap().is_none(),
            "missing file means first boot"
        );
    }

    #[test]
    fn stale_lives_are_pruned() {
        let mut s = State::default();
        s.lives.insert(
            "old".into(),
            Live {
                first_seen: 0,
                scheduled: None,
                started: Some(1),
                event_id: None,
            },
        );
        s.lives.insert(
            "new".into(),
            Live {
                first_seen: LIVE_TTL,
                scheduled: None,
                started: Some(1),
                event_id: None,
            },
        );
        s.lives.insert(
            "future".into(),
            Live {
                first_seen: 0,
                scheduled: Some(2 * LIVE_TTL),
                started: None,
                event_id: None,
            },
        );
        s.prune_lives(LIVE_TTL + 1);
        assert_eq!(s.lives.len(), 2);
        assert!(s.lives.contains_key("new"));
        assert!(
            s.lives.contains_key("future"),
            "do not expire a stream before its scheduled date"
        );
    }
}
