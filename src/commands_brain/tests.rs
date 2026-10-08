use super::dashboard::{
    AdminPreparation, acknowledged_admin_preparation, dashboard_rows, page_select_row,
};
use super::*;
use crate::persist::{PersistComponentOutcome, PersistErrorCategory, PersistReport};

#[test]
fn on_off_labels() {
    assert!(OnOff::On.is_on());
    assert_eq!(OnOff::Off.label(), "off");
}

#[test]
fn admin_flush_copy_is_truthful_component_level_and_content_free() {
    let report = PersistReport::from_components(
        PersistComponentOutcome::Committed,
        PersistComponentOutcome::Failed(PersistErrorCategory::SyncDirectory),
    );
    let rendered = render_admin_flush(&report);
    assert_eq!(
        rendered,
        "Persistence is partial. Canonical state: committed. WDBX projection: failed (sync-directory)."
    );
    assert!(!rendered.contains('/'));
    assert!(!rendered.contains("injected"));

    assert_eq!(
        render_admin_flush(&PersistReport::memory_only()),
        "Persistence is memory-only. Canonical state: not configured. WDBX projection: not configured."
    );
    assert_eq!(
        render_admin_flush(&PersistReport::from_components(
            PersistComponentOutcome::Committed,
            PersistComponentOutcome::Committed,
        )),
        "Persistence is complete. Canonical state: committed. WDBX projection: committed."
    );
    assert_eq!(
        render_admin_flush(&PersistReport::from_components(
            PersistComponentOutcome::Failed(PersistErrorCategory::WriteTemporary),
            PersistComponentOutcome::SkippedCanonicalFailure,
        )),
        "Persistence is failed. Canonical state: failed (write-temporary). WDBX projection: skipped after canonical failure."
    );
}

#[test]
fn facts_are_whitespace_normalized_and_character_bounded() {
    assert_eq!(
        memory::validated_fact("  Donald\nlikes\tRust.  "),
        Ok("Donald likes Rust.".to_string())
    );
    assert_eq!(
        memory::validated_fact(" \n\t "),
        Err("The fact must contain some text.")
    );
    assert!(memory::validated_fact(&"x".repeat(memory::MAX_FACT_CHARS)).is_ok());
    assert_eq!(
        memory::validated_fact(&"🦀".repeat(memory::MAX_FACT_CHARS + 1)),
        Err("Keep one remembered fact to 300 characters or fewer.")
    );
}

#[test]
fn memory_read_adapters_are_private_and_have_the_required_contexts() {
    let reputation = reputation();
    assert!(reputation.ephemeral);
    assert!(!reputation.guild_only);

    let memory = memory_context_menu();
    assert!(memory.ephemeral);
    assert!(memory.guild_only);
    assert!(matches!(
        memory.context_menu_action,
        Some(poise::ContextMenuCommandAction::User(_))
    ));
}

#[tokio::test]
async fn dashboard_adapter_acknowledges_before_validation_permission_and_reload() {
    use std::sync::{Arc, Mutex};
    let events = Arc::new(Mutex::new(Vec::new()));
    let ack_events = Arc::clone(&events);
    let validate_events = Arc::clone(&events);
    let load_events = Arc::clone(&events);
    let session = crate::admin_dashboard::AdminSession {
        owner: 1,
        guild: 2,
        expiry: 3,
        page: crate::admin_dashboard::AdminPage::Overview,
    };
    let prepared = acknowledged_admin_preparation(
        async move {
            ack_events.lock().unwrap().push("ack");
            Ok(())
        },
        || {
            validate_events.lock().unwrap().push("validate");
            Ok((
                session,
                crate::admin_dashboard::AdminAction::SetLearning(true),
            ))
        },
        || async move {
            load_events.lock().unwrap().push("permissions+reload");
            Ok((Permissions::MANAGE_GUILD, GuildSettings::default()))
        },
    )
    .await
    .unwrap();
    assert!(matches!(prepared, AdminPreparation::Ready(..)));
    assert_eq!(
        *events.lock().unwrap(),
        ["ack", "validate", "permissions+reload"]
    );
}

#[tokio::test]
async fn revoked_permission_fails_before_any_effect_is_reduced() {
    let prepared = acknowledged_admin_preparation(
        async { Ok(()) },
        || {
            Ok((
                crate::admin_dashboard::AdminSession {
                    owner: 1,
                    guild: 2,
                    expiry: 3,
                    page: crate::admin_dashboard::AdminPage::Learning,
                },
                crate::admin_dashboard::AdminAction::SetLearning(true),
            ))
        },
        || async { Ok((Permissions::empty(), GuildSettings::default())) },
    )
    .await
    .unwrap();
    assert!(matches!(prepared, AdminPreparation::PermissionDenied));
}

