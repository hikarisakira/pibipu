//! Everything that talks to Discord: plain-text messages and guild scheduled events.

use serenity::builder::{CreateScheduledEvent, EditScheduledEvent};
use serenity::http::Http;
use serenity::model::Timestamp;
use serenity::model::guild::{ScheduledEventStatus, ScheduledEventType};
use serenity::model::id::{ChannelId, GuildId, ScheduledEventId};
use std::sync::Arc;

/// External events need an end time and `YouTube` has none. Discord auto-completes the event at
/// this point, so a stream longer than this loses its event card early.
// ponytail: fixed 3h; make it a config knob if streams routinely run longer.
const EVENT_SECS: i64 = 3 * 3600;

pub struct Discord {
    pub http: Arc<Http>,
    pub guild: GuildId,
}

impl Discord {
    pub async fn send(&self, channel: u64, content: &str) -> crate::Result<()> {
        ChannelId::new(channel).say(&self.http, content).await?;
        Ok(())
    }

    /// Creates an external event pointing at `url`; returns its id.
    pub async fn create_event(&self, name: &str, start: i64, url: &str) -> crate::Result<u64> {
        let name: String = name.chars().take(100).collect();
        let builder = CreateScheduledEvent::new(ScheduledEventType::External, name, ts(start)?)
            .end_time(ts(start + EVENT_SECS)?)
            .location(url);
        Ok(self
            .guild
            .create_scheduled_event(&self.http, builder)
            .await?
            .id
            .get())
    }

    pub async fn reschedule_event(&self, event: u64, start: i64) -> crate::Result<()> {
        let current = self
            .guild
            .scheduled_event(&self.http, ScheduledEventId::new(event), false)
            .await?;
        // Active events cannot return to Scheduled, and Discord rejects start times in the past.
        if current.status != ScheduledEventStatus::Scheduled
            || current.start_time.unix_timestamp() == start
            || start <= Timestamp::now().unix_timestamp()
        {
            return Ok(());
        }
        let builder = EditScheduledEvent::new()
            .start_time(ts(start)?)
            .end_time(ts(start + EVENT_SECS)?);
        self.guild
            .edit_scheduled_event(&self.http, ScheduledEventId::new(event), builder)
            .await?;
        Ok(())
    }

    /// Reconcile with Discord's automatic transitions so replaying a persisted update is safe.
    pub async fn set_event_status(
        &self,
        event: u64,
        status: ScheduledEventStatus,
    ) -> crate::Result<()> {
        let event = ScheduledEventId::new(event);
        let mut current = self
            .guild
            .scheduled_event(&self.http, event, false)
            .await?
            .status;
        while let Some(next) = next_status(current, status) {
            current = self
                .guild
                .edit_scheduled_event(&self.http, event, EditScheduledEvent::new().status(next))
                .await?
                .status;
        }
        Ok(())
    }

    /// Pasting this into a channel renders the event card with its "Interested" button.
    pub fn event_link(&self, event: u64) -> String {
        format!("https://discord.com/events/{}/{event}", self.guild.get())
    }
}

fn next_status(
    current: ScheduledEventStatus,
    desired: ScheduledEventStatus,
) -> Option<ScheduledEventStatus> {
    use ScheduledEventStatus::{Active, Canceled, Completed, Scheduled};
    if current == desired || matches!(current, Completed | Canceled) {
        None
    } else if current == Scheduled && desired == Completed {
        Some(Active)
    } else {
        Some(desired)
    }
}

/// Only transient Discord failures should keep an event update in the retry queue.
pub fn retryable(error: &(dyn std::error::Error + Send + Sync + 'static)) -> bool {
    use serenity::http::HttpError;
    match error.downcast_ref::<serenity::Error>() {
        Some(serenity::Error::Http(HttpError::UnsuccessfulRequest(response))) => {
            retryable_status(response.status_code)
        }
        Some(serenity::Error::Http(HttpError::Request(error))) => !error.is_builder(),
        Some(serenity::Error::Http(HttpError::RateLimitI64F64 | HttpError::RateLimitUtf8)) => true,
        _ => false,
    }
}

fn retryable_status(status: reqwest::StatusCode) -> bool {
    status.is_server_error() || status == reqwest::StatusCode::TOO_MANY_REQUESTS
}

/// Permissions/credentials can be repaired; invalid payloads and deleted channels cannot.
pub fn retryable_message(error: &(dyn std::error::Error + Send + Sync + 'static)) -> bool {
    if let Some(serenity::Error::Http(serenity::http::HttpError::UnsuccessfulRequest(response))) =
        error.downcast_ref::<serenity::Error>()
        && matches!(response.status_code.as_u16(), 401 | 403)
    {
        return true;
    }
    retryable(error)
}

fn ts(unix: i64) -> crate::Result<Timestamp> {
    Ok(Timestamp::from_unix_timestamp(unix)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_updates_follow_discord_transitions_and_ignore_replays() {
        use ScheduledEventStatus::{Active, Canceled, Completed, Scheduled};
        assert_eq!(next_status(Scheduled, Active), Some(Active));
        assert_eq!(next_status(Scheduled, Completed), Some(Active));
        assert_eq!(next_status(Active, Completed), Some(Completed));
        assert_eq!(next_status(Active, Active), None);
        assert_eq!(next_status(Completed, Completed), None);
        assert_eq!(next_status(Completed, Active), None);
        assert_eq!(next_status(Canceled, Active), None);
    }

    #[test]
    fn retry_transient_failures_only() {
        for code in [429, 500, 502, 503, 504] {
            assert!(retryable_status(
                reqwest::StatusCode::from_u16(code).unwrap()
            ));
        }
        for code in [400, 401, 403, 404] {
            assert!(!retryable_status(
                reqwest::StatusCode::from_u16(code).unwrap()
            ));
        }
        assert!(!retryable(&serenity::Error::Other("invalid event")));
        assert!(retryable(&serenity::Error::Http(
            serenity::http::HttpError::RateLimitUtf8
        )));
    }
}
