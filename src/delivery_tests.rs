//! Offline checks exercise the real serenity HTTP path against a loopback server.
use crate::{
    Bot, Config, Discord, State,
    state::{Delivery, EventCreation, Live},
};
use serenity::{
    http::HttpBuilder,
    model::{channel::Message, id::GuildId},
};
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    path::PathBuf,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

fn server(replies: Vec<(u16, String)>) -> (String, thread::JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let task = thread::spawn(move || {
        let mut requests = Vec::new();
        for (status, body) in replies {
            let deadline = Instant::now() + Duration::from_secs(5);
            let (stream, _) = loop {
                match listener.accept() {
                    Ok(connection) => break connection,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(
                            Instant::now() < deadline,
                            "expected another local HTTP request"
                        );
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(e) => panic!("{e}"),
                }
            };
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut stream = BufReader::new(stream);
            let mut request = String::new();
            let mut length = 0;
            loop {
                let mut line = String::new();
                assert!(stream.read_line(&mut line).unwrap() > 0);
                if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = value.trim().parse().unwrap();
                }
                request.push_str(&line);
                if line == "\r\n" {
                    break;
                }
            }
            let mut bytes = vec![0; length];
            stream.read_exact(&mut bytes).unwrap();
            request.push_str(std::str::from_utf8(&bytes).unwrap());
            requests.push(request);
            write!(stream.get_mut(), "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        }
        requests
    });
    (url, task)
}

fn bot(url: &str, name: &str) -> Bot {
    let http = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap();
    Bot {
        config: serde_json::from_str::<Config>(
            r#"{"guild_id":1,"threads_interval_secs":300,"youtube_interval_secs":60,"targets":[]}"#,
        )
        .unwrap(),
        discord: Discord {
            http: Arc::new(
                HttpBuilder::new("test-only")
                    .proxy(url)
                    .client(http.clone())
                    .ratelimiter_disabled(true)
                    .build(),
            ),
            guild: GuildId::new(1),
        },
        http,
        google_key: "unused".into(),
        state_path: std::env::temp_dir().join(format!("pibipu-{name}-{}.json", std::process::id())),
    }
}

fn delivery(channel: u64, content: &str) -> Delivery {
    Delivery {
        channel,
        content: content.into(),
        event: None,
    }
}

fn message() -> (u16, String) {
    (200, serde_json::to_string(&Message::default()).unwrap())
}

fn error(code: u16) -> (u16, String) {
    (code, r#"{"code":50013,"message":"test failure"}"#.into())
}

#[tokio::test]
async fn failed_deliveries_survive_restart_without_replaying_success_or_reordering() {
    let (url, task) = server(vec![message(), error(403), message(), message()]);
    let bot = bot(&url, "retry");
    let mut state = State::default();
    state.pending.extend([
        delivery(10, "accepted"),
        delivery(20, "first"),
        delivery(20, "second"),
        delivery(30, &"x".repeat(2001)), // Invalid payload must not poison the queue.
    ]);
    state.save(&bot.state_path).unwrap();
    bot.flush(&mut state).await.unwrap();
    let mut restored = State::load(&bot.state_path).unwrap().unwrap();
    assert_eq!(
        restored.pending,
        [delivery(20, "first"), delivery(20, "second")]
    );
    bot.flush(&mut restored).await.unwrap();
    assert!(
        State::load(&bot.state_path)
            .unwrap()
            .unwrap()
            .pending
            .is_empty()
    );
    let requests = task.join().unwrap();
    let content: Vec<_> = requests
        .iter()
        .map(|r| {
            serde_json::from_str::<serde_json::Value>(r.split_once("\r\n\r\n").unwrap().1).unwrap()
                ["content"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect();
    assert_eq!(content, ["accepted", "first", "first", "second"]);
    std::fs::remove_file(&bot.state_path).unwrap();
}

#[tokio::test]
async fn event_creation_retries_and_destinations_share_the_persisted_event() {
    let at = crate::now() + 3600;
    let event = serde_json::json!({
        "id":"7", "guild_id":"1", "name":"stream", "scheduled_start_time":"2030-01-01T00:00:00Z",
        "privacy_level":2, "status":1, "entity_type":3
    });
    let (url, task) = server(vec![
        error(503),
        (200, event.to_string()),
        message(),
        message(),
    ]);
    let bot = bot(&url, "event-retry");
    let mut state = State::default();
    state.lives.insert(
        "video".into(),
        Live {
            first_seen: crate::now(),
            scheduled: Some(at),
            started: None,
            event_id: None,
        },
    );
    for channel in [10, 20] {
        let mut notice = delivery(channel, "scheduled");
        notice.event = Some(EventCreation {
            video: "video".into(),
            title: "標".repeat(110),
            at,
        });
        state.pending.push_back(notice);
    }
    state.save(&bot.state_path).unwrap();
    bot.flush(&mut state).await.unwrap();
    let mut restored = State::load(&bot.state_path).unwrap().unwrap();
    assert_eq!(restored.pending.len(), 1);
    assert_eq!(restored.lives["video"].event_id, Some(7));
    bot.flush(&mut restored).await.unwrap();
    assert!(restored.pending.is_empty());
    let requests = task.join().unwrap();
    assert!(requests[0].starts_with("POST /api/v10/guilds/1/scheduled-events "));
    assert!(requests[1].starts_with("POST /api/v10/guilds/1/scheduled-events "));
    for request in &requests[2..] {
        assert!(request.contains("https://discord.com/events/1/7"));
    }
    let payload: serde_json::Value =
        serde_json::from_str(requests[1].split_once("\r\n\r\n").unwrap().1).unwrap();
    assert_eq!(payload["name"].as_str().unwrap().chars().count(), 100);
    assert_eq!(
        payload["entity_metadata"]["location"],
        "https://www.youtube.com/watch?v=video"
    );
    std::fs::remove_file(&bot.state_path).unwrap();
}

#[tokio::test]
async fn missing_event_permission_still_sends_the_notice() {
    let (url, task) = server(vec![error(403), message()]);
    let bot = bot(&url, "event-permission");
    let mut state = State::default();
    let at = crate::now() + 3600;
    state.lives.insert(
        "video".into(),
        Live {
            first_seen: crate::now(),
            scheduled: Some(at),
            started: None,
            event_id: None,
        },
    );
    let mut notice = delivery(10, "scheduled");
    notice.event = Some(EventCreation {
        video: "video".into(),
        title: "stream".into(),
        at,
    });
    state.pending.push_back(notice);
    state.save(&bot.state_path).unwrap();
    bot.flush(&mut state).await.unwrap();
    assert!(state.pending.is_empty());
    assert_eq!(state.lives["video"].event_id, None);
    assert_eq!(task.join().unwrap().len(), 2);
    std::fs::remove_file(&bot.state_path).unwrap();
}

#[tokio::test]
async fn broken_state_stops_the_worker_before_any_network_request() {
    let mut bot = bot("http://127.0.0.1:9", "broken-state");
    std::fs::write(&bot.state_path, "{").unwrap();
    let path: PathBuf = bot.state_path.clone();
    assert!(bot.run().await.is_err());
    std::fs::remove_file(&path).unwrap();
    bot = self::bot("http://127.0.0.1:9", "unwritable-state");
    std::fs::write(&bot.state_path, "parent is a file").unwrap();
    let path = bot.state_path.clone();
    bot.state_path.push("state.json");
    assert!(bot.run().await.is_err());
    std::fs::remove_file(path).unwrap();
}
