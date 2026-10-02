use super::*;
fn source() -> SourceRef {
    SourceRef {
        scope: EngagementScope::Guild {
            guild: 7,
            channel: 9,
        },
        message: 10,
        author: 2,
        revision: 1,
        at: 100,
    }
}
#[test]
fn public_contract_binds_substantive_authored_request_without_question_mark() {
    let s = source();
    let text = "Please explain how this compiler lowers an async suspension into state transitions";
    let raw=serde_json::json!({"outcome":"unanswered_question","source_messages":[10],"question":{"source_message":10,"excerpt":text}}).to_string();
    assert_eq!(
        parse_public_assessment(&raw, &s, text).unwrap().outcome,
        PublicOutcome::UnansweredQuestion
    );
    assert!(PUBLIC_PROMPT.contains("NEVER unanswered_question"));
    assert!(PUBLIC_PROMPT.contains("quoted/code/example"));
}
#[test]
fn public_contract_rejects_personal_class_invented_question_and_unknown_fields() {
    let s = source();
    let text = "I am working on a compiler project.";
    for raw in [
        serde_json::json!({"outcome":"unresolved","source_messages":[10]}),
        serde_json::json!({"outcome":"unanswered_question","source_messages":[10],"question":null}),
        serde_json::json!({"outcome":"unanswered_question","source_messages":[10],"question":{"source_message":11,"excerpt":text}}),
        serde_json::json!({"outcome":"unanswered_question","source_messages":[10],"question":{"source_message":10,"excerpt":"How do I fix it?"}}),
        serde_json::json!({"outcome":"useful_context","source_messages":[11],"question":null}),
        serde_json::json!({"outcome":"useful_context","source_messages":[10,10],"question":null}),
        serde_json::json!({"outcome":"useful_context","source_messages":[10],"question":null,"tools":[]}),
    ] {
        assert!(parse_public_assessment(&raw.to_string(), &s, text).is_err());
    }
    let mut dm = s.clone();
    dm.scope = EngagementScope::Dm {
        member: 2,
        channel: 9,
    };
    assert!(
        parse_public_assessment(
            r#"{"outcome":"useful_context","source_messages":[10],"question":null}"#,
            &dm,
            text
        )
        .is_err()
    );
}
