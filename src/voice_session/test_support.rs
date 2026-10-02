use super::*;

impl VoiceRuntime {
    pub(crate) fn record_spoken_play_start_for_test(&self) {
        self.test_spoken_play_starts.fetch_add(1, Ordering::SeqCst);
    }
    pub(crate) fn spoken_play_starts_for_test(&self) -> u64 {
        self.test_spoken_play_starts.load(Ordering::SeqCst)
    }

    pub(crate) fn hold_activation_gate_for_test(&self, action: impl FnOnce()) {
        let _gate = self.activation_gate.lock().unwrap();
        action();
    }

    pub(crate) fn observe_next_media_gate_for_test(&self, reached: Arc<AtomicBool>) {
        *self.media_gate_observer.lock().unwrap() = Some(reached);
    }

    pub(super) fn observe_media_gate_wait_for_test(&self) {
        if let Some(reached) = self.media_gate_observer.lock().unwrap().take() {
            reached.store(true, Ordering::Release);
        }
    }
}
