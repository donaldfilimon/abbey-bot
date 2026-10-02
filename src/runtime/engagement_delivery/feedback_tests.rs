//! Real retained-admission races around the existing bounded reduction ledger.
use super::*;
pub(super) fn guild_candidate(h: &Harness) {
    let mut stores = AppState::lock(&h.state.stores);
    AppState::lock(&h.state.guilds).update("discord:7", &mut *stores, |s| {
        s.unsolicited = true;
        s.learning_enabled = true;
        s.reply_cooldown_seconds = 0;
    });
    let store = &mut stores.work.engagement;
    let mut source = store.candidates[&1].source.clone().unwrap();
    source.scope = EngagementScope::Guild {
        guild: 7,
        channel: 3,
    };
    store.eligibility.insert(2, [source.clone()].into());
    store.candidates.clear();
    store.sequence = 0;
    store
        .propose(
            CandidateProposal {
                kind: EngagementKind::FollowUp,
                source: Some(source.clone()),
                member: Some(2),
                scope: source.scope,
                due_at: NOW,
                introduction_id: None,
            },
            NOW,
        )
        .unwrap();
}
fn reduce(state: &AppState) {
    for user in [
        "discord:2",
        "discord:2",
        "discord:5",
        "discord:5",
        "discord:6",
    ] {
        state.observe_style("discord:7", user, "fewer follow-ups", NOW);
    }
    state.tick_addenda(NOW);
}
struct LateReduction(Fake);
impl EngagementTransport for LateReduction {
    async fn authorize(
        &self,
        r: &EngagementReservation,
    ) -> Result<AuthorizedDestination, WorkError> {
        self.0.authorize(r).await
    }
    async fn source_exists(&self, s: &SourceRef) -> Result<bool, WorkError> {
        self.0.source_exists(s).await
    }
    async fn hydrate(&self, c: &Candidate) -> Result<String, WorkError> {
        self.0.hydrate(c).await
    }
    async fn generate(
        &self,
        state: &AppState,
        c: &Candidate,
        source: &str,
        at: u64,
    ) -> Result<String, WorkError> {
        reduce(state);
        self.0.generate(state, c, source, at).await
    }
    async fn send(&self, channel: u64, body: &str) -> Result<u64, SendFailure> {
        self.0.send(channel, body).await
    }
}
#[tokio::test]
async fn engagement_reduction_before_reservation_does_not_charge_or_send() {
    let h = Harness::new();
    guild_candidate(&h);
    reduce(&h.state);
    let fake = Fake::new(&h, Race::None);
    h.state
        .clone()
        .deliver_engagement(&fake, fake.cancel.clone(), || NOW)
        .await
        .unwrap();
    assert_eq!(fake.sends.load(Ordering::SeqCst), 0);
    assert_eq!(h.status(), CandidateState::Pending);
    assert!(
        AppState::lock(&h.state.stores)
            .work
            .engagement
            .charges
            .is_empty()
    );
    h.finish().await;
}
#[tokio::test]
async fn engagement_reduction_after_generation_prevents_final_send_without_refund() {
    let h = Harness::new();
    guild_candidate(&h);
    let fake = LateReduction(Fake::new(&h, Race::None));
    h.state
        .clone()
        .deliver_engagement(&fake, fake.0.cancel.clone(), || NOW)
        .await
        .unwrap();
    assert_eq!(fake.0.sends.load(Ordering::SeqCst), 0);
    assert_eq!(h.status(), CandidateState::Rejected);
    assert_eq!(
        AppState::lock(&h.state.stores)
            .work
            .engagement
            .charges
            .len(),
        1
    );
    h.finish().await;
}
#[tokio::test]
async fn engagement_expired_reduction_never_removes_member_stop() {
    let h = Harness::new();
    guild_candidate(&h);
    reduce(&h.state);
    AppState::lock(&h.state.stores)
        .work
        .engagement
        .member_policies
        .get_mut(&2)
        .unwrap()
        .global_stop = true;
    let fake = Fake::new(&h, Race::None);
    h.state
        .clone()
        .deliver_engagement(&fake, fake.cancel.clone(), || {
            NOW + crate::brain::addenda::Policy::default().ttl_secs
        })
        .await
        .unwrap();
    assert_eq!(fake.sends.load(Ordering::SeqCst), 0);
    assert_eq!(h.status(), CandidateState::Pending);
    assert!(
        AppState::lock(&h.state.stores)
            .work
            .engagement
            .member_policies[&2]
            .global_stop
    );
    h.finish().await;
}
