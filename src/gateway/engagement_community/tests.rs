use super::*;
use serenity::all::{PermissionOverwrite, RoleId};
#[test]
fn community_work_audience_requires_closed_upper_bound() {
    let audience = BTreeSet::from([2, 3]);
    let deny = PermissionOverwrite {
        kind: PermissionOverwriteType::Role(RoleId::new(7)),
        allow: Permissions::empty(),
        deny: Permissions::VIEW_CHANNEL,
    };
    assert!(closed_overwrites(
        7,
        2,
        8,
        &audience,
        std::slice::from_ref(&deny)
    ));
    assert!(!closed_overwrites(7, 2, 8, &audience, &[]));
    assert!(!closed_overwrites(
        7,
        4,
        8,
        &audience,
        std::slice::from_ref(&deny)
    ));
    for kind in [
        PermissionOverwriteType::Role(RoleId::new(6)),
        PermissionOverwriteType::Member(UserId::new(4)),
    ] {
        assert!(!closed_overwrites(
            7,
            2,
            8,
            &audience,
            &[
                deny.clone(),
                PermissionOverwrite {
                    kind,
                    allow: Permissions::VIEW_CHANNEL,
                    deny: Permissions::empty()
                }
            ]
        ));
    }
}
#[test]
fn community_project_scope_audience_and_content_revision_are_fresh() {
    let mut w = crate::work::WorkStore::default();
    let scope = EngagementScope::Guild {
        guild: 7,
        channel: 9,
    };
    w.projects.insert(
        5,
        crate::work::WorkProject {
            id: 5,
            name: "Synthetic project".into(),
            scope: WorkScope::Team {
                guild: 7,
                channel: 9,
            },
            managers: BTreeSet::from([2]),
            members: BTreeSet::from([2]),
            revision: 1,
            allowed_github_repositories: BTreeSet::new(),
        },
    );
    w.tasks.insert(
        6,
        crate::work::WorkTask {
            id: 6,
            project_id: 5,
            title: "Verify experiment".into(),
            owner: 2,
            assignee: None,
            goal_id: None,
            priority: 1,
            status: crate::work::WorkStatus::Open,
            due_at: None,
            remind_at: None,
            reminder_revision: 0,
            snoozed_until: None,
            source: None,
            github: None,
            revision: 1,
        },
    );
    let evidence = CommunityEvidence::Project {
        project: 5,
        revision: 1,
        actor: 2,
        audience: BTreeSet::from([2]),
        content: BTreeSet::from([WorkContentRef::Task {
            project: 5,
            id: 6,
            revision: 1,
        }]),
    };
    assert!(
        project_text(&w, &scope, &evidence)
            .unwrap()
            .contains("Verify experiment")
    );
    for change in 0..4 {
        let mut changed = w.clone();
        match change {
            0 => {
                changed.projects.get_mut(&5).unwrap().members.insert(3);
            }
            1 => {
                changed.projects.get_mut(&5).unwrap().scope = WorkScope::Team {
                    guild: 8,
                    channel: 9,
                }
            }
            2 => changed.tasks.get_mut(&6).unwrap().revision += 1,
            _ => changed.projects.get_mut(&5).unwrap().managers.clear(),
        }
        assert!(project_text(&changed, &scope, &evidence).is_err());
    }
}
use crate::{
    llm::{ChatTurn, ModelTurn},
    provider::{ProviderId, TurnAdapter, TurnFuture},
};
use std::sync::{Arc, Mutex};
struct RecordingProvider {
    id: ProviderId,
    seen: Mutex<Vec<(String, Vec<ChatTurn>, usize)>>,
}
impl TurnAdapter for RecordingProvider {
    fn provider_id(&self) -> &ProviderId {
        &self.id
    }
    fn turn<'a>(
        &'a self,
        system: &'a str,
        turns: &'a [ChatTurn],
        tools: &'a [crate::tools::ToolSpec],
        _: &'a str,
    ) -> TurnFuture<'a> {
        self.seen
            .lock()
            .unwrap()
            .push((system.into(), turns.to_vec(), tools.len()));
        let text = if system == PUBLIC_PROMPT {
            let input: serde_json::Value = serde_json::from_str(&turns[0].text).unwrap();
            let body = input[0]["text"].as_str().unwrap();
            let outcome = if body.starts_with("Please explain") {
                "unanswered_question"
            } else {
                "useful_context"
            };
            let question = if outcome == "unanswered_question" {
                serde_json::json!({"source_message":input[0]["message"],"excerpt":body})
            } else {
                serde_json::Value::Null
            };
            serde_json::json!({"outcome":outcome,"source_messages":[input[0]["message"]],"question":question}).to_string()
        } else {
            "Which constraint should we test next?".into()
        };
        Box::pin(std::future::ready(Ok(ModelTurn {
            text,
            calls: Vec::new(),
        })))
    }
}
#[tokio::test]
async fn community_public_generation_is_source_only_without_tools_or_shared_history() {
    for kind in [
        EngagementKind::ConversationStarter,
        EngagementKind::UnansweredQuestion,
        EngagementKind::ProjectCheckIn,
    ] {
        let mut state = AppState::in_memory();
        let recorder = Arc::new(RecordingProvider {
            id: ProviderId::parse("primary").unwrap(),
            seen: Mutex::default(),
        });
        let mut providers = crate::provider::ProviderRuntime::empty();
        providers.register_test_adapter_with_locality(
            recorder.clone(),
            crate::provider::ExecutionLocality::SameHost,
        );
        Arc::get_mut(&mut state).unwrap().providers = providers;
        AppState::lock(&state.engine).commit(
            "discord:9",
            "OTHER_MEMBER_HISTORY_SENTINEL",
            "OLD_RESPONSE_SENTINEL",
            1,
        );
        let candidate = Candidate {
            id: 1,
            kind,
            source: None,
            member: None,
            scope: EngagementScope::Guild {
                guild: 7,
                channel: 9,
            },
            due_at: 2,
            revision: 1,
            state: crate::engagement::CandidateState::Reserved,
            dedupe_key: "synthetic".into(),
            policy_revision: 1,
            destination: DestinationPreference::Origin,
            message_id: None,
            introduction_id: None,
            work_ref: None,
            expires_at: None,
            follow_up_reason: None,
        };
        DiscordEngagementDelivery(Arc::new(serenity::all::Http::new("synthetic-fixture")))
            .generate(
                &state,
                &candidate,
                "CURRENT_AUTHORIZED_SOURCE_SENTINEL: constrained compiler experiment",
                2,
            )
            .await
            .unwrap();
        let seen = recorder.seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        let (system, turns, tools) = &seen[0];
        assert_eq!(*tools, 0);
        assert!(!system.contains("CURRENT_AUTHORIZED_SOURCE_SENTINEL"));
        assert!(
            turns
                .iter()
                .any(|turn| turn.text.contains("CURRENT_AUTHORIZED_SOURCE_SENTINEL"))
        );
        let prompt = format!("{system} {turns:?}");
        assert!(!prompt.contains("OTHER_MEMBER_HISTORY_SENTINEL"));
        assert!(!prompt.contains("OLD_RESPONSE_SENTINEL"));
        assert_eq!(turns.len(), 1);
        assert_eq!(AppState::lock(&state.engine).session_len("discord:9"), 2);
    }
}
#[test]
fn community_answered_forum_and_full_unknown_page_fail_closed() {
    assert!(public_exchange_current(10, &[(11, true)]));
    assert!(!public_exchange_current(10, &[(11, false)]));
    assert!(!public_exchange_current(10, &[(9, true)]));
    assert!(!public_exchange_current(
        10,
        &(11..111).map(|id| (id, true)).collect::<Vec<_>>()
    ));
}
const FAIR_NOW: u64 = 1_790_683_200;
fn fair_store() -> crate::engagement::EngagementStore {
    let mut s = crate::engagement::EngagementStore::default();
    s.guild_features.insert(
        7,
        crate::engagement::GuildFeaturePolicy {
            revision: 1,
            enabled: BTreeSet::from([crate::engagement::CommunityFeature::Questions]),
            channels: std::collections::BTreeMap::from([(
                crate::engagement::CommunityFeature::Questions,
                (1..=9).collect(),
            )]),
        },
    );
    for channel in 1..=9 {
        let scope = EngagementScope::Guild { guild: 7, channel };
        s.observations.entry(scope.clone()).or_default().insert(
            2,
            SourceRef {
                scope,
                message: 100 + channel,
                author: 2,
                revision: 1,
                at: FAIR_NOW - 86_400 - channel,
            },
        );
    }
    s
}
#[test]
fn community_fair_selection_skips_eight_consumed_before_work_limit() {
    let mut s = fair_store();
    let sources = public_sources(&s, FAIR_NOW);
    for source in sources {
        let facts = CommunityFacts {
            rows: vec![CommunityFact {
                kind: EngagementKind::UnansweredQuestion,
                scope: source.scope.clone(),
                source: Some(source.clone()),
                evidence: CommunityEvidence::Message,
                at: source.at,
                useful: true,
                current: true,
            }],
            join_events_available: false,
        };
        s.propose_community(&facts, FAIR_NOW).unwrap();
        let id = s.sequence;
        s.reserve(id, 1, FAIR_NOW).unwrap();
        s.settle(
            id,
            crate::engagement::lifecycle::DeliveryOutcome::Sent {
                message_id: 1000 + id,
            },
        )
        .unwrap();
    }
    let selected = public_sources(&s, FAIR_NOW);
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].message, 109);
}
#[test]
fn community_questions_only_refuses_personal_unresolved_project_contract() {
    let s = fair_store();
    let source = s.observations.values().next().unwrap()[&2].clone();
    let raw =
        serde_json::json!({"outcome":"unresolved","source_messages":[source.message]}).to_string();
    assert_eq!(
        assessed_kind(
            &s,
            &source,
            &raw,
            "I am working on a compiler project; the lowering pass remains in progress."
        ),
        None
    );
}
#[test]
fn community_public_semantics_keep_question_and_starter_switches_independent() {
    let mut s = fair_store();
    let source = s.observations.values().next().unwrap()[&2].clone();
    let text = "Please explain how this compiler lowers an async suspension into state transitions";
    let question=serde_json::json!({"outcome":"unanswered_question","source_messages":[source.message],"question":{"source_message":source.message,"excerpt":text}}).to_string();
    let project=serde_json::json!({"outcome":"useful_context","source_messages":[source.message],"question":null}).to_string();
    assert_eq!(
        assessed_kind(&s, &source, &question, text),
        Some(EngagementKind::UnansweredQuestion)
    );
    assert_eq!(
        assessed_kind(&s, &source, &project, "I am working on a compiler project."),
        None
    );
    let policy = s.guild_features.get_mut(&7).unwrap();
    policy.enabled = BTreeSet::from([crate::engagement::CommunityFeature::Starters]);
    policy.channels.insert(
        crate::engagement::CommunityFeature::Starters,
        (1..=9).collect(),
    );
    assert_eq!(
        assessed_kind(&s, &source, &project, "I am working on a compiler project."),
        Some(EngagementKind::ConversationStarter)
    );
    s.guild_features.get_mut(&7).unwrap().enabled.clear();
    assert_eq!(assessed_kind(&s, &source, &question, text), None);
}
use crate::{
    persist::Stores,
    service::{ServiceSupervisor, persistence::PersistenceWriter},
};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};
struct FairHarness {
    state: Arc<AppState>,
    dir: PathBuf,
    writer: PersistenceWriter,
    _supervisor: ServiceSupervisor,
}
impl FairHarness {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "abbey-public-assessment-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let mut state = AppState::in_memory();
        Arc::get_mut(&mut state).unwrap().data_dir = Some(dir.clone());
        {
            let mut stores = AppState::lock(&state.stores);
            stores.work.engagement = fair_store();
            AppState::lock(&state.guilds).update("discord:7", &mut *stores, |s| {
                s.unsolicited = true;
                s.learning_enabled = true;
            });
        }
        let mut supervisor = ServiceSupervisor::new();
        supervisor.finish_startup();
        let writer = state.attach_service(supervisor.operations());
        Self {
            state,
            dir,
            writer,
            _supervisor: supervisor,
        }
    }
    async fn finish(mut self) {
        self.writer.stop();
        self.writer.joined().await.unwrap();
        std::fs::remove_dir_all(self.dir).unwrap();
    }
}
struct FairTransport {
    failure: WorkError,
    model_calls: AtomicUsize,
    hang: bool,
    entered: tokio::sync::Notify,
}
impl FairTransport {
    fn new(failure: WorkError) -> Self {
        Self {
            failure,
            model_calls: AtomicUsize::new(0),
            hang: false,
            entered: tokio::sync::Notify::new(),
        }
    }
}
impl PublicSourceTransport for FairTransport {
    async fn context(&self, source: &SourceRef) -> Result<String, WorkError> {
        if self.hang {
            self.entered.notify_one();
            return std::future::pending().await;
        }
        if source.message <= 108 {
            return Err(self.failure);
        }
        Ok(
            "Please explain how this compiler lowers an async suspension into state transitions"
                .into(),
        )
    }
    async fn assessment(
        &self,
        _: &AppState,
        source: &SourceRef,
        text: &str,
    ) -> Result<String, WorkError> {
        self.model_calls.fetch_add(1, Ordering::SeqCst);
        Ok(serde_json::json!({"outcome":"unanswered_question","source_messages":[source.message],"question":{"source_message":source.message,"excerpt":text}}).to_string())
    }
}
#[tokio::test]
async fn community_production_inventory_advances_past_failed_eight_and_never_reclassifies_consumed()
{
    for failure in [WorkError::Denied, WorkError::Missing] {
        let h = FairHarness::new();
        let t = FairTransport::new(failure);
        assert!(
            public_source_facts(&h.state, &t, FAIR_NOW)
                .await
                .unwrap()
                .is_empty()
        );
        assert_eq!(t.model_calls.load(Ordering::SeqCst), 0);
        let rows = public_source_facts(&h.state, &t, FAIR_NOW).await.unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].source.as_ref().unwrap().message, 109);
        assert_eq!(t.model_calls.load(Ordering::SeqCst), 1);
        h.state
            .commit_engagement(move |store| {
                store.propose_community(
                    &CommunityFacts {
                        rows,
                        join_events_available: false,
                    },
                    FAIR_NOW,
                )?;
                let id = store.sequence;
                store.reserve(id, 1, FAIR_NOW)?;
                store.settle(
                    id,
                    crate::engagement::lifecycle::DeliveryOutcome::ReviewRequired,
                )
            })
            .await
            .unwrap();
        public_source_facts(&h.state, &t, FAIR_NOW).await.unwrap();
        assert_eq!(t.model_calls.load(Ordering::SeqCst), 1);
        h.finish().await;
    }
}
#[tokio::test]
async fn community_cursor_survives_dropped_rest_waiter_restart_wrap_and_new_source() {
    let h = FairHarness::new();
    let mut transport = FairTransport::new(WorkError::Denied);
    transport.hang = true;
    let t = Arc::new(transport);
    let state = h.state.clone();
    let worker_t = t.clone();
    let worker =
        tokio::spawn(async move { public_source_facts(&state, worker_t.as_ref(), FAIR_NOW).await });
    tokio::time::timeout(std::time::Duration::from_secs(5), t.entered.notified())
        .await
        .unwrap();
    worker.abort();
    assert!(worker.await.unwrap_err().is_cancelled());
    let mut loaded = Stores::load(&h.dir).unwrap().work.engagement;
    assert_eq!(loaded.community_cursor.as_ref().unwrap().message, 101);
    assert_eq!(public_sources(&loaded, FAIR_NOW)[0].message, 102);
    loaded.community_cursor = Some(loaded.observations.values().last().unwrap()[&2].clone());
    assert_eq!(public_sources(&loaded, FAIR_NOW)[0].message, 101);
    loaded
        .observations
        .get_mut(&EngagementScope::Guild {
            guild: 7,
            channel: 1,
        })
        .unwrap()
        .get_mut(&2)
        .unwrap()
        .message = 1001;
    assert_eq!(public_sources(&loaded, FAIR_NOW)[0].message, 1001);
    loaded.community_cursor.as_mut().unwrap().scope = EngagementScope::Dm {
        member: 2,
        channel: 9,
    };
    assert!(
        serde_json::from_value::<crate::engagement::EngagementStore>(
            serde_json::to_value(&loaded).unwrap()
        )
        .is_err()
    );
    h.finish().await;
}

