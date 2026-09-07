//! The `YouTube` state machine: given what the API says about a video right now, decide what to announce.
//! Pure over [`State`]; no network, no Discord. Every rule about "new", "already told", "rescheduled",
//! "started", "ended" lives here.

use crate::state::{Live, State};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Video {
    pub id: String,
    pub channel: String,
    pub title: String,
    /// Present for streams and premieres, absent for plain uploads.
    pub live: Option<LiveDetails>,
    pub views: Option<u64>,
    pub likes: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LiveDetails {
    pub scheduled: Option<i64>,
    pub started: Option<i64>,
    pub ended: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Notice<'a> {
    Uploaded(&'a Video),
    /// A Discord event should be created; store its id with [`State::lives`].
    Scheduled {
        video: &'a Video,
        at: i64,
    },
    Rescheduled {
        video: &'a Video,
        at: i64,
        event_id: Option<u64>,
    },
    Started {
        video: &'a Video,
        event_id: Option<u64>,
    },
    Ended {
        video: &'a Video,
        started: i64,
        ended: i64,
        event_id: Option<u64>,
    },
}

impl Video {
    pub fn url(&self) -> String {
        format!("https://www.youtube.com/watch?v={}", self.id)
    }
}

pub fn seen_key(channel: &str) -> String {
    format!("youtube:{channel}")
}

/// Advances the tracking state for one video and returns what to announce, in order.
pub fn observe<'a>(state: &mut State, v: &'a Video, now: i64) -> Vec<Notice<'a>> {
    if let Some(live) = state.lives.get_mut(&v.id) {
        let Some(d) = &v.live else { return Vec::new() };
        let mut out = Vec::new();
        if live.started.is_none()
            && d.started.is_none()
            && d.ended.is_none()
            && let Some(at) = d.scheduled
            && live.scheduled != Some(at)
        {
            out.push(if live.scheduled.is_none() {
                Notice::Scheduled { video: v, at }
            } else {
                Notice::Rescheduled {
                    video: v,
                    at,
                    event_id: live.event_id,
                }
            });
            live.scheduled = Some(at);
        }
        if let (Some(_), None) = (d.started, live.started) {
            live.started = d.started;
            out.push(Notice::Started {
                video: v,
                event_id: live.event_id,
            });
        }
        let ended = d
            .ended
            .map(|ended| (live.started.unwrap_or(ended), ended, live.event_id));
        if let Some((started, ended, event_id)) = ended {
            state.lives.remove(&v.id);
            out.push(Notice::Ended {
                video: v,
                started,
                ended,
                event_id,
            });
        }
        return out;
    }

    if !state.mark_seen(&seen_key(&v.channel), &v.id) {
        return Vec::new();
    }
    let track = |state: &mut State, d: &LiveDetails| {
        state.lives.insert(
            v.id.clone(),
            Live {
                first_seen: now,
                scheduled: d.scheduled,
                started: d.started,
                event_id: None,
            },
        );
    };
    match &v.live {
        None => vec![Notice::Uploaded(v)],
        // A stream can start and finish between polls or while the bot is offline.
        Some(LiveDetails {
            started,
            ended: Some(ended),
            ..
        }) => vec![Notice::Ended {
            video: v,
            started: started.unwrap_or(*ended),
            ended: *ended,
            event_id: None,
        }],
        Some(d) if d.started.is_some() => {
            track(state, d);
            vec![Notice::Started {
                video: v,
                event_id: None,
            }]
        }
        Some(d) => {
            track(state, d);
            d.scheduled
                .map_or_else(Vec::new, |at| vec![Notice::Scheduled { video: v, at }])
        }
    }
}

