use super::*;
use crate::brain::{
    reward::{ReactionKey, ReplyTurn},
    state::STATE_DIMENSIONS,
};
const G: &str = "discord:1";
const U: &str = "discord:7";
fn turn(id: &str, user: &str, scope: &str, at: u64) -> ReplyTurn {
    ReplyTurn {
        state: vec![0.0; STATE_DIMENSIONS],
        action: 1,
        sent_native_message_id: id.into(),
        scope: "discord:2".into(),
        scoped_guild_id: scope.into(),
        ask: "Which current version?".into(),
        asker: user.into(),
        now: at,
    }
}
#[test]
fn erase_restart_cache_and_late_settlement_do_not_resurrect() {
    let state = AppState::in_memory();
    let original = turn("own", U, G, 100);
    {
        let mut stores = AppState::lock(&state.stores);
        AppState::lock(&state.brains).brain(G, &*stores, 100);
        AppState::lock(&state.social).record_interaction(U, G, 1.0, 100, &mut *stores);
    }
    AppState::lock(&state.rewards).register_turn(original.clone());
    state.observe_style(G, U, "too long", 100);
    let report = state.erase_learning_now(G, Some(7), 101).unwrap();
    assert_eq!(report.pending, 1);
    assert_eq!(report.style, 1);
    assert!(!report.aggregate_reset);
    state.flush_social();
    state.settle_rewards();
    AppState::lock(&state.rewards).register_turn(original);
    state.observe_style(G, U, "too long", 100);
    let (stores, _) = state.take_snapshot(102);
    assert!(stores.reputations.is_empty());
    assert!(stores.events.is_empty());
    assert!(stores.pending_rewards.is_empty());
    let encoded = serde_json::to_vec(&stores).unwrap();
    let back: Stores = serde_json::from_slice(&encoded).unwrap();
    back.reward_recovery
        .validate(&back.pending_rewards)
        .unwrap();
    let mut restored = RewardCollector::new();
    restored
        .restore_recovered(back.pending_rewards, back.reward_recovery)
        .unwrap();
    restored.register_turn(turn("late", U, G, 100));
    restored.register_turn(turn("during", U, G, 401));
    assert_eq!(restored.pending_len(), 0);
    restored.register_turn(turn("new", U, G, 402));
    assert_eq!(restored.pending_len(), 1);
    assert_eq!(
        state.erase_learning_now(G, Some(7), 102).unwrap().pending,
        0
    );
}
#[test]
fn erase_preserves_other_members_facts_settings_and_scopes() {
    let state = AppState::in_memory();
    for (id, user, g) in [
        ("own", U, G),
        ("other", "discord:8", G),
        ("elsewhere", U, "discord:3"),
        ("dm", U, "discord:dm:7"),
    ] {
        AppState::lock(&state.rewards).register_turn(turn(id, user, g, 100));
        let mut stores = AppState::lock(&state.stores);
        AppState::lock(&state.social).record_interaction(user, g, 1.0, 100, &mut *stores);
    }
    state
        .memory_service()
        .remember(G, U, "I like Rust", 100)
        .unwrap();
    let facts = AppState::lock(&state.stores).memory.clone();
    let settings = AppState::lock(&state.stores).guilds.clone();
    let report = state.erase_learning_now(G, Some(7), 101).unwrap();
    assert_eq!(report.pending, 1);
    state.flush_social();
    let stores = AppState::lock(&state.stores);
    assert_eq!(stores.memory, facts);
    assert_eq!(stores.guilds, settings);
    assert_eq!(stores.events.len(), 3);
    assert_eq!(stores.reputations.len(), 3);
    assert_eq!(AppState::lock(&state.rewards).pending_len(), 3);
}
#[test]
fn erasure_reverses_only_linkable_reaction_and_invalidates_correction_observer() {
    let state = AppState::in_memory();
    let mut rewards = AppState::lock(&state.rewards);
    rewards.register_turn(turn("other", "discord:8", G, 100));
    for user in [U, "discord:8"] {
        rewards.reaction(
            ReactionKey {
                scope: "discord:2".into(),
                message: "other".into(),
                reactor_hash: crate::brain::addenda::member_hash(user),
                emoji: "👍".into(),
            },
            true,
            101,
        );
    }
    let source = rewards
        .correction_source("discord:2", G, U, Some("other"), "that is wrong", 102, true)
        .unwrap();
    drop(rewards);
    let report = state.erase_learning_now(G, Some(7), 103).unwrap();
    assert_eq!(report.reactions, 1);
    assert_eq!(report.pending, 0);
    let mut rewards = AppState::lock(&state.rewards);
    assert!(!rewards.correction_current(&source, 104));
    assert_eq!(rewards.export_pending()[0].1.positive_reactions, 1);
    assert_eq!(
        rewards.reaction(
            ReactionKey {
                scope: "discord:2".into(),
                message: "other".into(),
                reactor_hash: crate::brain::addenda::member_hash(U),
                emoji: "👍".into()
            },
            true,
            104
        ),
        crate::brain::reward::FeedbackAttribution::Expired
    );
}
#[test]
fn erasure_scope_reset_discards_loaded_weights_replay_audit_and_old_checkpoint() {
    let state = AppState::in_memory();
    {
        let stores = AppState::lock(&state.stores);
        let mut brains = AppState::lock(&state.brains);
        brains.brain(G, &*stores, 100);
        brains.brain("discord:3", &*stores, 100);
        brains.remember(
            G,
            RewardCollector::silence_experience(vec![0.0; STATE_DIMENSIONS]),
        );
        brains.record_learning_attribution(G, crate::brain::reward::FeedbackAttribution::Expired);
        brains.record_learning_attribution(
            "discord:3",
            crate::brain::reward::FeedbackAttribution::Expired,
        );
    }
    let (before, _) = state.take_snapshot(100);
    *AppState::lock(&state.checkpoints) = crate::checkpoint_gate::seed(&before.brains);
    let other = before.brains["discord:3"].clone();
    AppState::lock(&state.rewards).register_turn(turn("own", U, G, 100));
    let report = state.erase_learning_now(G, None, 101).unwrap();
    assert!(report.aggregate_reset);
    assert!(AppState::lock(&state.brains).get(G).is_none());
    assert_eq!(
        AppState::lock(&state.brains).learning_audit(G),
        Default::default()
    );
    assert_eq!(
        AppState::lock(&state.brains)
            .learning_audit("discord:3")
            .expired,
        1
    );
    assert!(!AppState::lock(&state.checkpoints).contains_key(G));
    let (after, _) = state.take_snapshot(102);
    assert!(!after.brains.contains_key(G));
    assert_eq!(after.brains["discord:3"], other);
    let back: Stores = serde_json::from_slice(&serde_json::to_vec(&after).unwrap()).unwrap();
    let mut brains = BrainRegistry::new(fresh_brain, DEFAULT_EVICT_AFTER_SECS);
    brains.brain(G, &back, 103);
    assert_eq!(brains.experience_count(G), Some(0));
}

