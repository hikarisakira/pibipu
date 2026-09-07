//! Threads posts, read straight off the public profile pages the same way `RSSHub` does:
//! Meta embeds the timeline as JSON in `<script data-sjs>` blocks; posts live in arrays named `thread_items`.

use serde_json::Value;

pub const THREADS: &str = "https://www.threads.com";
/// Embed fixer: swapping the host makes Discord render the post.
pub const FIXTHREADS: &str = "https://fixthreads.seria.moe";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Post {
    pub code: String,
    pub taken_at: i64,
    pub url: String,
}

/// The link we post to Discord.
pub fn post_url(user: &str, code: &str) -> String {
    format!("{FIXTHREADS}/@{user}/post/{code}")
}

/// Own posts, quotes and replies by `user` from the profile page and the replies page, oldest first.
pub async fn fetch(client: &reqwest::Client, user: &str) -> crate::Result<Vec<Post>> {
    let mut posts: Vec<Post> = Vec::new();
    for page in [
        format!("{THREADS}/@{user}"),
        format!("{THREADS}/@{user}/replies"),
    ] {
        let html = client
            .get(&page)
            .send()
            .await?
            .error_for_status()?
            .text()
            .await?;
        for p in extract(&html, user).map_err(|e| format!("{page}: {e}"))? {
            if !posts.iter().any(|q| q.code == p.code) {
                posts.push(p);
            }
        }
    }
    posts.sort_by_key(|p| p.taken_at);
    Ok(posts)
}

/// Pure part of [`fetch`]: posts authored by `user` in one page. Reposts of other people's posts are dropped
/// (they are not the account's own content); replies and quotes are kept. Pages also carry other
/// people's posts (the conversation around a reply), hence the author filter.
pub fn extract(html: &str, user: &str) -> crate::Result<Vec<Post>> {
    let mut out = Vec::new();
    let mut found_timeline = false;
    for json in
        script_blocks(html).filter(|s| s.contains("thread_items") || s.contains("\"mediaData\""))
    {
        let v = serde_json::from_str::<Value>(json)?;
        found_timeline |= walk(&v, user, &mut out);
    }
    if !found_timeline {
        return Err("no timeline data in page (login wall or page format changed?)".into());
    }
    Ok(out)
}

/// Bodies of `<script ... data-sjs ...>` tags. Meta escapes `</` inside the JSON, so the closing tag is unambiguous.
fn script_blocks(html: &str) -> impl Iterator<Item = &str> {
    html.split("<script").skip(1).filter_map(|chunk| {
        let (attrs, body) = chunk.split_once('>')?;
        if !attrs.contains("data-sjs") {
            return None;
        }
        Some(body.split_once("</script>").map_or(body, |(b, _)| b))
    })
}

fn walk(v: &Value, user: &str, out: &mut Vec<Post>) -> bool {
    let mut found_timeline = false;
    match v {
        Value::Object(map) => {
            // An account with no posts/replies has an empty connection, with no thread_items.
            found_timeline = map
                .get("mediaData")
                .and_then(|m| m.get("edges"))
                .and_then(Value::as_array)
                .is_some_and(Vec::is_empty);
            if let Some(Value::Array(items)) = map.get("thread_items") {
                found_timeline = true;
                for item in items {
                    if let Some(p) = post(item.get("post"), user)
                        && !out.iter().any(|q| q.code == p.code)
                    {
                        out.push(p);
                    }
                }
            }
            for child in map.values() {
                found_timeline |= walk(child, user, out);
            }
        }
        Value::Array(items) => {
            for child in items {
                found_timeline |= walk(child, user, out);
            }
        }
        _ => {}
    }
    found_timeline
}

