//! Incoming-webhook setup guide.
//!
//! The skill's "Set up a webhook" row: payload plus curl for an incoming
//! webhook. Emit-only, like `/server` — creating the webhook is one click in a
//! settings screen the user is already looking at, and a URL minted by the bot
//! would be a credential the bot then knows.
//!
//! Three facts carry most of this module's value, because all three are the
//! kind people learn the hard way:
//!
//! - **The webhook URL is the credential.** No token accompanies it; anyone
//!   holding the URL can post as the webhook forever. Treat it like a password,
//!   and rotate by deleting the webhook if it ever lands in a repo or a log.
//! - **Webhooks bypass member permissions.** A webhook posts regardless of who
//!   is denied Send Messages, and its content can ping `@everyone` unless the
//!   payload says otherwise — so every example payload ships with
//!   `"allowed_mentions": {"parse": []}`, which makes mentions inert unless
//!   deliberately enabled.
//! - **Threads and forums are not plain channels.** A webhook cannot be
//!   created on a thread (it attaches to the parent and posts in via
//!   `?thread_id=`), and a forum channel has no plain message stream at all —
//!   every execute must either target an existing post (`?thread_id=`) or
//!   create one (`"thread_name"` in the payload). A plain-channel curl against
//!   either is rejected by Discord.

/// Where the webhook will post, as the guide should describe it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// An ordinary channel, labelled for display ("#general", "🔊 Lobby").
    Channel { label: String },
    /// A thread. The webhook is created on the parent; the curl carries the
    /// thread's actual id so the command works as pasted.
    Thread {
        label: String,
        parent_label: String,
        thread_id: u64,
    },
    /// A forum-style channel — forum or media (type 16), which share execute
    /// semantics. Executes must create a post (`thread_name`) or target an
    /// existing one (`?thread_id=`).
    Forum { label: String },
}

/// The example payload for channels and threads, as one canonical string.
///
/// Kept as a constant rather than built with a JSON library so the guide shows
/// exactly what a user should put in a file — and a test parses it, so it
/// cannot drift into invalid JSON.
pub const EXAMPLE_PAYLOAD: &str = r#"{
  "username": "Abbey CI",
  "content": "Deploy finished — build 128 green in 4m12s",
  "embeds": [
    {
      "title": "build 128",
      "description": "All 60 checks passed",
      "color": 5763719
    }
  ],
  "allowed_mentions": { "parse": [] }
}"#;

/// The forum variant: identical, plus `thread_name`, because a forum execute
/// without `thread_name` or `?thread_id=` is rejected outright.
pub const EXAMPLE_FORUM_PAYLOAD: &str = r#"{
  "thread_name": "Deploy report — build 128",
  "username": "Abbey CI",
  "content": "Deploy finished — build 128 green in 4m12s",
  "allowed_mentions": { "parse": [] }
}"#;

