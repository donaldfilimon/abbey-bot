use super::memory_commands::{
    PendingButtonAction, format_pending_list_body, parse_pending_button_custom_id,
    pending_action_rows, pending_button_custom_id,
};
use crate::memory::PendingSupersession;

#[test]
fn custom_id_round_trips() {
    let id = pending_button_custom_id(42, PendingButtonAction::Confirm, 99, 3);
    assert_eq!(id, "42:p:c:99:3");
    assert_eq!(
        parse_pending_button_custom_id(&id, 42),
        Some((PendingButtonAction::Confirm, 99, 3))
    );
    assert!(parse_pending_button_custom_id(&id, 7).is_none());
    assert!(parse_pending_button_custom_id("42:p:x:99:3", 42).is_none());
    assert!(parse_pending_button_custom_id("42:p:c:99:3:extra", 42).is_none());
}

#[test]
fn action_rows_cap_at_five() {
    let pending: Vec<_> = (0..7)
        .map(|i| PendingSupersession {
            old_fact: format!("old-{i}"),
            new_fact: format!("new-{i}"),
            at: i as u64,
        })
        .collect();
    let rows = pending_action_rows(1, 2, &pending, 0);
    assert_eq!(rows.len(), 5);
    let body = format_pending_list_body(2, &pending);
    assert!(body.contains("Buttons cover the first 5"));
    assert!(body.contains("1. old-0 → new-0"));
}