fn post(p: Option<&Value>, user: &str) -> Option<Post> {
    let p = p?;
    let author = p.pointer("/user/username")?.as_str()?;
    if !author.eq_ignore_ascii_case(user) {
        return None;
    }
    if p.pointer("/text_post_app_info/share_info/reposted_post")
        .is_some_and(|r| !r.is_null())
    {
        return None;
    }
    let code = p.get("code")?.as_str()?;
    let url = p
        .get("canonical_url")
        .and_then(Value::as_str)
        .and_then(|url| url.strip_prefix(&format!("{THREADS}/")))
        .map_or_else(
            || post_url(author, code),
            |path| format!("{FIXTHREADS}/{path}"),
        );
    Some(Post {
        code: code.to_owned(),
        taken_at: p.get("taken_at")?.as_i64()?,
        url,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAIN: &str = include_str!("../tests/fixtures/threads_main.html");
    const REPLIES: &str = include_str!("../tests/fixtures/threads_replies.html");

    /// `cargo test threads::tests::live_fetch -- --ignored --exact`: check the public timeline.
    #[tokio::test]
    #[ignore = "hits threads.com"]
    async fn live_fetch() {
        let posts = fetch(&crate::browser_client().unwrap(), "miaopoyatw")
            .await
            .unwrap();
        assert!(posts.len() >= 4, "got {posts:?}");
        assert!(
            posts.windows(2).all(|w| w[0].taken_at <= w[1].taken_at),
            "oldest first"
        );
    }

    #[test]
    fn profile_page_yields_own_posts_including_quotes() {
        let codes: Vec<_> = extract(MAIN, "miaopoyatw")
            .unwrap()
            .into_iter()
            .map(|p| p.code)
            .collect();
        assert_eq!(
            codes,
            ["Dc34dNqmC33", "Dc8lC9GEgIp", "Dc8MTT-FJyf", "Dc6JmiHmIII"]
        );
        assert!(extract(MAIN, "someone_else").unwrap().is_empty());
        assert_eq!(
            extract(MAIN, "MiaopoyaTW").unwrap(),
            extract(MAIN, "miaopoyatw").unwrap()
        );
    }

    #[test]
    fn replies_page_keeps_only_the_accounts_own_replies() {
        let posts = extract(REPLIES, "miaopoyatw").unwrap();
        assert_eq!(posts.len(), 4, "8 items on the page, 4 are by other people");
        assert!(posts.iter().all(|p| p.taken_at > 1_788_000_000));
    }

    #[test]
    fn reposts_are_dropped_and_url_uses_fixthreads() {
        let html = r#"<script data-sjs>{"x":{"thread_items":[
            {"post":{"code":"own","taken_at":1,"user":{"username":"u"},"text_post_app_info":{"share_info":{"reposted_post":null}}}},
            {"post":{"code":"rt","taken_at":2,"user":{"username":"u"},"text_post_app_info":{"share_info":{"reposted_post":{"code":"theirs"}}}}}
        ]}}</script>"#;
        let posts = extract(html, "u").unwrap();
        assert_eq!(
            posts,
            [Post {
                code: "own".into(),
                taken_at: 1,
                url: post_url("u", "own")
            }]
        );
        assert_eq!(
            post_url("u", "own"),
            "https://fixthreads.seria.moe/@u/post/own"
        );
    }

    #[test]
    fn canonical_url_is_preserved_and_duplicates_are_dropped() {
        let html = r#"<script data-sjs>{"thread_items":[
            {"post":{"code":"own","taken_at":1,"user":{"username":"u"},"canonical_url":"https://www.threads.com/@u/post/canonical?x=1"}},
            {"post":{"code":"own","taken_at":1,"user":{"username":"u"}}}
        ]}</script>"#;
        let posts = extract(html, "u").unwrap();
        assert_eq!(posts.len(), 1);
        assert_eq!(
            posts[0].url,
            "https://fixthreads.seria.moe/@u/post/canonical?x=1"
        );
    }

    #[test]
    fn invalid_timeline_is_an_error_but_empty_timeline_is_valid() {
        for html in [
            "<html>Log in to see thread_items</html>",
            r#"<script data-sjs>{"thread_items":</script>"#,
            r#"<script data-sjs>{"thread_items":null}</script>"#,
            r#"<script data-sjs>{"text":"thread_items"}</script>"#,
        ] {
            assert!(extract(html, "u").is_err(), "{html}");
        }
        for html in [
            r#"<script data-sjs>{"thread_items":[]}</script>"#,
            r#"<script data-sjs>{"data":{"mediaData":{"edges":[]}}}</script>"#,
        ] {
            assert!(extract(html, "u").unwrap().is_empty());
        }
    }
}
