use super::*;
use crate::{
    engagement::CandidateState,
    llm::{ChatTurn, ModelTurn},
    provider::{ProviderId, TurnAdapter, TurnFuture},
};
use std::sync::Mutex;
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
            .push((system.to_owned(), turns.to_vec(), tools.len()));
        Box::pin(std::future::ready(Ok(ModelTurn {
            text: "How did the authorized test go?".into(),
            calls: Vec::new(),
        })))
    }
}
#[tokio::test]
async fn engagement_delivery_private_generation_uses_fresh_source_without_shared_history() {
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
    {
        let mut engine = AppState::lock(&state.engine);
        engine.commit(
            "discord:3",
            "DELETED_OTHER_MEMBER_SECRET_SENTINEL",
            "STALE_GENERATED_REPLY_SENTINEL",
            1,
        );
    }
    let candidate = Candidate {
        id: 1,
        kind: EngagementKind::FollowUp,
        source: Some(SourceRef {
            scope: EngagementScope::Guild {
                guild: 7,
                channel: 3,
            },
            message: 4,
            author: 2,
            revision: 1,
            at: 1,
        }),
        member: Some(2),
        scope: EngagementScope::Guild {
            guild: 7,
            channel: 3,
        },
        due_at: 2,
        revision: 1,
        state: CandidateState::Reserved,
        dedupe_key: "synthetic".into(),
        policy_revision: 1,
        destination: DestinationPreference::Private,
        message_id: None,
        introduction_id: None,
        work_ref: None,
        expires_at: None,
        follow_up_reason: None,
    };
    let adapter = DiscordEngagementDelivery(Arc::new(Http::new("synthetic-fixture")));
    adapter
        .generate(
            &state,
            &candidate,
            "FRESH_AUTHORIZED_SOURCE_SENTINEL: testing a compiler",
            2,
        )
        .await
        .unwrap();
    let seen = recorder.seen.lock().unwrap();
    assert_eq!(seen.len(), 1);
    let (system, turns, tools) = &seen[0];
    assert_eq!(*tools, 0);
    assert!(!system.contains("FRESH_AUTHORIZED_SOURCE_SENTINEL"));
    assert!(
        turns
            .iter()
            .any(|turn| turn.text.contains("FRESH_AUTHORIZED_SOURCE_SENTINEL"))
    );
    let prompt = format!("{system} {turns:?}");
    assert!(!prompt.contains("DELETED_OTHER_MEMBER_SECRET_SENTINEL"));
    assert!(!prompt.contains("STALE_GENERATED_REPLY_SENTINEL"));
    assert_eq!(turns.len(), 1);
    assert_eq!(AppState::lock(&state.engine).session_len("discord:3"), 2);
}
#[test]
fn engagement_delivery_public_news_threads_follow_parent_access_without_membership() {
    let recipient = Permissions::VIEW_CHANNEL | Permissions::READ_MESSAGE_HISTORY;
    let bot = recipient | Permissions::SEND_MESSAGES_IN_THREADS;
    for kind in [ChannelType::PublicThread, ChannelType::NewsThread] {
        assert!(thread_authorized(
            kind,
            false,
            false,
            recipient,
            bot,
            [false, false]
        ));
        assert!(!thread_authorized(
            kind,
            false,
            false,
            Permissions::empty(),
            bot,
            [true, true]
        ));
        assert!(!thread_authorized(
            kind,
            false,
            false,
            recipient,
            recipient,
            [true, true]
        ));
    }
}
#[test]
fn engagement_delivery_private_thread_membership_loss_and_inactive_state_fail_closed() {
    let recipient = Permissions::VIEW_CHANNEL | Permissions::READ_MESSAGE_HISTORY;
    let bot = recipient | Permissions::SEND_MESSAGES_IN_THREADS;
    assert!(thread_authorized(
        ChannelType::PrivateThread,
        false,
        false,
        recipient,
        bot,
        [true, true]
    ));
    for membership in [[false, true], [true, false], [false, false]] {
        assert!(!thread_authorized(
            ChannelType::PrivateThread,
            false,
            false,
            recipient,
            bot,
            membership
        ));
    }
    for kind in [
        ChannelType::PublicThread,
        ChannelType::NewsThread,
        ChannelType::PrivateThread,
    ] {
        assert!(!thread_authorized(
            kind,
            true,
            false,
            recipient,
            bot,
            [true, true]
        ));
        assert!(!thread_authorized(
            kind,
            false,
            true,
            recipient,
            bot,
            [true, true]
        ));
    }
}
#[test]
fn engagement_delivery_fresh_after_source_proof_blocks_restart_reply_and_full_page() {
    let source = SourceRef {
        scope: EngagementScope::Dm {
            member: 2,
            channel: 3,
        },
        author: 2,
        message: 4,
        revision: 1,
        at: 1,
    };
    assert!(exchange_current(&source, &[(5, 99, true), (6, 7, false)]));
    assert!(!exchange_current(&source, &[(5, 2, false)]));
    assert!(!exchange_current(&source, &vec![(5, 99, true); 100]));
    assert!(!exchange_current(&source, &[(4, 99, true)]));
}
#[path = "tests/introductions.rs"]
mod introductions;

#[path = "tests/identity.rs"]
mod identity;
