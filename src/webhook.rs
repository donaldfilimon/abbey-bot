//! Incoming-webhook setup guide.
//!
//! The skill's "Set up a webhook" row: payload plus curl for an incoming
//! webhook. Emit-only, like `/server` — creating the webhook is one click in a
//! settings screen the user is already looking at, and a URL minted by the bot
//! would be a credential the bot then knows.
//!
//! Two facts carry most of this module's value, because both are the kind
//! people learn the hard way:
//!
//! - **The webhook URL is the credential.** No token accompanies it; anyone
//!   holding the URL can post as the webhook forever. Treat it like a password,
//!   and rotate by deleting the webhook if it ever lands in a repo or a log.
//! - **Webhooks bypass member permissions.** A webhook posts regardless of who
//!   is denied Send Messages, and its content can ping `@everyone` unless the
//!   payload says otherwise — so the example payload ships with
//!   `"allowed_mentions": {"parse": []}`, which makes every mention inert
//!   unless deliberately enabled.

/// Where the webhook will post, as the guide should describe it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// An ordinary channel, labelled for display ("#general", "🔊 Lobby").
    Channel { label: String },
    /// A thread. Webhooks cannot be created on threads — they attach to the
    /// parent channel and post into the thread via `?thread_id=`.
    Thread { label: String, parent_label: String },
}

/// The example payload, as one canonical string.
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

/// Render the setup guide.
pub fn guide(target: &Target) -> String {
    let (where_to, thread_note) = match target {
        Target::Channel { label } => (label.clone(), String::new()),
        Target::Thread {
            label,
            parent_label,
        } => (
            parent_label.clone(),
            format!(
                "\n\nYou asked about {label}, which is a thread. Webhooks cannot be created on \
                 threads — create it on {parent_label} and append `?thread_id=<the thread's id>` \
                 to the URL when posting; the message lands in the thread."
            ),
        ),
    };

    format!(
        "**Incoming webhook for {where_to}**\n\n\
         1. Server Settings → Integrations → Webhooks → **New Webhook**.\n\
         2. Point it at {where_to}, name it, and **Copy Webhook URL**.\n\
         3. Treat that URL as a credential: anyone holding it can post as the webhook, \
         no token required. If it leaks, delete the webhook — that is the rotation.\n\
         4. Save the payload below as `payload.json`.\n\
         5. Post it:\n\
         ```sh\n\
         curl -X POST \"$WEBHOOK_URL\" -H 'Content-Type: application/json' -d @payload.json\n\
         ```\n\
         ```json\n{payload}\n```\n\
         The `allowed_mentions` block makes every mention in the content inert — webhooks \
         bypass member permissions entirely, so a payload without it can ping `@everyone`. \
         Delete the block only when a ping is the point.{thread_note}",
        payload = EXAMPLE_PAYLOAD,
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

    #[test]
    fn the_example_payload_is_valid_json_with_inert_mentions() {
        // Parsed, not string-matched: the guide tells people to save this to a
        // file verbatim, so drifting into invalid JSON is the worst failure.
        let parsed: serde_json::Value =
            serde_json::from_str(EXAMPLE_PAYLOAD).expect("example payload must parse");
        assert_eq!(
            parsed["allowed_mentions"]["parse"],
            serde_json::json!([]),
            "the safe-by-default mentions block is the point of the example"
        );
        assert!(parsed["content"].is_string());
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
    fn no_realistic_webhook_url_appears_anywhere() {
        // The guide must never contain something that looks like a live URL —
        // only the $WEBHOOK_URL placeholder. A pasted-in example URL is exactly
        // the leak the guide warns about.
        let out = guide(&channel());
        assert!(!out.contains("discord.com/api/webhooks/"), "{out}");
    }

    #[test]
    fn a_thread_target_redirects_to_the_parent_with_thread_id() {
        let out = guide(&Target::Thread {
            label: "\"release-chatter\"".to_string(),
            parent_label: "#releases".to_string(),
        });
        assert!(out.contains("?thread_id="), "{out}");
        assert!(
            out.contains("create it on #releases"),
            "must point at the parent: {out}"
        );
        // The numbered steps target the parent, not the thread.
        assert!(out.contains("Point it at #releases"), "{out}");
    }

    #[test]
    fn a_plain_channel_gets_no_thread_digression() {
        assert!(!guide(&channel()).contains("thread_id"));
    }
}
