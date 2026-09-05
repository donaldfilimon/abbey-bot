use std::convert::Infallible;
use std::sync::{Arc, Mutex};

use super::acknowledgement::{
    EphemeralAcknowledger, acknowledge_with_transition, authorize_and_close_media,
    with_acknowledged_context,
};

#[derive(Clone)]
struct RecordingContext {
    events: Arc<Mutex<Vec<&'static str>>>,
}

impl RecordingContext {
    fn record(&self, event: &'static str) {
        self.events.lock().unwrap().push(event);
    }
}

impl EphemeralAcknowledger for RecordingContext {
    type Error = Infallible;

    async fn defer_ephemeral(&self) -> Result<(), Self::Error> {
        self.record("acknowledge");
        Ok::<(), Infallible>(())
    }
}

async fn assert_start_acknowledgement_order() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let context = RecordingContext {
        events: Arc::clone(&events),
    };

    with_acknowledged_context(context, |context| async move {
        context.record("guard");
        context.record("network");
        Ok::<(), Infallible>(())
    })
    .await
    .unwrap();

    assert_eq!(*events.lock().unwrap(), ["acknowledge", "guard", "network"]);
}

#[tokio::test]
async fn voice_join_acknowledges_before_every_guard_or_network_path() {
    assert_start_acknowledgement_order().await;
}

#[tokio::test]
async fn voice_resume_acknowledges_before_every_guard_or_network_path() {
    assert_start_acknowledgement_order().await;
}

#[tokio::test]
async fn voice_leave_authorizes_closes_media_then_acknowledges_with_transition() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let authorization_events = Arc::clone(&events);
    let close_events = Arc::clone(&events);
    let closed = authorize_and_close_media(
        move || {
            authorization_events.lock().unwrap().push("authorize");
            true
        },
        move || close_events.lock().unwrap().push("close-media"),
    )
    .expect("authorized leave closes media");

    let rendezvous = Arc::new(tokio::sync::Barrier::new(2));
    let acknowledgement_events = Arc::clone(&events);
    let acknowledgement_rendezvous = Arc::clone(&rendezvous);
    let acknowledgement = async move {
        acknowledgement_events.lock().unwrap().push("acknowledge");
        acknowledgement_rendezvous.wait().await;
    };
    let transition_events = Arc::clone(&events);
    let transition = async move {
        transition_events.lock().unwrap().push("transition");
        rendezvous.wait().await;
    };

    tokio::time::timeout(
        std::time::Duration::from_millis(100),
        acknowledge_with_transition(closed, acknowledgement, transition),
    )
    .await
    .expect("acknowledgement and transition must be concurrent");

    let events = events.lock().unwrap();
    assert_eq!(&events[..2], ["authorize", "close-media"]);
    assert!(events[2..].contains(&"acknowledge"));
    assert!(events[2..].contains(&"transition"));
}

#[test]
fn unauthorized_voice_leave_never_closes_media() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let authorization_events = Arc::clone(&events);
    let close_events = Arc::clone(&events);
    let closed = authorize_and_close_media(
        move || {
            authorization_events.lock().unwrap().push("authorize");
            false
        },
        move || close_events.lock().unwrap().push("close-media"),
    );

    assert!(closed.is_none());
    assert_eq!(*events.lock().unwrap(), ["authorize"]);
}
