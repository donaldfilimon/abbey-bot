use super::*;

const FEEDBACK: [(&str, &str); 5] = [
    ("u1", "abbey that was too long"),
    ("u1", "still too long"),
    ("u2", "too long tbh"),
    ("u2", "way too long"),
    ("u3", "too long"),
];

async fn mention_feedback(state: &AppState) {
    let out = FakeOut::default();
    for (user, text) in FEEDBACK {
        handle(state, &out, message(text, Some("g"), user), true, None).await;
    }
}

fn set_learning(state: &AppState, on: bool) {
    let mut stores = AppState::lock(&state.stores);
    AppState::lock(&state.guilds).update("discord:g", &mut *stores, |s| s.learning_enabled = on);
}

#[tokio::test]
async fn style_signal_ignored_when_learning_disabled() {
    let state = AppState::in_memory();
    mention_feedback(&state).await;
    assert!(
        AppState::lock(&state.stores).addenda.is_empty(),
        "nothing observed"
    );
    state.learn_all();
    assert!(
        AppState::lock(&state.stores).addenda.is_empty(),
        "nothing ticked"
    );
    let context = assemble_context(&state, "discord:g", "discord:u1", "discord:c1", "hi", 0.5);
    assert_eq!(context.addenda, "");
    let prepared =
        crate::engine::Engine::new().prepare("discord:c1", Persona::Abbey, &context, "hi", 1);
    let parts = crate::prompt_budget::PromptParts {
        addenda: prepared.addenda.clone(),
        ..crate::prompt_budget::PromptParts::new(
            prepared.persona_core.clone(),
            &prepared.context,
            String::new(),
            prepared.turns.clone(),
        )
    };
    assert_eq!(
        parts.system(),
        prepared.system_prompt,
        "dormant prompt is unchanged"
    );
}

#[tokio::test]
async fn style_feedback_about_abbey_applies_when_learning_is_on() {
    let state = AppState::in_memory();
    set_learning(&state, true);
    // Unaddressed chatter is not feedback about Abbey.
    let out = FakeOut::default();
    for (user, text) in FEEDBACK {
        handle(&state, &out, message(text, Some("g"), user), false, None).await;
    }
    assert!(AppState::lock(&state.stores).addenda.is_empty());

    mention_feedback(&state).await;
    state.learn_all();
    let context = assemble_context(&state, "discord:g", "discord:u1", "discord:c1", "hi", 0.5);
    assert!(
        context.addenda.starts_with("Style preferences"),
        "{}",
        context.addenda
    );
    assert!(
        !context.render("hi").contains("Style preferences"),
        "not a context fact"
    );

    let prepared =
        crate::engine::Engine::new().prepare("discord:c1", Persona::Abbey, &context, "hi", 1);
    assert_eq!(prepared.addenda, context.addenda);
    assert!(!prepared.system_prompt.contains("Style preferences"));
    let parts = crate::prompt_budget::PromptParts {
        addenda: prepared.addenda.clone(),
        ..crate::prompt_budget::PromptParts::new(
            prepared.persona_core.clone(),
            &prepared.context,
            String::new(),
            prepared.turns.clone(),
        )
    };
    let expected = format!("{}\n\n{}", prepared.persona_core, context.addenda);
    assert!(
        parts.system().starts_with(&expected),
        "addenda follow the persona core"
    );
    assert_eq!(parts.instructions(), expected);

    // A DM from the same member carries no addenda.
    let dm = assemble_context(
        &state,
        "discord:dm:u1",
        "discord:u1",
        "discord:c9",
        "hi",
        0.5,
    );
    assert_eq!(dm.addenda, "");

    set_learning(&state, false);
    let context = assemble_context(&state, "discord:g", "discord:u1", "discord:c1", "hi", 0.5);
    assert_eq!(context.addenda, "", "learning off silences applied addenda");
}
