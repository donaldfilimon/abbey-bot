use super::*;
use std::sync::{Arc, Mutex};

#[tokio::test]
async fn acknowledgement_completes_before_permission_work_is_constructed() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let result = acknowledged(
        async {
            events.lock().unwrap().push("ack");
            Ok(())
        },
        || {
            events.lock().unwrap().push("permissions");
            async {
                events.lock().unwrap().push("operation");
                Ok(())
            }
        },
    )
    .await;
    assert!(result.is_ok());
    assert_eq!(
        *events.lock().unwrap(),
        vec!["ack", "permissions", "operation"]
    );
}
#[tokio::test]
async fn failed_acknowledgement_never_constructs_domain_work() {
    let called = std::sync::atomic::AtomicBool::new(false);
    let result = acknowledged(async { Err::<(), Error>("ack failed".into()) }, || {
        called.store(true, std::sync::atomic::Ordering::SeqCst);
        async { Ok(()) }
    })
    .await;
    assert!(result.is_err());
    assert!(!called.load(std::sync::atomic::Ordering::SeqCst));
}
#[test]
fn modal_accepts_exactly_one_named_question_field() {
    let valid = serde_json::json!([{"type":1,"components":[{"type":4,"custom_id":"question","style":2,"label":"Question","value":" hi "}]}]);
    let rows: Vec<serenity::all::ActionRow> = serde_json::from_value(valid.clone()).unwrap();
    assert_eq!(modal_question(&rows), Some("hi"));
    assert_eq!(modal_question(&[]), None);
    let mut wrong = valid.clone();
    wrong[0]["components"][0]["custom_id"] = "other".into();
    assert_eq!(
        modal_question(&serde_json::from_value::<Vec<_>>(wrong).unwrap()),
        None
    );
    let mut duplicates = valid.clone();
    duplicates[0]["components"]
        .as_array_mut()
        .unwrap()
        .push(valid[0]["components"][0].clone());
    assert_eq!(
        modal_question(&serde_json::from_value::<Vec<_>>(duplicates).unwrap()),
        None
    );
}
#[test]
fn manager_task_is_visible_only_with_current_authority() {
    let mut input = catalog::EligibilityInput::new(catalog::InteractionContext::Guild);
    let member = serde_json::to_string(&rows(1, Some(2), 3, 900, &input)).unwrap();
    assert!(!member.contains("Manage Abbey"));
    input
        .permissions
        .push(catalog::DiscordPermission::ManageServer);
    let manager = serde_json::to_value(rows(1, Some(2), 3, 900, &input)).unwrap();
    assert_eq!(manager[0]["components"].as_array().unwrap().len(), 5);
    assert!(manager.to_string().contains("Manage Abbey"));
}
#[test]
fn image_guidance_names_real_inputs_and_both_private_menus() {
    let body = image_guidance(&catalog::EligibilityInput::new(
        catalog::InteractionContext::Guild,
    ));
    for exact in [
        "/see image:<attachment>",
        "/ocr image:<attachment>",
        "Abbey: describe image",
        "Abbey: read image text",
        "has not read or submitted",
    ] {
        assert!(body.contains(exact));
    }
}
