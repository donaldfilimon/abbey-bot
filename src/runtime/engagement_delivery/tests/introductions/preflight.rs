use super::*;

#[derive(Clone, Copy, Debug)]
enum Failure {
    Denied,
    Invalid,
    Stale,
    Mismatch,
    Missing,
    Persistence,
    Full,
    Cancelled,
    Timeout,
}
impl Failure {
    fn terminal(self) -> bool {
        matches!(
            self,
            Self::Denied | Self::Invalid | Self::Stale | Self::Mismatch
        )
    }
}
struct PreflightTransport {
    inner: IntroductionFake,
    failure: Failure,
    calls: AtomicUsize,
    cancel: CancellationToken,
    entered: tokio::sync::Notify,
}
impl EngagementTransport for PreflightTransport {
    async fn authorize(
        &self,
        reservation: &EngagementReservation,
    ) -> Result<AuthorizedDestination, WorkError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        match self.failure {
            Failure::Denied => Err(WorkError::Denied),
            Failure::Invalid => Err(WorkError::Invalid),
            Failure::Stale => Err(WorkError::Stale),
            Failure::Missing => Err(WorkError::Missing),
            Failure::Persistence => Err(WorkError::Persistence),
            Failure::Full => Err(WorkError::Full),
            Failure::Mismatch => {
                let mut destination = self.inner.authorize(reservation).await?;
                destination.channel += 1;
                Ok(destination)
            }
            Failure::Cancelled => {
                self.cancel.cancel();
                std::future::pending().await
            }
            Failure::Timeout => {
                self.entered.notify_one();
                std::future::pending().await
            }
        }
    }
    async fn source_exists(&self, source: &SourceRef) -> Result<bool, WorkError> {
        self.inner.source_exists(source).await
    }
    async fn hydrate(&self, candidate: &Candidate) -> Result<String, WorkError> {
        self.inner.hydrate(candidate).await
    }
    async fn generate(
        &self,
        state: &AppState,
        candidate: &Candidate,
        text: &str,
        now: u64,
    ) -> Result<String, WorkError> {
        self.inner.generate(state, candidate, text, now).await
    }
    async fn send(&self, channel: u64, body: &str) -> Result<u64, SendFailure> {
        self.inner.send(channel, body).await
    }
}

#[tokio::test(start_paused = true)]
async fn engagement_preflight_introduction_terminal_denial_vs_deferred_proof_retains_approvals() {
    for failure in [
        Failure::Denied,
        Failure::Invalid,
        Failure::Stale,
        Failure::Mismatch,
        Failure::Missing,
        Failure::Persistence,
        Failure::Full,
        Failure::Cancelled,
        Failure::Timeout,
    ] {
        let h = Harness::new();
        setup(&h, true);
        let transport = PreflightTransport {
            inner: IntroductionFake {
                state: h.state.clone(),
                dir: h.dir.clone(),
                auth: AtomicUsize::new(0),
                sends: AtomicUsize::new(0),
                change: Change::None,
            },
            failure,
            calls: AtomicUsize::new(0),
            cancel: CancellationToken::new(),
            entered: tokio::sync::Notify::new(),
        };
        let delivery =
            h.state
                .clone()
                .deliver_engagement(&transport, transport.cancel.clone(), || NOW);
        if matches!(failure, Failure::Timeout) {
            let deadline = async {
                transport.entered.notified().await;
                tokio::time::advance(Duration::from_secs(31)).await;
            };
            let (delivered, ()) = tokio::join!(delivery, deadline);
            delivered.unwrap();
        } else {
            delivery.await.unwrap();
        }
        assert_eq!(transport.calls.load(Ordering::SeqCst), 1, "{failure:?}");
        assert_eq!(
            transport.inner.sends.load(Ordering::SeqCst),
            0,
            "{failure:?}"
        );
        {
            let stores = AppState::lock(&h.state.stores);
            let store = &stores.work.engagement;
            assert!(store.charges.is_empty(), "{failure:?}");
            let introduction = &store.introductions[&1];
            if failure.terminal() {
                assert_eq!(
                    store.candidates[&2].state,
                    CandidateState::Rejected,
                    "{failure:?}"
                );
                assert_eq!(
                    introduction.state,
                    IntroductionState::Cancelled,
                    "{failure:?}"
                );
                assert_eq!(introduction.approvals, [None; 2], "{failure:?}");
            } else {
                assert_eq!(
                    store.candidates[&2].state,
                    CandidateState::Pending,
                    "{failure:?}"
                );
                assert_eq!(introduction.state, IntroductionState::Ready, "{failure:?}");
                assert_eq!(introduction.approvals, [Some(2); 2], "{failure:?}");
            }
        }
        let healthy = IntroductionFake {
            state: h.state.clone(),
            dir: h.dir.clone(),
            auth: AtomicUsize::new(0),
            sends: AtomicUsize::new(0),
            change: Change::None,
        };
        h.state
            .clone()
            .deliver_engagement(&healthy, CancellationToken::new(), || NOW)
            .await
            .unwrap();
        let expected = usize::from(!failure.terminal());
        assert_eq!(
            healthy.sends.load(Ordering::SeqCst),
            expected,
            "{failure:?}"
        );
        assert_eq!(
            healthy.auth.load(Ordering::SeqCst),
            expected * 2,
            "{failure:?}"
        );
        let persisted = Stores::load(&h.dir).unwrap();
        assert_eq!(
            persisted.work.engagement.charges.len(),
            expected * 2,
            "{failure:?}"
        );
        h.finish().await;
    }
}