/// Render the setup guide.
pub fn guide(target: &Target) -> String {
    // Where the webhook itself gets created — for threads, that is the parent.
    let create_on = match target {
        Target::Channel { label } | Target::Forum { label } => label.clone(),
        Target::Thread { parent_label, .. } => parent_label.clone(),
    };

    // The curl must work as pasted: a thread execute carries the real id.
    let curl = match target {
        Target::Thread { thread_id, .. } => format!(
            "curl -X POST \"$WEBHOOK_URL?thread_id={thread_id}\" -H 'Content-Type: application/json' -d @payload.json"
        ),
        _ => "curl -X POST \"$WEBHOOK_URL\" -H 'Content-Type: application/json' -d @payload.json"
            .to_string(),
    };

    let payload = match target {
        Target::Forum { .. } => EXAMPLE_FORUM_PAYLOAD,
        _ => EXAMPLE_PAYLOAD,
    };

    let tail = match target {
        Target::Channel { .. } => String::new(),
        Target::Thread {
            label, thread_id, ..
        } => format!(
            "\n\nYou asked about {label}, which is a thread. Webhooks cannot be created on \
             threads — the steps above create it on the parent, and the curl already carries \
             `?thread_id={thread_id}` (that thread's actual id), so the message lands in the \
             thread as pasted."
        ),
        Target::Forum { label } => format!(
            "\n\n{label} is a forum-style channel (forum and media channels work identically \
             here), which has no plain message stream — every webhook execute must either create \
             a post (the `thread_name` field in the payload above) or target an existing one by \
             appending `?thread_id=<post id>` to the URL and dropping `thread_name`. A \
             plain-channel curl is rejected by Discord."
        ),
    };

    format!(
        "**Incoming webhook for {create_on}**\n\n\
         1. Server Settings → Integrations → Webhooks → **New Webhook** (this needs \
         **Manage Webhooks** on the channel — if you cannot see the screen, that is why).\n\
         2. Point it at {create_on}, name it, and **Copy Webhook URL**.\n\
         3. Treat that URL as a credential: anyone holding it can post as the webhook, \
         no token required. If it leaks, delete the webhook — that is the rotation.\n\
         4. Save the payload below as `payload.json`.\n\
         5. Post it:\n\
         ```sh\n{curl}\n```\n\
         ```json\n{payload}\n```\n\
         The `allowed_mentions` block makes every mention in the content inert — webhooks \
         bypass member permissions entirely, so a payload without it can ping `@everyone`. \
         Delete the block only when a ping is the point.{tail}",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn channel() -> Target {
        Target::Channel {
            label: "#ops".to_string(),
        }
    }

    fn thread() -> Target {
        Target::Thread {
            label: "\"release-chatter\"".to_string(),
            parent_label: "#releases".to_string(),
            thread_id: 1_234_567_890,
        }
    }

    #[test]
    fn both_example_payloads_are_valid_json_with_inert_mentions() {
        // Parsed, not string-matched: the guide tells people to save these to a
        // file verbatim, so drifting into invalid JSON is the worst failure.
        for (name, payload) in [
            ("channel", EXAMPLE_PAYLOAD),
            ("forum", EXAMPLE_FORUM_PAYLOAD),
        ] {
            let parsed: serde_json::Value =
                serde_json::from_str(payload).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(
                parsed["allowed_mentions"]["parse"],
                serde_json::json!([]),
                "{name}: the safe-by-default mentions block is the point"
            );
        }
    }

    #[test]
    fn the_forum_payload_creates_a_post_and_the_channel_payload_does_not() {
        let forum: serde_json::Value = serde_json::from_str(EXAMPLE_FORUM_PAYLOAD).unwrap();
        assert!(
            forum["thread_name"].is_string(),
            "forum executes must name a post"
        );
        let plain: serde_json::Value = serde_json::from_str(EXAMPLE_PAYLOAD).unwrap();
        assert!(plain.get("thread_name").is_none());
    }

    #[test]
    fn the_guide_walks_creation_and_posting_end_to_end() {
        let out = guide(&channel());
        for step in ["1.", "2.", "3.", "4.", "5."] {
            assert!(out.contains(step), "missing step {step}: {out}");
        }
        assert!(out.contains("curl -X POST \"$WEBHOOK_URL\""), "{out}");
        assert!(out.contains("Content-Type: application/json"), "{out}");
        assert!(out.contains(EXAMPLE_PAYLOAD), "{out}");
        assert!(
            out.contains("Manage Webhooks"),
            "the gating permission must be named: {out}"
        );
    }

    #[test]
    fn the_url_is_named_as_the_credential() {
        let out = guide(&channel());
        assert!(out.contains("credential"), "{out}");
        assert!(
            out.contains("delete the webhook"),
            "rotation must be stated: {out}"
        );
    }

    #[test]
    fn no_realistic_webhook_url_appears_in_any_variant() {
        for target in [
            channel(),
            thread(),
            Target::Forum {
                label: "#help".into(),
            },
        ] {
            assert!(
                !guide(&target).contains("discord.com/api/webhooks/"),
                "{target:?}"
            );
        }
    }

    #[test]
    fn a_thread_curl_works_as_pasted() {
        // The command has the thread's id in hand; a placeholder would post to
        // the parent instead and silently look like success.
        let out = guide(&thread());
        assert!(
            out.contains("curl -X POST \"$WEBHOOK_URL?thread_id=1234567890\""),
            "{out}"
        );
        assert!(!out.contains("<the thread's id>"), "no placeholder: {out}");
        assert!(
            out.contains("Point it at #releases"),
            "created on the parent: {out}"
        );
    }

    #[test]
    fn a_forum_gets_the_post_semantics_not_the_plain_curl_story() {
        let out = guide(&Target::Forum {
            label: "#help".into(),
        });
        assert!(out.contains(EXAMPLE_FORUM_PAYLOAD), "{out}");
        assert!(
            out.contains("?thread_id=<post id>"),
            "existing-post path: {out}"
        );
        assert!(
            out.contains("rejected"),
            "must say the plain curl fails: {out}"
        );
    }

    #[test]
    fn a_plain_channel_gets_no_thread_digression() {
        let out = guide(&channel());
        assert!(!out.contains("thread_id"), "{out}");
        assert!(!out.contains("thread_name"), "{out}");
    }
}