struct Directory(std::path::PathBuf);
impl Directory {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let p = std::env::temp_dir().join(format!(
            "abbey-learning-erase-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir(&p).unwrap();
        Self(p)
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
struct Sink {
    mode: std::sync::atomic::AtomicU8,
    entered: tokio::sync::Notify,
    release: (std::sync::Mutex<bool>, std::sync::Condvar),
}
impl Sink {
    fn new(mode: u8, hold: bool) -> Self {
        Self {
            mode: std::sync::atomic::AtomicU8::new(mode),
            entered: tokio::sync::Notify::new(),
            release: (std::sync::Mutex::new(!hold), std::sync::Condvar::new()),
        }
    }
}
impl PersistenceSink for Sink {
    fn publish(
        &self,
        directory: &std::path::Path,
        destination: &std::path::Path,
        bytes: &[u8],
    ) -> Result<(), crate::persist::PersistErrorCategory> {
        use crate::persist::PersistErrorCategory;
        if destination
            .file_name()
            .is_some_and(|f| f == crate::persist::STATE_FILE)
        {
            self.entered.notify_one();
            let mut released = self.release.0.lock().unwrap();
            while !*released {
                released = self.release.1.wait(released).unwrap();
            }
            match self.mode.load(std::sync::atomic::Ordering::SeqCst) {
                1 => return Err(PersistErrorCategory::SyncTemporary),
                2 => {
                    FsPersistenceSink.publish(directory, destination, bytes)?;
                    return Err(PersistErrorCategory::SyncTemporary);
                }
                3 => return Ok(()),
                _ => {}
            }
        }
        FsPersistenceSink.publish(directory, destination, bytes)
    }
}
struct Release(Arc<Sink>);
impl Drop for Release {
    fn drop(&mut self) {
        *self.0.release.0.lock().unwrap() = true;
        self.0.release.1.notify_all();
    }
}
#[tokio::test]
async fn erasure_retained_publication_reopen_and_stale_snapshot_are_atomic() {
    let dir = Directory::new();
    let sink = Arc::new(Sink::new(0, true));
    let release = Release(sink.clone());
    let state = AppState::in_memory_with_persistence(Some(dir.0.clone()), sink.clone());
    let mut supervisor = crate::service::ServiceSupervisor::new();
    supervisor.finish_startup();
    let mut writer = state.attach_service(supervisor.operations());
    state
        .memory_service()
        .remember(G, U, "uses Rust", 100)
        .unwrap();
    AppState::lock(&state.rewards).register_turn(turn("old", U, G, now()));
    let stale = state.take_snapshot(now());
    let own = state.clone();
    let waiter = tokio::spawn(async move { own.erase_personal_learning(G.into(), 7).await });
    tokio::time::timeout(std::time::Duration::from_secs(5), sink.entered.notified())
        .await
        .unwrap();
    assert_eq!(AppState::lock(&state.rewards).pending_len(), 0);
    waiter.abort();
    assert!(waiter.await.is_err());
    assert_eq!(state.memory_service().facts(G, U), vec!["uses Rust"]);
    drop(release);
    supervisor.next_completion().await;
    let disk = Stores::load(&dir.0).unwrap();
    assert!(disk.pending_rewards.is_empty());
    assert_eq!(disk.reward_recovery.erasure.rows.len(), 1);
    assert!(disk.reward_recovery.erasure.blocks(G, U, now()));
    assert_ne!(
        state.persist_snapshot(stale).canonical_state,
        crate::persist::PersistComponentOutcome::Committed
    );
    assert!(Stores::load(&dir.0).unwrap().pending_rewards.is_empty());
    assert_eq!(
        state
            .erase_personal_learning(G.into(), 7)
            .await
            .unwrap()
            .pending,
        0
    );
    writer.stop();
    writer.joined().await.unwrap();
}
#[tokio::test]
async fn erasure_failed_or_unverified_publication_never_claims_success_or_restores_cache() {
    for mode in [1, 2, 3] {
        let dir = Directory::new();
        let sink = Arc::new(Sink::new(0, false));
        let state = AppState::in_memory_with_persistence(Some(dir.0.clone()), sink.clone());
        let mut supervisor = crate::service::ServiceSupervisor::new();
        supervisor.finish_startup();
        let mut writer = state.attach_service(supervisor.operations());
        AppState::lock(&state.rewards).register_turn(turn("old", U, G, now()));
        state.request_persistence().await.unwrap();
        assert_eq!(Stores::load(&dir.0).unwrap().pending_rewards.len(), 1);
        sink.mode.store(mode, std::sync::atomic::Ordering::SeqCst);
        assert_eq!(
            state.erase_personal_learning(G.into(), 7).await,
            Err(WorkError::Persistence)
        );
        assert_eq!(AppState::lock(&state.rewards).pending_len(), 0);
        assert!(
            AppState::lock(&state.stores)
                .reward_recovery
                .erasure
                .blocks(G, U, now())
        );
        let reopened = Stores::load(&dir.0).unwrap();
        assert_eq!(reopened.pending_rewards.len(), usize::from(mode != 2));
        assert_eq!(
            reopened.reward_recovery.erasure.rows.len(),
            usize::from(mode == 2)
        );
        sink.mode.store(0, std::sync::atomic::Ordering::SeqCst);
        assert_eq!(
            state
                .erase_personal_learning(G.into(), 7)
                .await
                .unwrap()
                .pending,
            0
        );
        let disk = Stores::load(&dir.0).unwrap();
        assert!(disk.pending_rewards.is_empty());
        assert_eq!(disk.reward_recovery.erasure.rows.len(), 1);
        writer.stop();
        writer.joined().await.unwrap();
    }
}
#[tokio::test]
async fn erasure_queued_expired_manager_authority_cannot_reset() {
    let dir = Directory::new();
    let state =
        AppState::in_memory_with_persistence(Some(dir.0.clone()), Arc::new(FsPersistenceSink));
    let mut supervisor = crate::service::ServiceSupervisor::new();
    supervisor.finish_startup();
    let mut writer = state.attach_service(supervisor.operations());
    AppState::lock(&state.rewards).register_turn(turn("old", U, G, now()));
    let owner = state.persistence_preparation.lock().await;
    let created = now() - 60;
    let retained = state.clone();
    let waiter =
        tokio::spawn(async move { retained.reset_learning_scope(G.into(), created).await });
    // Valid at initial admission, then expires while the retained owner waits.
    tokio::time::sleep(std::time::Duration::from_millis(2100)).await;
    assert!(!waiter.is_finished());
    drop(owner);
    assert_eq!(waiter.await.unwrap(), Err(WorkError::Stale));
    assert_eq!(AppState::lock(&state.rewards).pending_len(), 1);
    assert!(
        AppState::lock(&state.stores)
            .reward_recovery
            .erasure
            .rows
            .is_empty()
    );
    writer.stop();
    writer.joined().await.unwrap();
}

#[tokio::test]
async fn erasure_reset_reopens_canonical_without_pre_reset_checkpoint_or_facts_loss() {
    let dir = Directory::new();
    let state =
        AppState::in_memory_with_persistence(Some(dir.0.clone()), Arc::new(FsPersistenceSink));
    let mut supervisor = crate::service::ServiceSupervisor::new();
    supervisor.finish_startup();
    let mut writer = state.attach_service(supervisor.operations());
    state
        .memory_service()
        .remember(G, U, "uses Rust", 100)
        .unwrap();
    {
        let stores = AppState::lock(&state.stores);
        let mut brains = AppState::lock(&state.brains);
        brains.brain(G, &*stores, now());
        brains.remember(
            G,
            RewardCollector::silence_experience(vec![0.0; STATE_DIMENSIONS]),
        );
    }
    assert_eq!(
        state.request_persistence().await.unwrap().canonical_state,
        crate::persist::PersistComponentOutcome::Committed
    );
    let before = Stores::load(&dir.0).unwrap();
    assert!(before.brains.contains_key(G));
    *AppState::lock(&state.checkpoints) = crate::checkpoint_gate::seed(&before.brains);
    assert!(
        state
            .reset_learning_scope(G.into(), now())
            .await
            .unwrap()
            .aggregate_reset
    );
    let disk = Stores::load(&dir.0).unwrap();
    assert!(!disk.brains.contains_key(G));
    assert_eq!(disk.memory, before.memory);
    // A newly loaded replacement is not implicitly admitted to a covered scope.
    {
        let stores = AppState::lock(&state.stores);
        AppState::lock(&state.brains).brain(G, &*stores, now());
    }
    let (mut snapshot, _) = state.take_snapshot(now());
    {
        let checkpoints = AppState::lock(&state.checkpoints);
        assert!(!checkpoints.contains_key(G));
        assert_eq!(
            crate::checkpoint_gate::plan(&snapshot.brains, &checkpoints, |g| g == G).len(),
            1
        );
        crate::checkpoint_gate::restrict_to_admitted(&mut snapshot, &checkpoints, |g| g == G);
        assert!(!snapshot.brains.contains_key(G));
    }
    writer.stop();
    writer.joined().await.unwrap();
}

#[tokio::test]
async fn erasure_late_actual_observation_refuses_without_poisoning_other_members() {
    let dir = Directory::new();
    let state =
        AppState::in_memory_with_persistence(Some(dir.0.clone()), Arc::new(FsPersistenceSink));
    let mut supervisor = crate::service::ServiceSupervisor::new();
    supervisor.finish_startup();
    let mut writer = state.attach_service(supervisor.operations());
    let admitted = now();
    state.erase_personal_learning(G.into(), 7).await.unwrap();
    let source = |author| crate::engagement::SourceRef {
        scope: crate::engagement::EngagementScope::Guild {
            guild: 1,
            channel: 2,
        },
        message: author + 100,
        author,
        revision: 1,
        at: admitted,
    };
    assert_eq!(
        state.observe_engagement(source(7)).await,
        Err(WorkError::Stale)
    );
    assert!(
        state
            .engagement_events_healthy
            .load(std::sync::atomic::Ordering::SeqCst)
    );
    assert_eq!(state.observe_engagement(source(8)).await, Ok(true));
    let disk = Stores::load(&dir.0).unwrap();
    assert_eq!(disk.work.engagement.observations[&source(8).scope].len(), 1);
    assert!(disk.work.engagement.observations[&source(8).scope].contains_key(&8));
    writer.stop();
    writer.joined().await.unwrap();
}

#[test]
fn continuity_learning_erasure_fence_begins_after_gateway_phase() {
    let state = AppState::in_memory();
    state
        .finish_learning_erasure(G, Some(7), None, 100, 501)
        .unwrap();
    assert!(
        AppState::lock(&state.stores)
            .reward_recovery
            .erasure
            .blocks(G, U, 501)
    );
    assert!(
        AppState::lock(&state.stores)
            .reward_recovery
            .erasure
            .blocks(G, U, 800)
    );
    assert!(
        !AppState::lock(&state.stores)
            .reward_recovery
            .erasure
            .blocks(G, U, 802)
    );
}
