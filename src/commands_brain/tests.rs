use super::dashboard::{AdminPreparation, acknowledged_admin_preparation, dashboard_rows, page_select_row};
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
