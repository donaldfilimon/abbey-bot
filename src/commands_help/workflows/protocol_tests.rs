use super::*;

fn session() -> Session {
    Session {
        owner: 7,
        guild: Some(8),
        channel: 9,
        expiry: 1000,
        action: Action::Conversation,
    }
}
#[test]
fn roundtrip_binds_every_identity_and_expiry() {
    let id = session().custom_id();
    assert!(validate(&id, 7, Some(8), 9, 999).is_ok());
    assert!(validate(&id, 6, Some(8), 9, 999).is_err());
    assert!(validate(&id, 7, Some(6), 9, 999).is_err());
    assert!(validate(&id, 7, None, 9, 999).is_err());
    assert!(validate(&id, 7, Some(8), 6, 999).is_err());
    assert!(validate(&id, 7, Some(8), 9, 1000).is_err());
    assert!(validate(&id, 7, Some(8), 9, 0).is_err());
}
#[test]
fn exact_actions_and_strict_envelope() {
    for action in Action::ALL {
        let s = Session {
            action,
            ..session()
        };
        assert_eq!(
            validate(&s.custom_id(), 7, Some(8), 9, 999).unwrap().action,
            action
        );
    }
    for id in [
        "abbey:task:v2:7:8:9:1000:ask",
        "abbey:task:v1:07:8:9:1000:ask",
        "abbey:task:v1:7:8:9:1000:delete",
        "abbey:task:v1:7:8:9:1000:ask:extra",
        "abbey:task:v1:7:0:9:1000:ask",
    ] {
        assert!(validate(id, 7, Some(8), 9, 999).is_err());
    }
}
#[test]
fn discord_id_limit_and_dm_binding() {
    let s = Session {
        owner: u64::MAX,
        guild: Some(u64::MAX),
        channel: u64::MAX,
        expiry: 2_000_000_000,
        action: Action::Administration,
    };
    assert!(s.custom_id().len() <= 100);
    let dm = Session {
        guild: None,
        ..session()
    };
    assert!(validate(&dm.custom_id(), 7, None, 9, 999).is_ok());
}
#[test]
fn question_validation_rejects_empty_and_excessive_input() {
    assert_eq!(question("  hi  "), Some("hi"));
    assert_eq!(question(" \n "), None);
    assert_eq!(question(&"x".repeat(2001)), None);
}
