use super::*;
use std::cell::Cell;

fn missing() -> Result<String, std::env::VarError> {
    Err(std::env::VarError::NotPresent)
}

fn non_unicode() -> Result<String, std::env::VarError> {
    Err(std::env::VarError::NotUnicode(std::ffi::OsString::from(
        "private-byte-canary",
    )))
}

fn select(
    primary: Result<String, std::env::VarError>,
    fallback: Result<String, std::env::VarError>,
) -> Result<SelectedDiscordToken, String> {
    let mut primary = Some(primary);
    let mut fallback = Some(fallback);
    read_discord_token(|source| match source {
        DiscordTokenSource::Primary => primary.take().expect("primary read once"),
        DiscordTokenSource::Fallback => fallback.take().expect("fallback read once"),
    })
}

#[test]
fn missing_both_sources_fails_with_a_sentence() {
    assert_eq!(
        select(missing(), missing()).err().expect("must fail"),
        "Neither DISCORD_TOKEN nor DISCORD_BOT_TOKEN is set. Export one bot token; never hardcode it."
    );
}

#[test]
fn nonblank_primary_wins_without_reading_fallback() {
    let fallback_reads = Cell::new(0);
    let selected = read_discord_token(|source| match source {
        DiscordTokenSource::Primary => Ok("  primary-token  ".into()),
        DiscordTokenSource::Fallback => {
            fallback_reads.set(fallback_reads.get() + 1);
            Ok("fallback-token".into())
        }
    })
    .expect("primary selected");
    assert_eq!(selected.source(), DiscordTokenSource::Primary);
    assert_eq!(selected.secret(), "primary-token");
    assert_eq!(fallback_reads.get(), 0);
}

#[test]
fn blank_primary_fails_without_reading_fallback() {
    let fallback_reads = Cell::new(0);
    let error = read_discord_token(|source| match source {
        DiscordTokenSource::Primary => Ok("  ".into()),
        DiscordTokenSource::Fallback => {
            fallback_reads.set(fallback_reads.get() + 1);
            Ok("fallback-token".into())
        }
    })
    .err()
    .expect("blank primary must fail");
    assert_eq!(
        error,
        "DISCORD_TOKEN is present but blank; refusing to consult DISCORD_BOT_TOKEN."
    );
    assert_eq!(fallback_reads.get(), 0);
}

#[test]
fn absent_primary_selects_nonblank_fallback() {
    let selected = select(missing(), Ok(" fallback-token ".into())).expect("fallback selected");
    assert_eq!(selected.source(), DiscordTokenSource::Fallback);
    assert_eq!(selected.secret(), "fallback-token");
}

#[test]
fn blank_fallback_has_a_source_specific_error() {
    assert_eq!(
        select(missing(), Ok(" \t ".into()))
            .err()
            .expect("blank fallback must fail"),
        "DISCORD_BOT_TOKEN is present but blank."
    );
}

#[test]
fn non_unicode_primary_fails_without_reading_fallback_or_bytes() {
    let fallback_reads = Cell::new(0);
    let error = read_discord_token(|source| match source {
        DiscordTokenSource::Primary => non_unicode(),
        DiscordTokenSource::Fallback => {
            fallback_reads.set(fallback_reads.get() + 1);
            Ok("fallback-token".into())
        }
    })
    .err()
    .expect("non-Unicode primary must fail");
    assert_eq!(
        error,
        "DISCORD_TOKEN is not valid Unicode; refusing to consult DISCORD_BOT_TOKEN."
    );
    assert!(!error.contains("private-byte-canary"));
    assert_eq!(fallback_reads.get(), 0);
}

#[test]
fn non_unicode_fallback_fails_without_reproducing_bytes() {
    let error = select(missing(), non_unicode())
        .err()
        .expect("non-Unicode fallback must fail");
    assert_eq!(error, "DISCORD_BOT_TOKEN is not valid Unicode.");
    assert!(!error.contains("private-byte-canary"));
}

#[test]
fn accepted_and_rejected_diagnostics_name_only_the_selected_source() {
    for (source, selected_name, other_name) in [
        (
            DiscordTokenSource::Primary,
            "DISCORD_TOKEN",
            "DISCORD_BOT_TOKEN",
        ),
        (
            DiscordTokenSource::Fallback,
            "DISCORD_BOT_TOKEN",
            "DISCORD_TOKEN",
        ),
    ] {
        let accepted = source.accepted_diagnostic();
        let rejected = source.rejected_diagnostic();
        assert!(accepted.contains(selected_name));
        assert!(rejected.contains(selected_name));
        assert!(!accepted.contains(other_name));
        assert!(!rejected.contains(other_name));
        assert!(!accepted.contains("secret-canary"));
        assert!(!rejected.contains("secret-canary"));
    }
}

#[test]
fn auth_rejections_are_mapped_but_other_failures_are_preserved() {
    for source in [DiscordTokenSource::Primary, DiscordTokenSource::Fallback] {
        assert_eq!(
            explain_discord_http_status(source, 401),
            Some(source.rejected_diagnostic())
        );
        assert_eq!(explain_discord_http_status(source, 403), None);
        assert_eq!(explain_discord_http_status(source, 500), None);
        assert_eq!(
            explain_discord_gateway_error(
                source,
                &serenity::gateway::GatewayError::InvalidAuthentication,
            ),
            Some(source.rejected_diagnostic())
        );
        assert_eq!(
            explain_discord_gateway_error(
                source,
                &serenity::gateway::GatewayError::InvalidGatewayIntents,
            ),
            None
        );
        let original = serenity::gateway::GatewayError::InvalidGatewayIntents;
        let expected = original.to_string();
        let mapped = map_discord_startup_error(serenity::Error::Gateway(original), source);
        assert_eq!(mapped.to_string(), expected);
    }
}
