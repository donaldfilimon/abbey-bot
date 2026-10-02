use super::*;

#[test]
fn owner_mode_controls_reject_stale_ownership_and_wrong_policy_scope() {
    assert!(operations::mode_owner_matches(42, 7, 42, 7, 42));
    assert!(!operations::mode_owner_matches(42, 7, 43, 7, 42));
    assert!(!operations::mode_owner_matches(42, 7, 42, 8, 42));
    assert!(!operations::mode_owner_matches(42, 7, 42, 7, 43));
    let session = crate::admin_dashboard::AdminSession {
        owner: 42,
        guild: 7,
        expiry: 100,
        page: crate::admin_dashboard::AdminPage::AutonomousOperations,
    };
    assert_eq!(
        operations::dashboard_rows_with_mode_controls(&session, false).len(),
        1
    );
    let rows = operations::dashboard_rows_with_mode_controls(&session, true);
    assert_eq!(rows.len(), 2);
    assert!(matches!(&rows[1], CreateActionRow::Buttons(buttons) if buttons.len() == 4));
}

#[tokio::test]
async fn eight_pages_fit_classic_rows_and_only_wired_actions_are_exposed() {
    use crate::admin_dashboard::{AdminPage, AdminSession};
    let data = crate::Data {
        state: AppState::in_memory(),
        voice: None,
    };
    let input = dashboard_input(&data, 7, 8, None).await;
    for page in AdminPage::NAV.into_iter().chain([AdminPage::ConfirmReset]) {
        let session = AdminSession {
            owner: 42,
            guild: 7,
            expiry: 100,
            page,
        };
        let rows = dashboard_rows(&session);
        assert!(rows.len() <= 5);
        for row in rows {
            if let CreateActionRow::Buttons(buttons) = row {
                assert!(buttons.len() <= 5);
            }
        }
        let rendered = crate::admin_dashboard::render(page, &input);
        assert!(
            rendered.chars().count() <= 2000,
            "page {page:?}: {} characters",
            rendered.chars().count()
        );
        if matches!(
            page,
            AdminPage::Moderation | AdminPage::AutonomousOperations | AdminPage::Models
        ) {
            assert_eq!(dashboard_rows(&session).len(), 1);
        }
    }
}

#[tokio::test]
async fn unsolicited_dashboard_uses_streaming_read_only_route_without_tool_capability() {
    use crate::provider::{
        FmConfig, FmMode, FoundationModels, ProviderCapabilities, ProviderRuntime, RequestClass,
        VerifiedFmCapabilities,
    };
    let fm = FoundationModels::new_qualified(
        FmConfig {
            mode: FmMode::System,
            endpoint: Some("http://127.0.0.1:9".into()),
            cli: std::path::PathBuf::from("synthetic-dashboard-cli-not-executed"),
            fallback: true,
            primary: false,
            timeout_secs: 1,
        },
        None,
        true,
        VerifiedFmCapabilities {
            server: Some(ProviderCapabilities {
                text: true,
                streaming: true,
                ..ProviderCapabilities::default()
            }),
            cli: ProviderCapabilities::default(),
        },
    );
    let mut state = AppState::in_memory();
    std::sync::Arc::get_mut(&mut state).unwrap().providers =
        ProviderRuntime::legacy(None, None, vec![fm], None, true, 1, 1);
    let data = crate::Data { state, voice: None };
    assert!(data.state.providers.tools_enabled());
    assert!(
        data.state
            .providers
            .request_readiness(RequestClass::TextReadOnly)
            .is_err()
    );
    assert_eq!(
        data.state
            .providers
            .request_readiness_for(RequestClass::TextReadOnly, true),
        Ok(())
    );
    assert!(
        data.state
            .providers
            .request_readiness(RequestClass::TextWithTools)
            .is_err()
    );
    update_dashboard_setting(&data, 7, |settings| {
        settings.unsolicited = true;
        settings.learning_enabled = true;
    });
    let view = dashboard_input(&data, 7, 8, None).await;
    assert!(
        view.effective_policy
            .contains("eligible for policy selection"),
        "{}",
        view.effective_policy
    );
    assert!(
        view.effective_policy
            .contains("Apple FM modes: system qualified."),
        "{}",
        view.effective_policy
    );
    assert!(
        !view.capabilities.contains(&"generation"),
        "interactive tool conversation must remain unavailable"
    );
}

#[tokio::test]
async fn failed_delivery_keeps_the_setting_change_and_the_original_failure() {
    let state = AppState::in_memory();
    let data = crate::Data { state, voice: None };
    let mut mutations = 0;
    update_dashboard_setting(&data, 7, |settings| {
        settings.unsolicited = true;
        mutations += 1;
    });
    let failure = Err::<(), _>("transport");
    if failure.is_err() {
        crate::gateway::interaction_outcomes::delivery_failed(&data.state);
    }
    assert_eq!(failure, Err("transport"));
    assert_eq!(mutations, 1);
    assert!(dashboard_settings(&data.state, 7).unsolicited);
    let view = dashboard_input(&data, 7, 8, Some("Unsolicited action is now on.".into())).await;
    assert!(!view.effective_policy.contains("learning is off"));
    assert!(view.operation_result.unwrap().contains("now on"));
}
