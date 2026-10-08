// Apply as src/runtime/engagement_follow_up/tests/contact_reason.rs; register
// #[path = "tests/contact_reason.rs"] mod contact_reason; in tests.rs.
// Exercise the actual settings/Budget/ReplyCooldown and RateLimits; no new API.
use super::*;
use crate::{
    guild::GuildSettings,
    pipeline::{Outcome, RateLimits},
};

const GUILD_KEY: &str = "discord:7";
const CHANNEL_KEY: &str = "discord:3";
fn guild_origin() -> EngagementScope {
    EngagementScope::Guild {
        guild: 7,
        channel: 3,
    }
}
fn configure(state: &AppState, change: impl FnOnce(&mut GuildSettings)) -> GuildSettings {
    let mut stores = AppState::lock(&state.stores);
    AppState::lock(&state.guilds).update(GUILD_KEY, &mut *stores, |settings| {
        settings.enabled = true;
        settings.unsolicited = true;
        settings.unsolicited_per_hour = 2;
        settings.reply_cooldown_seconds = 20;
        settings.unsolicited_channels = Some(BTreeSet::from([CHANNEL_KEY.to_string()]));
        change(settings);
    })
}
fn acquire(state: &AppState, settings: &GuildSettings, at: u64) -> Result<(), Outcome> {
    RateLimits {
        budget: &state.budget,
        cooldown: &state.cooldown,
    }
    .try_acquire(GUILD_KEY, CHANNEL_KEY, settings, at)
}
fn rate_image(state: &AppState) -> (String, String) {
    // Same cooldown-before-budget lock order as the real acquisition owner.
    let cooldown = AppState::lock(&state.cooldown);
    let budget = AppState::lock(&state.budget);
    (format!("{cooldown:?}"), format!("{budget:?}"))
}
fn assert_read_only(
    state: &AppState,
    scope: &EngagementScope,
    at: u64,
    expected: FollowUpDecision,
) {
    let rates = rate_image(state);
    let stores = serde_json::to_value(&*AppState::lock(&state.stores)).unwrap();
    let cached = AppState::lock(&state.guilds).is_cached(GUILD_KEY);
    for _ in 0..3 {
        assert_eq!(state.task_follow_up_contact_reason(scope, at), expected);
        assert_eq!(
            rate_image(state),
            rates,
            "inspection must not refill, spend, or record a channel reply"
        );
        assert_eq!(
            serde_json::to_value(&*AppState::lock(&state.stores)).unwrap(),
            stores,
            "inspection must not provision or publish settings"
        );
        assert_eq!(
            AppState::lock(&state.guilds).is_cached(GUILD_KEY),
            cached,
            "inspection must not hydrate a settings cache"
        );
    }
}

#[test]
fn task_follow_up_contact_unknown_guild_is_disabled_without_provisioning_or_caching() {
    let state = AppState::in_memory();
    assert!(!AppState::lock(&state.stores).guilds.contains_key(GUILD_KEY));
    assert!(!AppState::lock(&state.guilds).is_cached(GUILD_KEY));
    assert_read_only(&state, &guild_origin(), NOW, FollowUpDecision::Disabled);
}

#[test]
fn task_follow_up_contact_actual_guild_and_channel_settings_disable_before_shared_capacity() {
    for blocker in 0..4 {
        let state = AppState::in_memory();
        let settings = configure(&state, |settings| {
            match blocker {
                0 => settings.enabled = false,
                1 => settings.unsolicited = false,
                2 => settings.unsolicited_channels = Some(BTreeSet::new()),
                _ => settings.unsolicited_channels = Some(BTreeSet::from(["discord:8".into()])),
            }
            settings.unsolicited_per_hour = 0;
        });
        assert_eq!(acquire(&state, &settings, NOW), Err(Outcome::OverBudget));
        assert_read_only(&state, &guild_origin(), NOW, FollowUpDecision::Disabled);
    }
}

#[test]
fn task_follow_up_contact_pipeline_consumption_is_same_guild_budget_and_read_only() {
    let state = AppState::in_memory();
    let settings = configure(&state, |settings| {
        settings.unsolicited_per_hour = 1;
        settings.reply_cooldown_seconds = 0;
    });
    assert_read_only(&state, &guild_origin(), NOW, FollowUpDecision::Allowed);
    acquire(&state, &settings, NOW).unwrap();
    assert_eq!(
        AppState::lock(&state.budget).tokens_left(GUILD_KEY, 1, NOW),
        0.0
    );
    assert_read_only(&state, &guild_origin(), NOW, FollowUpDecision::Budget);
    // A different native channel shares this actual guild budget.
    configure(&state, |settings| {
        settings.unsolicited_per_hour = 1;
        settings.reply_cooldown_seconds = 0;
        settings.unsolicited_channels = None;
    });
    assert_read_only(
        &state,
        &EngagementScope::Guild {
            guild: 7,
            channel: 8,
        },
        NOW,
        FollowUpDecision::Budget,
    );
    // Read-only refill can show a later token; it must not reset bucket.last.
    assert_read_only(
        &state,
        &guild_origin(),
        NOW + 3600,
        FollowUpDecision::Allowed,
    );
    assert_read_only(&state, &guild_origin(), NOW, FollowUpDecision::Budget);
    assert_eq!(acquire(&state, &settings, NOW), Err(Outcome::OverBudget));
}

