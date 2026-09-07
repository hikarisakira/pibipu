//! pibipu: posts new Threads posts and `YouTube` uploads/streams into Discord channels.
//!
//! One task owns all state and alternates between a Threads round and a `YouTube` round on their own
//! intervals; the serenity gateway runs alongside only so the bot shows as online.

mod config;
#[cfg(test)]
mod delivery_tests;
mod discord;
mod state;
mod threads;
mod tracker;
mod youtube;

use config::Config;
use discord::Discord;
use serenity::gateway::ActivityData;
use serenity::model::gateway::GatewayIntents;
use serenity::model::guild::ScheduledEventStatus;
use serenity::model::id::GuildId;
use state::{Delivery, EventAction, EventCreation, EventUpdate, State};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tracker::Notice;

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[tokio::main]
async fn main() -> Result<()> {
    let config = Config::load(Path::new(&env("CONFIG_PATH", "config.json")))?;
    let state_path = PathBuf::from(env("STATE_PATH", "/data/state.json"));
    let token = std::env::var("DISCORD_TOKEN")?;
    let google_key = std::env::var("GOOGLE_API_KEY")?;

    let client = serenity::Client::builder(&token, GatewayIntents::empty())
        .activity(ActivityData::watching("阿苗"))
        .await?;
    let bot = Bot {
        discord: Discord {
            http: Arc::clone(&client.http),
            guild: GuildId::new(config.guild_id),
        },
        http: browser_client()?,
        config,
        google_key,
        state_path,
    };
    let mut client = client;
    tokio::select! {
        result = bot.run() => result,
        result = client.start() => {
            result?;
            Ok(())
        }
    }
}

fn env(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_owned())
}

/// Threads serves its timeline JSON only to requests that look like a browser.
fn browser_client() -> Result<reqwest::Client> {
    use reqwest::header::{ACCEPT, ACCEPT_LANGUAGE, HeaderMap, HeaderValue, USER_AGENT};
    let mut h = HeaderMap::new();
    h.insert(USER_AGENT, HeaderValue::from_static("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/128.0.0.0 Safari/537.36"));
    h.insert(
        ACCEPT,
        HeaderValue::from_static("text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8"),
    );
    h.insert(ACCEPT_LANGUAGE, HeaderValue::from_static("en-US,en;q=0.9"));
    h.insert("Sec-Fetch-Dest", HeaderValue::from_static("document"));
    h.insert("Sec-Fetch-Mode", HeaderValue::from_static("navigate"));
    h.insert("Sec-Fetch-Site", HeaderValue::from_static("none"));
    Ok(reqwest::Client::builder()
        .default_headers(h)
        .timeout(Duration::from_secs(30))
        .build()?)
}

struct Bot {
    config: Config,
    discord: Discord,
    http: reqwest::Client,
    google_key: String,
    state_path: PathBuf,
}

impl Bot {
    async fn run(self) -> Result<()> {
        let mut state = State::load(&self.state_path)?.unwrap_or_default();
        // Fail before polling or sending if the volume cannot store our progress.
        state.save(&self.state_path)?;
        let mut threads_tick =
            tokio::time::interval(Duration::from_secs(self.config.threads_interval_secs));
        let mut youtube_tick =
            tokio::time::interval(Duration::from_secs(self.config.youtube_interval_secs));
        threads_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        youtube_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        self.flush(&mut state).await?;
        loop {
            tokio::select! {
                _ = threads_tick.tick() => self.threads_round(&mut state).await,
                _ = youtube_tick.tick() => self.youtube_round(&mut state).await,
            }
            state.save(&self.state_path)?;
            self.flush(&mut state).await?;
        }
    }

