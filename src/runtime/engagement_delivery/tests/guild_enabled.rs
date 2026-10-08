// Apply as src/runtime/engagement_delivery/tests/guild_enabled.rs; register
// #[path = "tests/guild_enabled.rs"] mod guild_enabled; in delivery/tests.rs.
// Uses the existing real Harness/Fake and the unchanged legacy conversation API.
use super::*;

struct CountGeneration {
    fake: Fake,
    generations: AtomicUsize,
}
impl EngagementTransport for CountGeneration {
    async fn authorize(
        &self,
        r: &EngagementReservation,
    ) -> Result<AuthorizedDestination, WorkError> {
        self.fake.authorize(r).await
    }
    async fn source_exists(&self, source: &SourceRef) -> Result<bool, WorkError> {
        self.fake.source_exists(source).await
    }
    async fn hydrate(&self, c: &Candidate) -> Result<String, WorkError> {
        self.fake.hydrate(c).await
    }
    async fn generate(
        &self,
        state: &AppState,
        candidate: &Candidate,
        source: &str,
        now: u64,
    ) -> Result<String, WorkError> {
        self.generations.fetch_add(1, Ordering::SeqCst);
        self.fake.generate(state, candidate, source, now).await
    }
    async fn send(&self, channel: u64, body: &str) -> Result<u64, SendFailure> {
        self.fake.send(channel, body).await
    }
}

fn guild_candidate(h: &Harness, enabled: bool) {
    let mut stores = AppState::lock(&h.state.stores);
    let scope = EngagementScope::Guild {
        guild: 7,
        channel: 3,
    };
    let e = &mut stores.work.engagement;
    let source = {
        let candidate = e.candidates.get_mut(&1).unwrap();
        candidate.scope = scope.clone();
        candidate.source.as_mut().unwrap().scope = scope;
        candidate.source.clone().unwrap()
    };
    e.eligibility.entry(2).or_default().insert(source);
    e.validate().unwrap();
    AppState::lock(&h.state.guilds).update("discord:7", &mut *stores, |settings| {
        settings.enabled = enabled;
        settings.unsolicited = true;
        settings.unsolicited_channels = None;
        settings.unsolicited_per_hour = 1;
        settings.reply_cooldown_seconds = 0;
        settings.learning_enabled = true;
    });
}

#[tokio::test]
async fn engagement_delivery_enabled_guild_control_uses_real_reservation_and_send() {
    let h = Harness::new();
    guild_candidate(&h, true);
    let transport = CountGeneration {
        fake: Fake::new(&h, Race::None),
        generations: AtomicUsize::new(0),
    };
    h.state
        .clone()
        .deliver_engagement(&transport, transport.fake.cancel.clone(), || NOW)
        .await
        .unwrap();
    assert_eq!(transport.generations.load(Ordering::SeqCst), 1);
    assert_eq!(transport.fake.sends.load(Ordering::SeqCst), 1);
    assert_eq!(h.status(), CandidateState::Sent);
    assert_eq!(
        Stores::load(&h.dir).unwrap().work.engagement.charges.len(),
        1
    );
    h.finish().await;
}

#[tokio::test]
async fn engagement_delivery_disabled_guild_does_not_generate_charge_or_send_when_unsolicited_is_on()
 {
    let h = Harness::new();
    guild_candidate(&h, false);
    let transport = CountGeneration {
        fake: Fake::new(&h, Race::None),
        generations: AtomicUsize::new(0),
    };
    h.state
        .clone()
        .deliver_engagement(&transport, transport.fake.cancel.clone(), || NOW)
        .await
        .unwrap();
    assert_eq!(
        transport.generations.load(Ordering::SeqCst),
        0,
        "guild enabled=false must block before a generation attempt"
    );
    assert_eq!(transport.fake.sends.load(Ordering::SeqCst), 0);
    assert_ne!(h.status(), CandidateState::Reserved);
    assert!(
        AppState::lock(&h.state.stores)
            .work
            .engagement
            .charges
            .is_empty()
    );
    assert!(
        Stores::load(&h.dir)
            .unwrap()
            .work
            .engagement
            .charges
            .is_empty()
    );
    assert_eq!(
        AppState::lock(&h.state.budget).tokens_left("discord:7", 1, NOW),
        1.0
    );
    assert!(AppState::lock(&h.state.cooldown).permitted("discord:3", 20, NOW));
    h.finish().await;
}
