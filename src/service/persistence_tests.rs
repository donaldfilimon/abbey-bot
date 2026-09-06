use super::persistence::*;
use crate::persist::{PersistErrorCategory, PersistenceSink, Stores};
use crate::wdbx::Recall;
use std::path::Path;
use std::sync::{Arc, Condvar, Mutex};
struct HeldSink {
    events: Mutex<Vec<Vec<u8>>>,
    entered: tokio::sync::Notify,
    release: (Mutex<bool>, Condvar),
}
impl PersistenceSink for HeldSink {
    fn publish(&self, _: &Path, _: &Path, bytes: &[u8]) -> Result<(), PersistErrorCategory> {
        let first = {
            let mut events = self.events.lock().unwrap();
            events.push(bytes.to_vec());
            events.len() == 1
        };
        if first {
            self.entered.notify_one();
            let (lock, ready) = &self.release;
            let mut released = lock.lock().unwrap();
            while !*released {
                released = ready.wait(released).unwrap();
            }
        }
        Ok(())
    }
}
struct ReleaseOnDrop(Arc<HeldSink>);
impl Drop for ReleaseOnDrop {
    fn drop(&mut self) {
        *self.0.release.0.lock().unwrap() = true;
        self.0.release.1.notify_all();
    }
}
fn snapshot() -> Snapshot {
    Snapshot {
        stores: Stores::default(),
        recall: Recall::new(),
    }
}
#[tokio::test]
async fn canceled_waiter_cannot_release_actual_writer_or_reuse_earlier_report() {
    let sink = Arc::new(HeldSink {
        events: Mutex::new(Vec::new()),
        entered: tokio::sync::Notify::new(),
        release: (Mutex::new(false), Condvar::new()),
    });
    let _release_on_panic = ReleaseOnDrop(sink.clone());
    let mut writer = PersistenceWriter::start(Some("test-only-injected-sink".into()), sink.clone());
    let requests = writer.requests();
    let first = tokio::spawn(async move { requests.submit(snapshot()).await });
    sink.entered.notified().await;
    first.abort();
    let _ = first.await;
    assert!(!writer.idle());
    let requests = writer.requests();
    let second = tokio::spawn(async move { requests.submit(snapshot()).await });
    tokio::task::yield_now().await;
    assert!(!second.is_finished());
    assert_eq!(sink.events.lock().unwrap().len(), 1);
    writer.close_admission();
    assert_eq!(
        writer.requests().submit(snapshot()).await.unwrap_err(),
        RequestError::Draining
    );
    *sink.release.0.lock().unwrap() = true;
    sink.release.1.notify_all();
    second.await.unwrap().unwrap();
    assert_eq!(
        sink.events.lock().unwrap().len(),
        4,
        "both canonical/projection transactions completed in FIFO order"
    );
    assert!(writer.idle());
    writer
        .final_snapshot(
            snapshot(),
            tokio::time::Instant::now() + std::time::Duration::from_secs(1),
        )
        .unwrap()
        .await
        .unwrap()
        .unwrap();
    assert!(
        writer
            .final_snapshot(
                snapshot(),
                tokio::time::Instant::now() + std::time::Duration::from_secs(1)
            )
            .is_err(),
        "root cannot start a second final transaction"
    );
    writer.stop();
    writer.joined().await.unwrap();
    assert_eq!(sink.events.lock().unwrap().len(), 6);
}

#[tokio::test]
async fn sink_panic_wakes_root_without_another_request_or_waiter() {
    struct PanicSink;
    impl PersistenceSink for PanicSink {
        fn publish(&self, _: &Path, _: &Path, _: &[u8]) -> Result<(), PersistErrorCategory> {
            panic!("controlled sink panic");
        }
    }
    let mut writer = PersistenceWriter::start(Some("injected".into()), Arc::new(PanicSink));
    let requests = writer.requests();
    let _waiter = tokio::spawn(async move {
        let _ = requests.submit(snapshot()).await;
    });
    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        writer.failure().notified(),
    )
    .await
    .unwrap();
    assert!(
        !writer.idle(),
        "panic never claims an unfinished snapshot completed"
    );
    writer.stop();
    assert!(writer.joined().await.is_err());
}

#[test]
fn expired_final_request_never_starts_io_when_blocking_worker_wakes_late() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct CountSink(Arc<AtomicUsize>);
    impl PersistenceSink for CountSink {
        fn publish(&self, _: &Path, _: &Path, _: &[u8]) -> Result<(), PersistErrorCategory> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(1)
        .build()
        .unwrap();
    runtime.block_on(async {
        let (entered, started) = tokio::sync::oneshot::channel();
        let (release, wait) = std::sync::mpsc::channel();
        let blocker = tokio::task::spawn_blocking(move || {
            let _ = entered.send(());
            let _ = wait.recv();
        });
        started.await.unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let mut writer =
            PersistenceWriter::start(Some("injected".into()), Arc::new(CountSink(calls.clone())));
        writer.close_admission();
        let receipt = writer
            .final_snapshot(
                snapshot(),
                tokio::time::Instant::now() + std::time::Duration::from_millis(10),
            )
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        release.send(()).unwrap();
        blocker.await.unwrap();
        assert_eq!(receipt.await.unwrap(), Err(RequestError::DeadlineExpired));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert!(writer.last_completed().is_none());
        writer.stop();
        writer.joined().await.unwrap();
    });
}