#[tokio::test]
async fn community_actual_adapter_uses_public_semantic_contract_without_tools_or_history() {
    let mut state = AppState::in_memory();
    let recorder = Arc::new(RecordingProvider {
        id: ProviderId::parse("primary").unwrap(),
        seen: Mutex::default(),
    });
    let mut providers = crate::provider::ProviderRuntime::empty();
    providers.register_test_adapter_with_locality(
        recorder.clone(),
        crate::provider::ExecutionLocality::SameHost,
    );
    Arc::get_mut(&mut state).unwrap().providers = providers;
    AppState::lock(&state.engine).commit("discord:1", "OTHER_SCOPE_SECRET", "OLD_QUESTION", 1);
    let store = fair_store();
    let source = store.observations.values().next().unwrap()[&2].clone();
    let adapter =
        DiscordEngagementDelivery(Arc::new(serenity::all::Http::new("synthetic-fixture")));
    for text in [
        "I am working on a compiler project; the lowering pass remains in progress.",
        "Please explain how this compiler lowers an async suspension into state transitions",
    ] {
        let raw = adapter.assessment(&state, &source, text).await.unwrap();
        let assessment = parse_public_assessment(&raw, &source, text).unwrap();
        if assessment.outcome == PublicOutcome::UsefulContext {
            assert_eq!(assessed_kind(&store, &source, &raw, text), None);
        } else {
            assert_eq!(
                assessed_kind(&store, &source, &raw, text),
                Some(EngagementKind::UnansweredQuestion)
            );
        }
    }
    let seen = recorder.seen.lock().unwrap();
    assert_eq!(seen.len(), 2);
    for (system, turns, tools) in seen.iter() {
        assert_eq!(system, PUBLIC_PROMPT);
        assert_eq!(*tools, 0);
        assert_eq!(turns.len(), 1);
        assert!(!turns[0].text.contains("OTHER_SCOPE_SECRET"));
    }
    assert_eq!(AppState::lock(&state.engine).session_len("discord:1"), 2);
}

