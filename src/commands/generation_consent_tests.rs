use super::*;

#[test]
fn personal_memory_adapter_slash_delivery_rechecks_captured_exposure() {
    let state = AppState::in_memory();
    let context = pipeline::assemble_context(
        &state,
        "discord:g",
        "discord:u",
        "discord:c",
        "question",
        0.5,
    );
    let guard = generation::consent::GenerationGuard::capture(
        &state,
        &generation::Ask {
            subject: Some(("discord:g", "discord:u")),
            session_mode: generation::SessionMode::SourceOnly,
            scope: "discord:c",
            context: &context,
            user_input: "question",
            now: 1,
        },
    )
    .unwrap();
    let reply = GeneratedReply {
        text: "PREPARED_PRIVATE_ANSWER".into(),
        memory: Default::default(),
        guard: Some(guard),
        timing: None,
    };
    assert_eq!(reply.delivery_text(&state), "PREPARED_PRIVATE_ANSWER");
    state
        .memory_service()
        .remember(
            "discord:g",
            "discord:other",
            "other ordinary fact advances exposure",
            2,
        )
        .unwrap();
    assert_eq!(
        reply.delivery_text(&state),
        generation::consent::WITHDRAWN_REPLY
    );
}
