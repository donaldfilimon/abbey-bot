// Integrate as a child of commands_help::dispatch_tests::modcase_native_tests.
// Root owns source registration and Cargo execution; this is external only.
use super::*;

#[tokio::test]
async fn actual_unknown_modcase_show_is_read_only_when_capture_is_stopped_or_disabled() {
    for stopped in [true, false] {
        let fixture = DiscordFixture::new().await;
        native_routes(&fixture);
        let directory = Directory::new();
        if stopped {
            directory.stop();
        } else {
            directory.disable_capture();
        }
        let data = native_data(&fixture, &directory);
        let mut supervisor = ServiceSupervisor::new();
        supervisor.finish_startup();
        let writer = data.state.attach_service(supervisor.operations());
        let options = options();
        let absent = "a".repeat(64);
        let before_policy = std::fs::read(directory.policy()).unwrap();
        let operations = directory.0.join("community-operations");
        assert!(!operations.exists());
        let text = invoke(
            &fixture,
            &data,
            &options,
            "show",
            OTHER,
            CHANNEL,
            (&absent, 0, 0),
        )
        .await;
        assert!(!text.contains(&absent));
        assert!(!text.contains("receipt saved"));
        assert!(
            !operations.exists(),
            "private inspection must not create a missing operational directory"
        );
        assert_eq!(std::fs::read(directory.policy()).unwrap(), before_policy);
        assert!(!directory.0.join(crate::persist::STATE_FILE).exists());
        assert!(!directory.0.join(crate::persist::WDBX_FILE).exists());
        finish(supervisor, writer).await;
    }
}
