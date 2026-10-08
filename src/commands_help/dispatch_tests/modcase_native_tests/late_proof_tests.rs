//! Real registered /modcall proof refresh under a blocked capture policy.
//! Integrate as commands_help::dispatch_tests::modcase_native_tests::late_proof_tests.
//! Dynamic GET responses are route-specific and sticky at their final value.
use super::*;

const HIGHER_ROLE: u64 = STAFF_ROLE + 1;

#[derive(Clone, Copy, Debug)]
enum TargetTransition {
    StaticOrdinary,
    BecameStaff,
    HigherOrdinaryRole,
}

fn target_sequence(fixture: &DiscordFixture, transition: TargetTransition) {
    if matches!(transition, TargetTransition::StaticOrdinary) {
        return;
    }
    let target_route = format!("/guilds/{GUILD}/members/{OTHER}");
    let mut routes = fixture.native_responses.lock().unwrap();
    let ordinary = routes[&target_route].clone();
    let mut refreshed: Member = serde_json::from_value(ordinary.clone()).unwrap();
    match transition {
        TargetTransition::StaticOrdinary => unreachable!(),
        TargetTransition::BecameStaff => refreshed.roles.push(RoleId::new(STAFF_ROLE)),
        TargetTransition::HigherOrdinaryRole => {
            let guild_route = format!("/guilds/{GUILD}");
            let mut current: Guild = serde_json::from_value(routes[&guild_route].clone()).unwrap();
            let mut higher = Role::default();
            higher.id = RoleId::new(HIGHER_ROLE);
            higher.guild_id = GuildId::new(GUILD);
            higher.position = 20;
            // This role grants no staff permission. Only hierarchy changes.
            higher.permissions = Permissions::VIEW_CHANNEL | Permissions::READ_MESSAGE_HISTORY;
            current.roles.insert(higher.id, higher);
            routes.insert(guild_route, serde_json::to_value(current).unwrap());
            refreshed.roles.push(RoleId::new(HIGHER_ROLE));
        }
    }
    drop(routes);
    fixture.native_response_sequences.lock().unwrap().insert(
        target_route,
        std::collections::VecDeque::from([ordinary, serde_json::to_value(refreshed).unwrap()]),
    );
}

async fn blocked_reply(stopped: bool, transition: TargetTransition) -> String {
    let fixture = DiscordFixture::new().await;
    native_routes(&fixture);
    target_sequence(&fixture, transition);
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
    dispatch(&fixture, &data, &options, &capture_interaction())
        .await
        .unwrap();
    let requests = fixture.take_requests();
    let text = private_reply(&requests, &options, CommandKey::Modcall).to_owned();
    println!("Actual blocked contextual reply, stopped={stopped}, {transition:?}: {text}");
    let target_route = format!("/guilds/{GUILD}/members/{OTHER}");
    let target_reads = requests
        .iter()
        .filter(|r| r.method == "GET" && r.route.ends_with(&target_route))
        .count();
    assert_eq!(
        target_reads, 2,
        "the regression must reach initial and refreshed target REST proof"
    );
    assert_eq!(
        requests
            .iter()
            .filter(|r| r.method == "GET" && r.route.ends_with(&format!("/messages/{SOURCE}")))
            .count(),
        2,
        "both proofs must observe the same actual source version"
    );
    {
        let queues = fixture.native_response_sequences.lock().unwrap();
        if matches!(transition, TargetTransition::StaticOrdinary) {
            assert!(
                queues.is_empty(),
                "the valid control uses static native facts"
            );
        } else {
            let remaining = &queues[&target_route];
            assert_eq!(remaining.len(), 1, "initial target response was consumed");
            let expected_roles = match transition {
                TargetTransition::StaticOrdinary => unreachable!(),
                TargetTransition::BecameStaff => json!([STAFF_ROLE.to_string()]),
                TargetTransition::HigherOrdinaryRole => json!([HIGHER_ROLE.to_string()]),
            };
            assert_eq!(remaining.front().unwrap()["roles"], expected_roles);
        }
    }
    assert!(text.to_ascii_lowercase().contains("no action taken"));
    assert!(!text.contains("Saved shadow case"));
    assert!(!text.contains("Confirmed existing shadow case"));
    assert!(!text.contains(SOURCE_TEXT));
    assert!(!directory.0.join("community-operations").exists());
    assert!(!directory.policy().with_extension("mode-lock").exists());
    assert!(!directory.0.join(crate::persist::STATE_FILE).exists());
    assert!(!directory.0.join(crate::persist::WDBX_FILE).exists());
    assert_eq!(std::fs::read(directory.policy()).unwrap(), before_policy);
    assert!(runtime::AppState::lock(&data.state.stores).payload_eq(&before_memory));
    finish(supervisor, writer).await;
    text
}

#[tokio::test]
async fn blocked_contextual_refresh_preserves_unchanged_ordinary_target_control() {
    for stopped in [false, true] {
        let text = blocked_reply(stopped, TargetTransition::StaticOrdinary).await;
        assert!(text.contains("Proposed action: delete this offending message."));
        assert!(text.contains("Proposed timeout: 10 minutes maximum."));
        assert!(text.contains("not a saved case"));
    }
}

#[tokio::test]
async fn blocked_contextual_refresh_refuses_target_that_has_just_become_staff() {
    let mut mismatches = Vec::new();
    for stopped in [false, true] {
        let text = blocked_reply(stopped, TargetTransition::BecameStaff).await;
        if text.contains("Proposed action: delete this offending message.")
            || text.contains("Proposed timeout:")
            || !text.contains("staff and owner targets are excluded")
            || !text.contains("Human review required.")
        {
            mismatches.push(format!("stopped={stopped}: {text}"));
        }
    }
    assert!(
        mismatches.is_empty(),
        "Fresh native staff exclusion must govern the unsaved reply:\n{}",
        mismatches.join("\n\n")
    );
}

#[tokio::test]
async fn blocked_contextual_refresh_refuses_target_with_new_higher_ordinary_role() {
    let mut mismatches = Vec::new();
    for stopped in [false, true] {
        let text = blocked_reply(stopped, TargetTransition::HigherOrdinaryRole).await;
        if text.contains("Proposed action: delete this offending message.")
            || text.contains("Proposed timeout:")
            || !text.contains("Current moderator permissions or target hierarchy do not qualify")
        {
            mismatches.push(format!("stopped={stopped}: {text}"));
        }
    }
    assert!(
        mismatches.is_empty(),
        "Fresh native role hierarchy must govern the unsaved reply:\n{}",
        mismatches.join("\n\n")
    );
}