struct PoisonTransport {
    poison: u64,
    calls: AtomicUsize,
    entered: tokio::sync::Notify,
}
impl PublicSourceTransport for PoisonTransport {
    async fn context(&self, source: &SourceRef) -> Result<String, WorkError> {
        if source.message == self.poison {
            self.entered.notify_one();
            return std::future::pending().await;
        }
        Ok(
            "Please explain how this compiler lowers an async suspension into state transitions"
                .into(),
        )
    }
    async fn assessment(
        &self,
        _: &AppState,
        source: &SourceRef,
        text: &str,
    ) -> Result<String, WorkError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(serde_json::json!({"outcome":"unanswered_question","source_messages":[source.message],"question":{"source_message":source.message,"excerpt":text}}).to_string())
    }
}
impl EngagementTransport for PoisonTransport {
    async fn community_facts(
        &self,
        state: &AppState,
        now: u64,
        completed: &mut CommunityFacts,
    ) -> Result<(), WorkError> {
        collect_public_source_facts(state, self, now, &mut completed.rows).await?;
        if self.poison == 0 {
            // Model a later Work project audience proof that never completes.
            self.entered.notify_one();
            return std::future::pending().await;
        }
        Ok(())
    }
    async fn authorize(
        &self,
        _: &EngagementReservation,
    ) -> Result<crate::runtime::engagement_delivery::AuthorizedDestination, WorkError> {
        Err(WorkError::Denied)
    }
    async fn source_exists(&self, _: &SourceRef) -> Result<bool, WorkError> {
        Err(WorkError::Denied)
    }
    async fn hydrate(&self, _: &Candidate) -> Result<String, WorkError> {
        Err(WorkError::Denied)
    }
    async fn generate(
        &self,
        _: &AppState,
        _: &Candidate,
        _: &str,
        _: u64,
    ) -> Result<String, WorkError> {
        Err(WorkError::Denied)
    }
    async fn send(
        &self,
        _: u64,
        _: &str,
    ) -> Result<u64, crate::runtime::engagement_delivery::SendFailure> {
        Err(crate::runtime::engagement_delivery::SendFailure::Rejected)
    }
}
#[tokio::test]
async fn community_timed_production_planning_publishes_verified_fact_despite_poison_in_both_orders_restart()
 {
    for poison in [101, 102, 0] {
        let h = FairHarness::new();
        h.state
            .commit_engagement(|store| {
                store.observations.retain(|scope, _| {
                    matches!(scope, EngagementScope::Guild { channel: 1 | 2, .. })
                });
                Ok(())
            })
            .await
            .unwrap();
        let t = Arc::new(PoisonTransport {
            poison,
            calls: AtomicUsize::new(0),
            entered: tokio::sync::Notify::new(),
        });
        timed_poison_tick(h.state.clone(), t.clone()).await;
        assert_eq!(
            Stores::load(&h.dir)
                .unwrap()
                .work
                .engagement
                .candidates
                .len(),
            match poison {
                101 => 0,
                102 => 1,
                _ => 2,
            },
            "completed facts must publish in the first tick"
        );
        // Reload the canonical cursor after the first timed tick, preserving any
        // completed proposal. The second tick must publish the other source.
        let loaded = Stores::load(&h.dir).unwrap().work.engagement;
        h.state
            .commit_engagement(move |store| {
                *store = loaded;
                Ok(())
            })
            .await
            .unwrap();
        timed_poison_tick(h.state.clone(), t.clone()).await;
        timed_poison_tick(h.state.clone(), t.clone()).await;
        let saved = Stores::load(&h.dir).unwrap().work.engagement;
        assert_eq!(
            saved.candidates.len(),
            if poison == 0 { 2 } else { 1 },
            "poison source {poison} discarded completed work"
        );
        assert_eq!(
            saved
                .candidates
                .values()
                .next()
                .unwrap()
                .source
                .as_ref()
                .unwrap()
                .message,
            if poison == 101 { 102 } else { 101 }
        );
        assert_eq!(
            t.calls.load(Ordering::SeqCst),
            if poison == 0 { 2 } else { 1 },
            "consumed valid source was reclassified"
        );
        assert_eq!(
            saved.community_cursor.unwrap().message,
            if poison == 0 { 102 } else { poison }
        );
        h.finish().await;
    }
}

async fn timed_poison_tick(state: Arc<AppState>, transport: Arc<PoisonTransport>) {
    let worker_transport = transport.clone();
    let worker = tokio::spawn(async move {
        state
            .plan_community(worker_transport.as_ref(), FAIR_NOW)
            .await
    });
    // Keep canonical filesystem writes on real time until the pending proof is
    // reached. Then advance only the existing production timeout explicitly.
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        transport.entered.notified(),
    )
    .await
    .unwrap();
    tokio::time::pause();
    tokio::time::advance(std::time::Duration::from_secs(30)).await;
    tokio::time::resume();
    worker.await.unwrap().unwrap();
}