/// Record the first successful snapshot, including ongoing streams, without notifying.
pub fn baseline(state: &mut State, video: &Video, now: i64) {
    let _ = observe(state, video, now);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn video(id: &str, live: Option<LiveDetails>) -> Video {
        Video {
            id: id.into(),
            channel: "UC1".into(),
            title: id.into(),
            live,
            views: None,
            likes: None,
        }
    }

    #[test]
    fn upload_is_announced_once() {
        let mut s = State::default();
        let v = video("a", None);
        assert_eq!(observe(&mut s, &v, 0), [Notice::Uploaded(&v)]);
        assert!(observe(&mut s, &v, 0).is_empty());
        assert!(s.lives.is_empty());
    }

    #[test]
    fn stream_goes_scheduled_rescheduled_started_ended() {
        let mut s = State::default();
        let v = video(
            "s",
            Some(LiveDetails {
                scheduled: Some(100),
                ..Default::default()
            }),
        );
        assert_eq!(
            observe(&mut s, &v, 0),
            [Notice::Scheduled { video: &v, at: 100 }]
        );
        assert!(
            observe(&mut s, &v, 1).is_empty(),
            "same schedule, nothing new"
        );
        s.lives.get_mut("s").unwrap().event_id = Some(7);

        let v = video(
            "s",
            Some(LiveDetails {
                scheduled: Some(200),
                ..Default::default()
            }),
        );
        assert_eq!(
            observe(&mut s, &v, 2),
            [Notice::Rescheduled {
                video: &v,
                at: 200,
                event_id: Some(7)
            }]
        );

        let v = video(
            "s",
            Some(LiveDetails {
                scheduled: Some(200),
                started: Some(205),
                ended: None,
            }),
        );
        assert_eq!(
            observe(&mut s, &v, 3),
            [Notice::Started {
                video: &v,
                event_id: Some(7)
            }]
        );
        assert!(observe(&mut s, &v, 4).is_empty());

        let v = video(
            "s",
            Some(LiveDetails {
                scheduled: Some(150),
                started: Some(205),
                ended: Some(900),
            }),
        );
        assert_eq!(
            observe(&mut s, &v, 5),
            [Notice::Ended {
                video: &v,
                started: 205,
                ended: 900,
                event_id: Some(7)
            }],
            "a schedule change after start is not a reschedule"
        );
        assert!(s.lives.is_empty(), "finished streams stop being tracked");
        assert!(
            observe(&mut s, &v, 6).is_empty(),
            "and are not announced again"
        );
    }

    #[test]
    fn stream_first_seen_on_air_or_finished() {
        let mut s = State::default();
        let on_air = video(
            "l",
            Some(LiveDetails {
                scheduled: Some(1),
                started: Some(2),
                ended: None,
            }),
        );
        assert_eq!(
            observe(&mut s, &on_air, 0),
            [Notice::Started {
                video: &on_air,
                event_id: None
            }]
        );

        let over = video(
            "o",
            Some(LiveDetails {
                scheduled: Some(1),
                started: Some(2),
                ended: Some(3),
            }),
        );
        assert_eq!(
            observe(&mut s, &over, 4),
            [Notice::Ended {
                video: &over,
                started: 2,
                ended: 3,
                event_id: None
            }]
        );
        assert!(observe(&mut s, &over, 5).is_empty());
        assert!(!s.lives.contains_key("o"));
        assert!(
            !s.mark_seen(&seen_key("UC1"), "o"),
            "still remembered so it is never re-examined"
        );
    }

    #[test]
    fn first_snapshot_tracks_streams_without_replaying_their_existing_status() {
        let mut s = State::default();
        let mut scheduled = video(
            "scheduled",
            Some(LiveDetails {
                scheduled: Some(100),
                ..Default::default()
            }),
        );
        let mut live = video(
            "live",
            Some(LiveDetails {
                started: Some(1),
                ..Default::default()
            }),
        );
        let ended = video(
            "ended",
            Some(LiveDetails {
                started: Some(1),
                ended: Some(2),
                ..Default::default()
            }),
        );
        for v in [&scheduled, &live, &video("upload", None), &ended] {
            baseline(&mut s, v, 10);
            assert!(observe(&mut s, v, 11).is_empty());
        }
        assert_eq!(s.lives.len(), 2);
        assert!(s.lives.values().all(|l| l.event_id.is_none()));
        scheduled.live.as_mut().unwrap().started = Some(102);
        assert!(matches!(
            observe(&mut s, &scheduled, 103).as_slice(),
            [Notice::Started { .. }]
        ));
        live.live.as_mut().unwrap().ended = Some(104);
        assert!(matches!(
            observe(&mut s, &live, 105).as_slice(),
            [Notice::Ended { .. }]
        ));
    }

    #[test]
    fn incomplete_stream_metadata_is_not_an_upload_and_start_does_not_reschedule() {
        let mut s = State::default();
        let mut v = video("v", Some(LiveDetails::default()));
        assert!(observe(&mut s, &v, 0).is_empty());
        v.live.as_mut().unwrap().scheduled = Some(100);
        assert!(matches!(
            observe(&mut s, &v, 1).as_slice(),
            [Notice::Scheduled { at: 100, .. }]
        ));
        v.live = Some(LiveDetails {
            scheduled: Some(105),
            started: Some(110),
            ended: Some(120),
        });
        assert!(matches!(
            observe(&mut s, &v, 121).as_slice(),
            [Notice::Started { .. }, Notice::Ended { .. }]
        ));
    }
}
