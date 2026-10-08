//! Actual native retry/admission paths; parent fixtures fetch all native authority.
use super::*;

#[tokio::test]
async fn stopped_exact_retry_confirms_existing_native_capture_after_private_delivery_failure() {
    let fixture = DiscordFixture::new().await;
    native_routes(&fixture);
    let directory = Directory::new();
    let data = native_data(&fixture, &directory);
    let mut supervisor = ServiceSupervisor::new();
    supervisor.finish_startup();
    let writer = data.state.attach_service(supervisor.operations());
    let options = options();
    // Actual publication finishes, then Discord refuses the private webhook.
    // The caller lacks a delivered receipt. This is delivery uncertainty;
    // persist tests separately exercise real post-rename directory-sync errors.
    fixture.fail_next_followup.store(true, Ordering::SeqCst);
    assert!(
        dispatch(&fixture, &data, &options, &capture_interaction())
            .await
            .is_err()
    );
    let first = fixture.take_requests();
    assert_deferred_first(
        &first,
        command_by_key(&options.commands, CommandKey::Modcall),
    );
    assert!(first.iter().any(|r| r.route.contains("/webhooks/")));
    assert!(
        first
            .iter()
            .filter(|r| r.method != "GET")
            .all(|r| r.route.ends_with("/callback") || r.route.contains("/webhooks/"))
    );
    let cases = directory.cases();
    assert_eq!(cases.cases.len(), 1);
    assert_eq!(cases.revision, 1);
    let id = cases.cases.keys().next().unwrap().clone();
    let original = cases.cases[&id].clone();
    let bytes = std::fs::read(directory.ledger()).unwrap();
    directory.stop();
    // Known exact existing record permits readback/republication only. The
    // stopped current owner policy still refuses every new/edited source.
    dispatch(&fixture, &data, &options, &capture_interaction())
        .await
        .unwrap();
    let requests = fixture.take_requests();
    let text = private_reply(&requests, &options, CommandKey::Modcall);
    println!("Actual native stopped exact publication retry: {text}");
    assert!(text.contains(&id));
    assert!(text.to_ascii_lowercase().contains("existing"));
    assert!(text.contains("No action taken."));
    assert!(!text.contains("not a saved case"));
    assert!(
        requests
            .iter()
            .any(|r| r.method == "GET" && r.route.ends_with(&format!("/messages/{SOURCE}")))
    );
    assert_eq!(directory.cases().cases[&id], original);
    assert_eq!(directory.cases().revision, 1);
    assert_eq!(std::fs::read(directory.ledger()).unwrap(), bytes);
    assert!(!String::from_utf8(bytes).unwrap().contains(SOURCE_TEXT));
    finish(supervisor, writer).await;
}

#[tokio::test]
async fn disabled_or_stopped_no_existing_native_capture_creates_no_operational_paths() {
    for stopped in [false, true] {
        let fixture = DiscordFixture::new().await;
        native_routes(&fixture);
        let directory = Directory::new();
        if stopped {
            directory.stop();
        } else {
            directory.disable_capture();
        }
        let before_policy = std::fs::read(directory.policy()).unwrap();
        let data = native_data(&fixture, &directory);
        let before_memory = runtime::AppState::lock(&data.state.stores).clone();
        let mut supervisor = ServiceSupervisor::new();
        supervisor.finish_startup();
        let writer = data.state.attach_service(supervisor.operations());
        let options = options();
        assert!(!directory.0.join("community-operations").exists());
        dispatch(&fixture, &data, &options, &capture_interaction())
            .await
            .unwrap();
        let requests = fixture.take_requests();
        let text = private_reply(&requests, &options, CommandKey::Modcall);
        assert!(text.contains("not a saved case"));
        assert!(!text.contains("Saved shadow case"));
        assert!(
            requests
                .iter()
                .any(|r| r.method == "GET" && r.route.ends_with(&format!("/messages/{SOURCE}")))
        );
        assert!(
            !directory.0.join("community-operations").exists(),
            "read-only existing preflight must never create a directory"
        );
        assert!(!directory.policy().with_extension("mode-lock").exists());
        assert!(!directory.0.join(crate::persist::STATE_FILE).exists());
        assert!(!directory.0.join(crate::persist::WDBX_FILE).exists());
        let entries: Vec<_> = std::fs::read_dir(&directory.0)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(entries, vec![std::ffi::OsString::from("owner-policy.json")]);
        assert_eq!(std::fs::read(directory.policy()).unwrap(), before_policy);
        assert!(runtime::AppState::lock(&data.state.stores).payload_eq(&before_memory));
        finish(supervisor, writer).await;
    }
}
