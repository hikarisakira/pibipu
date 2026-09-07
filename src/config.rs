//! `config.json`: which Discord channels get which Threads/YouTube sources.

use serde::Deserialize;
use std::collections::{BTreeSet, HashMap};
use std::path::Path;

#[derive(Debug, Deserialize)]
pub struct Config {
    #[serde(deserialize_with = "id")]
    pub guild_id: u64,
    pub threads_interval_secs: u64,
    pub youtube_interval_secs: u64,
    pub targets: Vec<Target>,
}

/// One Discord channel and the sources it subscribes to.
#[derive(Debug, Deserialize)]
pub struct Target {
    #[serde(deserialize_with = "id")]
    pub channel_id: u64,
    #[serde(default)]
    pub threads: Vec<String>,
    #[serde(default)]
    pub youtube: Vec<String>,
    #[serde(default)]
    prefix: Option<Prefix>,
}

/// Text put in front of every notice: one string for all kinds, or per kind
/// (`threads`, `video`, `scheduled`, `live`, `end`); missing kinds get nothing.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Prefix {
    All(String),
    ByKind(HashMap<String, String>),
}

impl Config {
    pub fn load(path: &Path) -> crate::Result<Config> {
        let mut config: Self = serde_json::from_slice(&std::fs::read(path)?)?;
        config.validate()?;
        Ok(config)
    }

    fn validate(&mut self) -> crate::Result<()> {
        if self.guild_id == 0 || self.targets.iter().any(|t| t.channel_id == 0) {
            return Err("guild_id and channel_id must be nonzero Discord IDs".into());
        }
        if self.threads_interval_secs == 0 || self.youtube_interval_secs == 0 {
            return Err("poll intervals must be greater than zero".into());
        }
        let mut channels = BTreeSet::new();
        for target in &mut self.targets {
            if !channels.insert(target.channel_id) {
                return Err("combine sources for the same channel_id into one target".into());
            }
            for user in &mut target.threads {
                *user = user.trim().trim_start_matches('@').to_ascii_lowercase();
                if user.is_empty()
                    || !user
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'.')
                {
                    return Err("Threads sources must be usernames, not URLs".into());
                }
            }
            for channel in &target.youtube {
                if channel.len() != 24
                    || !channel.starts_with("UC")
                    || !channel
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
                {
                    return Err("YouTube sources must be 24-character UC channel IDs".into());
                }
            }
        }
        Ok(())
    }

    pub fn threads_users(&self) -> BTreeSet<&str> {
        self.targets
            .iter()
            .flat_map(|t| t.threads.iter().map(String::as_str))
            .collect()
    }

    pub fn youtube_channels(&self) -> BTreeSet<&str> {
        self.targets
            .iter()
            .flat_map(|t| t.youtube.iter().map(String::as_str))
            .collect()
    }

    pub fn targets_for_threads<'a>(&'a self, user: &'a str) -> impl Iterator<Item = &'a Target> {
        self.targets
            .iter()
            .filter(move |t| t.threads.iter().any(|u| u == user))
    }

    pub fn targets_for_youtube<'a>(&'a self, channel: &'a str) -> impl Iterator<Item = &'a Target> {
        self.targets
            .iter()
            .filter(move |t| t.youtube.iter().any(|c| c == channel))
    }
}

impl Target {
    /// Prefix for a notice kind, already followed by a space when non-empty.
    pub fn prefix(&self, kind: &str) -> String {
        let p = match &self.prefix {
            Some(Prefix::All(s)) => s.as_str(),
            Some(Prefix::ByKind(m)) => m.get(kind).map_or("", String::as_str),
            None => "",
        };
        if p.is_empty() {
            String::new()
        } else {
            format!("{p} ")
        }
    }
}

/// Discord snowflakes arrive as strings from people who copy them out of the app; accept numbers too.
fn id<'de, D: serde::Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Raw {
        N(u64),
        S(String),
    }
    match Raw::deserialize(d)? {
        Raw::N(n) => Ok(n),
        Raw::S(s) => s.parse().map_err(serde::de::Error::custom),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sample_config() {
        let mut c: Config = serde_json::from_str(include_str!("../config.json")).unwrap();
        c.validate().unwrap();
        assert!(c.guild_id > 0);
        assert!(!c.targets.is_empty());
    }

    #[test]
    fn prefix_string_or_map_and_ids_as_string_or_number() {
        let c: Config = serde_json::from_str(
            r#"{"guild_id":"1","threads_interval_secs":300,"youtube_interval_secs":60,"targets":[
              {"channel_id":"10","threads":["a"],"prefix":"<@&1> hey"},
              {"channel_id":11,"youtube":["UC1"],"prefix":{"live":"go","end":""}},
              {"channel_id":12}]}"#,
        )
        .unwrap();
        assert_eq!(c.targets[0].prefix("threads"), "<@&1> hey ");
        assert_eq!(c.targets[1].prefix("live"), "go ");
        assert_eq!(c.targets[1].prefix("end"), "");
        assert_eq!(c.targets[1].prefix("video"), "");
        assert_eq!(c.targets[2].prefix("video"), "");
        assert_eq!(c.targets[1].channel_id, 11);
        assert_eq!(c.threads_users().into_iter().collect::<Vec<_>>(), ["a"]);
        assert_eq!(c.targets_for_youtube("UC1").count(), 1);
        assert_eq!(c.targets_for_youtube("UC9").count(), 0);
    }

    #[test]
    fn invalid_config_is_rejected_before_starting_the_bot() {
        let mut valid = serde_json::json!({
            "guild_id":"1", "threads_interval_secs":300, "youtube_interval_secs":60,
            "targets":[{"channel_id":"10", "threads":[" @MiaopoyaTW "], "youtube":["UCICZqWqYDD4zfwQ9_7Kw-2g"]}]
        });
        let mut c: Config = serde_json::from_value(valid.clone()).unwrap();
        c.validate().unwrap();
        assert_eq!(c.targets[0].threads, ["miaopoyatw"]);
        for (pointer, value) in [
            ("/guild_id", "0".into()),
            ("/threads_interval_secs", 0.into()),
            ("/youtube_interval_secs", 0.into()),
            ("/targets/0/channel_id", "0".into()),
            ("/targets/0/threads/0", "https://threads.com/@u".into()),
            ("/targets/0/youtube/0", "@u".into()),
        ] {
            let mut bad = valid.clone();
            *bad.pointer_mut(pointer).unwrap() = value;
            let mut c: Config = serde_json::from_value(bad).unwrap();
            assert!(c.validate().is_err(), "{pointer}");
        }
        let duplicate = valid["targets"][0].clone();
        valid["targets"].as_array_mut().unwrap().push(duplicate);
        let mut c: Config = serde_json::from_value(valid).unwrap();
        assert!(
            c.validate().is_err(),
            "duplicate destination causes duplicate notices"
        );
    }
}