#[test]
fn task_follow_up_contact_pipeline_reply_sets_exact_channel_cooldown_without_extra_spend() {
    let state = AppState::in_memory();
    let settings = configure(&state, |_| {});
    acquire(&state, &settings, NOW).unwrap();
    assert_eq!(
        AppState::lock(&state.budget).tokens_left(GUILD_KEY, 2, NOW),
        1.0
    );
    assert_read_only(&state, &guild_origin(), NOW, FollowUpDecision::Cooldown);
    assert_eq!(acquire(&state, &settings, NOW), Err(Outcome::CooledDown));
    assert_read_only(
        &state,
        &guild_origin(),
        NOW + 19,
        FollowUpDecision::Cooldown,
    );
    assert_read_only(&state, &guild_origin(), NOW + 20, FollowUpDecision::Allowed);
    // Another admitted channel is not suppressed by this channel's reply.
    configure(&state, |settings| settings.unsolicited_channels = None);
    assert_read_only(
        &state,
        &EngagementScope::Guild {
            guild: 7,
            channel: 8,
        },
        NOW,
        FollowUpDecision::Allowed,
    );
}

#[test]
fn task_follow_up_contact_matches_pipeline_budget_before_cooldown_precedence() {
    let state = AppState::in_memory();
    let settings = configure(&state, |settings| settings.unsolicited_per_hour = 1);
    acquire(&state, &settings, NOW).unwrap();
    assert!(!AppState::lock(&state.cooldown).permitted(CHANNEL_KEY, 20, NOW));
    assert_eq!(acquire(&state, &settings, NOW), Err(Outcome::OverBudget));
    assert_read_only(&state, &guild_origin(), NOW, FollowUpDecision::Budget);
    assert_read_only(&state, &guild_origin(), NOW + 20, FollowUpDecision::Budget);
    assert_read_only(
        &state,
        &guild_origin(),
        NOW + 3600,
        FollowUpDecision::Allowed,
    );
}

#[test]
fn task_follow_up_contact_global_quiet_and_unhealthy_observation_are_closed_before_guild_facts() {
    let mut state = AppState::in_memory();
    Arc::get_mut(&mut state).unwrap().quiet = true;
    assert_read_only(&state, &guild_origin(), NOW, FollowUpDecision::Quiet);
    assert_read_only(&state, &origin(), NOW, FollowUpDecision::Quiet);
    state
        .engagement_events_healthy
        .store(false, Ordering::SeqCst);
    assert_read_only(&state, &guild_origin(), NOW, FollowUpDecision::AccessDenied);
    assert_read_only(&state, &origin(), NOW, FollowUpDecision::AccessDenied);
}

#[tokio::test]
async fn task_follow_up_status_reserved_attempt_keeps_its_existing_charge_without_inspection_mutation()
 {
    let h = RequestHarness::new(DestinationPreference::Origin);
    h.persist_seed().await;
    let transport = MockEngagementTransport::new(&h, false);
    h.state
        .request_task_follow_up(h.request(), transport.clone(), || NOW)
        .await
        .unwrap();
    h.state
        .commit_work_owned(|work| {
            let revision = work.engagement.candidates[&1].revision;
            work.engagement.reserve(1, revision, NOW)
        })
        .await
        .unwrap();
    let disk_before = std::fs::read(Stores::state_path(&h.dir)).unwrap();
    let store_before = serde_json::to_value(&*AppState::lock(&h.state.stores)).unwrap();
    let rates_before = rate_image(&h.state);
    for _ in 0..3 {
        let text = h
            .state
            .task_follow_up_status(2, &origin(), transport.as_ref(), || NOW)
            .await;
        assert!(text.contains("Candidate 1: Reserved"));
        assert_eq!(
            AppState::lock(&h.state.stores)
                .work
                .engagement
                .charges
                .len(),
            1
        );
        assert_eq!(
            AppState::lock(&h.state.stores).work.engagement.candidates[&1].state,
            CandidateState::Reserved
        );
        assert_eq!(
            serde_json::to_value(&*AppState::lock(&h.state.stores)).unwrap(),
            store_before
        );
        assert_eq!(rate_image(&h.state), rates_before);
        assert_eq!(
            std::fs::read(Stores::state_path(&h.dir)).unwrap(),
            disk_before
        );
    }
    assert_eq!(transport.generations.load(Ordering::SeqCst), 0);
    assert_eq!(transport.sends.load(Ordering::SeqCst), 0);
    h.finish().await;
}