    async fn threads_round(&self, state: &mut State) {
        for user in self.config.threads_users() {
            let posts = match threads::fetch(&self.http, user).await {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("threads @{user}: {e}");
                    continue;
                }
            };
            let key = format!("threads:{user}");
            // A failed first fetch must not turn the next successful snapshot into a backlog.
            let announce = state.seen.contains_key(&key);
            state.seen.entry(key.clone()).or_default();
            for post in posts {
                if !state.mark_seen(&key, &post.code) || !announce {
                    continue;
                }
                for t in self.config.targets_for_threads(user) {
                    state.pending.push_back(Delivery {
                        channel: t.channel_id,
                        content: format!("{}{}", t.prefix("threads"), post.url),
                        event: None,
                    });
                }
            }
        }
    }

    async fn youtube_round(&self, state: &mut State) {
        let now = now();
        // Tracked streams are re-checked every round even after they leave the playlist's top 10.
        let mut candidates: Vec<String> = state.lives.keys().cloned().collect();
        let mut baselines = HashMap::new();
        for channel in self.config.youtube_channels() {
            let key = tracker::seen_key(channel);
            match youtube::latest_ids(&self.http, &self.google_key, channel).await {
                Ok(ids) => {
                    if !state.seen.contains_key(&key) {
                        baselines.insert(key.clone(), ids.clone());
                    }
                    candidates.extend(ids.into_iter().rev().filter(|id| !state.is_seen(&key, id)));
                }
                Err(e) => eprintln!("youtube {channel}: {e}"),
            }
        }
        let mut unique = HashSet::new();
        candidates.retain(|id| unique.insert(id.clone()));
        // Capture readiness before observe() records any IDs during this round.
        let initialized: HashSet<_> = state.seen.keys().cloned().collect();
        let mut fetched = HashSet::new();
        for chunk in candidates.chunks(50) {
            let videos = match youtube::videos(&self.http, &self.google_key, chunk).await {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("youtube videos: {e}");
                    continue;
                }
            };
            fetched.extend(chunk.iter().cloned());
            for video in &videos {
                if initialized.contains(&tracker::seen_key(&video.channel)) {
                    for notice in tracker::observe(state, video, now) {
                        self.announce(state, notice, now);
                    }
                } else {
                    tracker::baseline(state, video, now);
                }
            }
        }
        for (key, ids) in baselines {
            if ids.iter().all(|id| fetched.contains(id)) {
                state.seen.entry(key.clone()).or_default();
                for id in ids.iter().rev() {
                    state.mark_seen(&key, id);
                }
            } else {
                // Retry the entire first snapshot if the videos request failed partway through.
                state.seen.remove(&key);
            }
        }
        state.prune_lives(now);
    }

    fn announce(&self, state: &mut State, notice: Notice<'_>, now: i64) {
        let mut event = None;
        let (kind, video, text) = match notice {
            Notice::Uploaded(v) => ("video", v, v.url()),
            Notice::Scheduled { video, at } => {
                event = Some(EventCreation {
                    video: video.id.clone(),
                    title: video.title.clone(),
                    at,
                });
                (
                    "scheduled",
                    video,
                    format!("📅 直播預定 <t:{at}:F>（<t:{at}:R>）\n{}", video.url()),
                )
            }
            Notice::Rescheduled { at, event_id, .. } => {
                if let Some(event) = event_id.filter(|_| at > now) {
                    state.event_updates.push_back(EventUpdate {
                        event,
                        action: EventAction::Reschedule(at),
                    });
                }
                return;
            }
            Notice::Started { video, event_id } => {
                if let Some(event) = event_id {
                    state.event_updates.push_back(EventUpdate {
                        event,
                        action: EventAction::Start,
                    });
                }
                ("live", video, video.url())
            }
            Notice::Ended {
                video,
                started,
                ended,
                event_id,
            } => {
                if let Some(event) = event_id {
                    state.event_updates.push_back(EventUpdate {
                        event,
                        action: EventAction::Complete,
                    });
                }
                let text = format!(
                    "📺 直播結束 ｜ 🕑 {} ｜ 👀 {} ｜ ❤️ {}\n{}",
                    hms(ended - started),
                    count(video.views),
                    count(video.likes),
                    video.url()
                );
                ("end", video, text)
            }
        };
        for t in self.config.targets_for_youtube(&video.channel) {
            state.pending.push_back(Delivery {
                channel: t.channel_id,
                content: format!("{}{text}", t.prefix(kind)),
                event: event.clone(),
            });
        }
    }

    async fn delivery_content(&self, state: &mut State, delivery: &Delivery) -> Result<String> {
        let mut content = delivery.content.clone();
        if let Some(event) = &delivery.event
            && let Some(live) = state.lives.get_mut(&event.video)
        {
            let at = live.scheduled.unwrap_or(event.at);
            content = content.replace(&format!("<t:{}:", event.at), &format!("<t:{at}:"));
            if live.event_id.is_none() && live.started.is_none() && at > now() {
                let url = format!("https://www.youtube.com/watch?v={}", event.video);
                match self.discord.create_event(&event.title, at, &url).await {
                    Ok(id) => live.event_id = Some(id),
                    Err(e) => {
                        eprintln!("create event for {}: {e}", event.video);
                        if discord::retryable(e.as_ref()) {
                            return Err(e);
                        }
                        // Missing event permissions must not prevent sending the notice.
                    }
                }
            }
            if let Some(id) = live.event_id {
                content.push('\n');
                content.push_str(&self.discord.event_link(id));
            }
        }
        // Save a newly created event ID before sending, so failed deliveries reuse it.
        state.save(&self.state_path)?;
        Ok(content)
    }

    async fn flush(&self, state: &mut State) -> Result<()> {
        let mut blocked_events = HashSet::new();
        let mut index = 0;
        while let Some(update) = state.event_updates.get(index).cloned() {
            if blocked_events.contains(&update.event) {
                index += 1;
                continue;
            }
            let result = match update.action {
                EventAction::Reschedule(at) if at > now() => {
                    self.discord.reschedule_event(update.event, at).await
                }
                EventAction::Reschedule(_) => Ok(()),
                EventAction::Start => {
                    self.discord
                        .set_event_status(update.event, ScheduledEventStatus::Active)
                        .await
                }
                EventAction::Complete => {
                    self.discord
                        .set_event_status(update.event, ScheduledEventStatus::Completed)
                        .await
                }
            };
            if let Err(e) = result {
                eprintln!("update event {}: {e}", update.event);
                if discord::retryable(e.as_ref()) {
                    blocked_events.insert(update.event);
                    index += 1;
                    continue;
                }
            }
            state.event_updates.remove(index);
            state.save(&self.state_path)?;
        }
        let mut blocked_channels = HashSet::new();
        let mut index = 0;
        while let Some(delivery) = state.pending.get(index).cloned() {
            if blocked_channels.contains(&delivery.channel) {
                index += 1;
                continue;
            }
            let content = match self.delivery_content(state, &delivery).await {
                Ok(content) => content,
                Err(e) if discord::retryable(e.as_ref()) => {
                    blocked_channels.insert(delivery.channel);
                    index += 1;
                    continue;
                }
                Err(e) => return Err(e),
            };
            if let Err(e) = self.discord.send(delivery.channel, &content).await {
                eprintln!("send to {}: {e}", delivery.channel);
                if discord::retryable_message(e.as_ref()) {
                    blocked_channels.insert(delivery.channel);
                    index += 1;
                    continue;
                }
                eprintln!("discarding invalid delivery to {}", delivery.channel);
            }
            state.pending.remove(index);
            state.save(&self.state_path)?;
        }
        Ok(())
    }
}

fn now() -> i64 {
    i64::try_from(
        std::time::SystemTime::UNIX_EPOCH
            .elapsed()
            .map_or(0, |d| d.as_secs()),
    )
    .unwrap_or(0)
}

fn hms(secs: i64) -> String {
    let s = secs.max(0);
    format!("{}:{:02}:{:02}", s / 3600, s % 3600 / 60, s % 60)
}

fn count(n: Option<u64>) -> String {
    n.map_or_else(|| "?".to_owned(), |n| n.to_string())
}

#[cfg(test)]
mod tests {
    #[test]
    fn duration_and_count_formatting() {
        assert_eq!(super::hms(9_242), "2:34:02");
        assert_eq!(super::hms(-5), "0:00:00");
        assert_eq!(super::count(Some(1_499)), "1499");
        assert_eq!(super::count(None), "?");
    }
}
