//! `YouTube` Data API v3: a channel's latest uploads and whether each one is a stream.
//! Two list calls, one quota unit each.

use crate::tracker::{LiveDetails, Video};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serenity::model::Timestamp;

const API: &str = "https://www.googleapis.com/youtube/v3";

/// Newest-first ids of the channel's uploads playlist, which also lists scheduled and running streams.
/// The playlist id is the channel id with `UC` swapped for `UU`.
pub async fn latest_ids(
    client: &reqwest::Client,
    key: &str,
    channel: &str,
) -> crate::Result<Vec<String>> {
    #[derive(Deserialize)]
    struct Resp {
        items: Vec<Item>,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Item {
        content_details: Details,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Details {
        video_id: String,
    }
    let uploads = format!(
        "UU{}",
        channel
            .strip_prefix("UC")
            .ok_or("YouTube channel id must start with UC")?
    );
    let query = [
        ("part", "contentDetails"),
        ("maxResults", "10"),
        ("playlistId", &uploads),
        ("key", key),
    ];
    let r: Resp = get(client, "playlistItems", &query).await?;
    Ok(r.items
        .into_iter()
        .map(|i| i.content_details.video_id)
        .collect())
}

/// Details for up to 50 ids. Deleted or private videos are simply missing from the result.
pub async fn videos(
    client: &reqwest::Client,
    key: &str,
    ids: &[String],
) -> crate::Result<Vec<Video>> {
    #[derive(Deserialize)]
    struct Resp {
        items: Vec<Item>,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Item {
        id: String,
        snippet: Snippet,
        statistics: Option<Stats>,
        live_streaming_details: Option<Live>,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Snippet {
        title: String,
        channel_id: String,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Stats {
        view_count: Option<String>,
        like_count: Option<String>,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    #[allow(clippy::struct_field_names)] // mirrors the API's field names
    struct Live {
        scheduled_start_time: Option<String>,
        actual_start_time: Option<String>,
        actual_end_time: Option<String>,
    }
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let query = [
        ("part", "snippet,statistics,liveStreamingDetails"),
        ("id", &ids.join(",")),
        ("key", key),
    ];
    let r: Resp = get(client, "videos", &query).await?;
    r.items
        .into_iter()
        .map(|i| {
            let live = match i.live_streaming_details {
                Some(l) => Some(LiveDetails {
                    scheduled: unix(l.scheduled_start_time)?,
                    started: unix(l.actual_start_time)?,
                    ended: unix(l.actual_end_time)?,
                }),
                None => None,
            };
            let count = |s: Option<&String>| s.and_then(|s| s.parse().ok());
            Ok(Video {
                id: i.id,
                channel: i.snippet.channel_id,
                title: i.snippet.title,
                live,
                views: count(i.statistics.as_ref().and_then(|s| s.view_count.as_ref())),
                likes: count(i.statistics.as_ref().and_then(|s| s.like_count.as_ref())),
            })
        })
        .collect()
}

/// Non-2xx responses carry Google's JSON error (quota, bad key); keep it in the message so logs say why.
async fn get<T: DeserializeOwned>(
    client: &reqwest::Client,
    path: &str,
    query: &[(&str, &str)],
) -> crate::Result<T> {
    // reqwest includes the full request URL in errors; its query contains the API key.
    let resp = client
        .get(format!("{API}/{path}"))
        .query(query)
        .send()
        .await
        .map_err(reqwest::Error::without_url)?;
    if !resp.status().is_success() {
        let status = resp.status();
        let mut body = resp.text().await.map_err(reqwest::Error::without_url)?;
        for (_, key) in query
            .iter()
            .filter(|(name, key)| *name == "key" && !key.is_empty())
        {
            body = body.replace(key, "[redacted]");
        }
        return Err(format!("youtube {path}: {status} {body}").into());
    }
    Ok(resp.json().await.map_err(reqwest::Error::without_url)?)
}

fn unix(rfc3339: Option<String>) -> crate::Result<Option<i64>> {
    rfc3339
        .map(|s| Ok(Timestamp::parse(&s)?.unix_timestamp()))
        .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn transport_errors_do_not_expose_the_api_key() {
        // A local proxy that never replies forces a timeout without contacting Google.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let client = reqwest::Client::builder()
            .proxy(
                reqwest::Proxy::all(format!("http://{}", listener.local_addr().unwrap())).unwrap(),
            )
            .timeout(std::time::Duration::from_millis(50))
            .build()
            .unwrap();
        let error = get::<serde_json::Value>(&client, "videos", &[("key", "secret-api-key")])
            .await
            .unwrap_err();
        assert!(error.downcast_ref::<reqwest::Error>().unwrap().is_timeout());
        assert!(!format!("{error:?}").contains("secret-api-key"));
    }

    /// `GOOGLE_API_KEY=... cargo test -- --ignored`: does the real API still have this shape?
    #[tokio::test]
    #[ignore = "hits the YouTube API, needs GOOGLE_API_KEY"]
    async fn live_shapes() {
        let key = std::env::var("GOOGLE_API_KEY").unwrap();
        let client = reqwest::Client::new();
        let ids = latest_ids(&client, &key, "UCG3hBfZxzLixJPZQ9QUP_Lw")
            .await
            .unwrap();
        assert_eq!(ids.len(), 10);
        let vids = videos(&client, &key, &ids).await.unwrap();
        assert_eq!(vids.len(), 10);
        assert!(
            vids.iter()
                .all(|v| v.channel == "UCG3hBfZxzLixJPZQ9QUP_Lw" && !v.title.is_empty())
        );
        assert!(
            vids.iter().any(|v| v.live.is_some()),
            "a streamer's last 10 uploads include a stream"
        );
        assert!(vids.iter().any(|v| v.views.is_some()));
    }

    #[test]
    fn youtube_timestamps_parse_to_unix() {
        assert_eq!(
            unix(Some("2026-09-05T09:57:22Z".into())).unwrap(),
            Some(1_788_602_242)
        );
        assert_eq!(unix(None).unwrap(), None);
        assert!(unix(Some("yesterday".into())).is_err());
    }
}
