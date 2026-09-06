use super::*;
use crate::observability::{EventCode, EventComponent, EventOutcome};
use std::sync::atomic::{AtomicBool, Ordering};

struct ControlledSink {
    entered: Mutex<Option<oneshot::Sender<()>>>,
    release: Mutex<mpsc::Receiver<()>>,
    order: Arc<Mutex<Vec<&'static str>>>,
    fail: bool,
}
impl Sink for ControlledSink {
    fn event(&self, _: &OperationalEvent) -> Result<(), ManagedFailure> {
        if let Some(entered) = self.entered.lock().unwrap().take() {
            let _ = entered.send(());
            self.release
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(5))
                .unwrap();
        }
        self.order.lock().unwrap().push("event");
        if self.fail {
            Err(ManagedFailure::Sync)
        } else {
            Ok(())
        }
    }
    fn readiness(&self, _: &ReadinessDocument) -> Result<(), ManagedFailure> {
        self.order.lock().unwrap().push("readiness");
        Ok(())
    }
    fn remove(&self) -> Result<(), ManagedFailure> {
        self.order.lock().unwrap().push("remove");
        Ok(())
    }
}
fn event() -> OperationalEvent {
    OperationalEvent::new(
        42,
        EventComponent::Voice,
        EventCode::VoiceState,
        EventOutcome::Stopped,
    )
    .unwrap()
}
type Fixture = (
    TelemetryWriter,
    oneshot::Receiver<()>,
    mpsc::Sender<()>,
    Arc<Mutex<Vec<&'static str>>>,
    Arc<AtomicBool>,
);
fn fixture(fail: bool) -> Fixture {
    let (entered, started) = oneshot::channel();
    let (release, wait) = mpsc::channel();
    let order = Arc::new(Mutex::new(Vec::new()));
    let fatal = Arc::new(AtomicBool::new(false));
    let failed = fatal.clone();
    let writer = TelemetryWriter::start_sink(
        ControlledSink {
            entered: Mutex::new(Some(entered)),
            release: Mutex::new(wait),
            order: order.clone(),
            fail,
        },
        Arc::new(move || {
            failed.store(true, Ordering::SeqCst);
        }),
    );
    (writer, started, release, order, fatal)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dropped_receipt_and_expired_wait_do_not_release_the_actual_writer() {
    let (mut writer, entered, release, order, fatal) = fixture(false);
    let requests = writer.requests();
    requests.event(event()).unwrap();
    entered.await.unwrap();
    let document = ReadinessDocument::decode(include_bytes!(
        "../../../tests/fixtures/service-protocol/readiness-v1.json"
    ))
    .unwrap();
    drop(requests.readiness(document).unwrap());
    let removed = requests.remove_final().unwrap();
    writer.stop();
    assert!(!writer.idle());
    assert!(
        tokio::time::timeout(Duration::from_millis(1), writer.joined())
            .await
            .is_err()
    );
    assert!(!writer.finished());
    release.send(()).unwrap();
    removed.await.unwrap().unwrap();
    writer.joined().await.unwrap();
    assert_eq!(*order.lock().unwrap(), ["event", "readiness", "remove"]);
    assert!(writer.idle());
    assert!(!fatal.load(Ordering::SeqCst));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn overflow_is_bounded_fatal_and_never_claims_queued_readiness_durable() {
    let (mut writer, entered, release, order, fatal) = fixture(false);
    let requests = writer.requests();
    requests.event(event()).unwrap();
    entered.await.unwrap();
    for _ in 0..QUEUE_CAPACITY {
        requests.event(event()).unwrap();
    }
    assert_eq!(requests.event(event()), Err(ManagedFailure::Write));
    assert!(fatal.load(Ordering::SeqCst));
    assert!(requests.remove_final().is_err());
    writer.stop();
    release.send(()).unwrap();
    writer.joined().await.unwrap();
    assert_eq!(*order.lock().unwrap(), ["event"]);
    assert!(writer.idle());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_output_latches_and_returns_fixed_receipt_errors() {
    let (mut writer, entered, release, order, fatal) = fixture(true);
    let requests = writer.requests();
    requests.event(event()).unwrap();
    entered.await.unwrap();
    let receipt = requests.remove_final().unwrap();
    release.send(()).unwrap();
    assert_eq!(receipt.await.unwrap(), Err(ManagedFailure::Sync));
    writer.stop();
    writer.joined().await.unwrap();
    assert!(fatal.load(Ordering::SeqCst));
    assert_eq!(*order.lock().unwrap(), ["event"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn panicked_output_owner_fails_receipts_and_never_reports_idle_completion() {
    struct PanickingSink(ControlledSink);
    impl Sink for PanickingSink {
        fn event(&self, event: &OperationalEvent) -> Result<(), ManagedFailure> {
            self.0.event(event)?;
            panic!("synthetic output-owner panic");
        }
        fn readiness(&self, _: &ReadinessDocument) -> Result<(), ManagedFailure> {
            unreachable!()
        }
        fn remove(&self) -> Result<(), ManagedFailure> {
            unreachable!()
        }
    }
    let (entered, started) = oneshot::channel();
    let (release, wait) = mpsc::channel();
    let fatal = Arc::new(AtomicBool::new(false));
    let failed = fatal.clone();
    let mut writer = TelemetryWriter::start_sink(
        PanickingSink(ControlledSink {
            entered: Mutex::new(Some(entered)),
            release: Mutex::new(wait),
            order: Arc::new(Mutex::new(Vec::new())),
            fail: false,
        }),
        Arc::new(move || {
            failed.store(true, Ordering::SeqCst);
        }),
    );
    let requests = writer.requests();
    requests.event(event()).unwrap();
    started.await.unwrap();
    let document = ReadinessDocument::decode(include_bytes!(
        "../../../tests/fixtures/service-protocol/readiness-v1.json"
    ))
    .unwrap();
    let ready = requests.readiness(document).unwrap();
    let removed = requests.remove_final().unwrap();
    release.send(()).unwrap();
    assert!(ready.await.is_err());
    assert!(removed.await.is_err());
    assert!(writer.joined().await.unwrap_err().is_panic());
    assert!(fatal.load(Ordering::SeqCst));
    assert!(!writer.idle());
}