#[test]
fn dashboard_uses_classic_bounded_rows_and_private_registration() {
    let command = admin_dashboard();
    assert!(command.ephemeral);
    let show = admin_show();
    assert!(show.ephemeral);
    assert!(show.guild_only);
    for page in [
        crate::admin_dashboard::AdminPage::Overview,
        crate::admin_dashboard::AdminPage::Conversation,
        crate::admin_dashboard::AdminPage::Learning,
        crate::admin_dashboard::AdminPage::Operations,
        crate::admin_dashboard::AdminPage::ConfirmReset,
    ] {
        let session = crate::admin_dashboard::AdminSession {
            owner: u64::MAX,
            guild: u64::MAX,
            expiry: u64::MAX,
            page,
        };
        let rows = dashboard_rows(&session);
        assert!(rows.len() <= 5);
        let CreateActionRow::SelectMenu(_) = &rows[0] else {
            panic!("dashboard navigation must be a classic String Select")
        };
        for row in &rows[1..] {
            let CreateActionRow::Buttons(buttons) = row else {
                panic!("dashboard actions must use classic button rows")
            };
            assert!(buttons.len() <= 5);
        }
        let show_row = page_select_row(&session);
        let CreateActionRow::SelectMenu(_) = show_row else {
            panic!("/admin show must attach a classic page select")
        };
    }
}

#[test]
fn brain_diagnostics_private_authority_pending_guard_and_maximum_copy() {
    use crate::brain::state::BotAction;
    use crate::brain::telemetry::{BrainStats, LearningAudit};
    assert!(!brain_diagnostics_authorized(Permissions::VIEW_CHANNEL));
    assert!(!brain_diagnostics_authorized(Permissions::MANAGE_GUILD));
    assert!(brain_diagnostics_authorized(
        Permissions::VIEW_CHANNEL | Permissions::MANAGE_GUILD
    ));
    let mut stats = BrainStats::default();
    let mut audit = LearningAudit::default();
    let mut view = BrainView {
        epsilon: 0.1,
        learn_steps: 0,
        buffer_len: 0,
        buffer_capacity: runtime::REPLAY_CAPACITY,
        experiences: 0,
        budget_per_hour: 6,
        tokens_left: 6.0,
        topology: &runtime::TOPOLOGY,
    };
    for case in ["empty", "current", "maximum"] {
        let pending = match case {
            "empty" => (0, None),
            "current" => {
                stats.record_decision(&[], &[0.1, 0.7, 0.2], BotAction::Reply);
                audit.record(crate::brain::reward::FeedbackAttribution::ExactReply);
                (2, Some(149))
            }
            _ => {
                stats.record_decision(&[], &[f32::MAX; 3], BotAction::Reply);
                stats.action_counts = [u64::MAX; 3];
                stats.forced_replies = u64::MAX;
                stats.settled_total = u64::MAX;
                audit = LearningAudit {
                    exact: u64::MAX,
                    unique: u64::MAX,
                    duplicate: u64::MAX,
                    ambiguous: u64::MAX,
                    expired: u64::MAX,
                    unsupported: u64::MAX,
                };
                view.epsilon = f32::MAX;
                view.learn_steps = u64::MAX;
                view.experiences = u64::MAX;
                view.buffer_len = usize::MAX;
                view.buffer_capacity = usize::MAX;
                view.budget_per_hour = u32::MAX;
                view.tokens_left = f32::MAX;
                (usize::MAX, Some(u64::MAX))
            }
        };
        let text = render_brain_diagnostics(&stats, &view, audit, pending, BrainGuard::Ready);
        println!("{case} ({} chars):\n{text}", text.chars().count());
        assert!(text.chars().count() <= 2000);
        for field in [
            "accepted exact reply/reaction",
            "unique scoped",
            "duplicate",
            "ambiguous",
            "expired",
            "unsupported",
            "oldest age",
            "Guard reason",
        ] {
            assert!(text.contains(field));
        }
        for secret in [
            "discord:",
            "member",
            "turn id",
            "raw ask",
            "probability",
            "truth score",
        ] {
            assert!(!text.contains(secret));
        }
        if case != "empty" {
            assert!(text.contains("action values"));
        }
    }
    let mut settings = GuildSettings::default();
    assert_eq!(
        BrainGuard::from_settings(&settings, false, "discord:c").label(),
        "learning off"
    );
    settings.learning_enabled = true;
    assert!(
        BrainGuard::from_settings(&settings, true, "discord:c")
            .label()
            .contains("quiet")
    );
    assert_eq!(
        BrainGuard::from_settings(&settings, false, "discord:c").label(),
        "unsolicited actions off"
    );
}
