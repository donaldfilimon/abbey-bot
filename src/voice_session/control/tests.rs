use super::*;

fn snapshot(phase: VoicePhase) -> VoiceSnapshot {
    VoiceSnapshot {
        epoch: 12,
        phase,
        media_enabled: phase.processes_audio(),
        start_pending: false,
        status: "untrusted prose saying speech is back on".into(),
        consent_epoch: 7,
        participant_count: 4,
        dropped_input: 1,
        aborted_overruns: 2,
        barge_ins: 3,
        completed_turns: 4,
    }
}

#[test]
fn flexible_negative_grammar_accepts_clear_withdrawals() {
    let active = snapshot(VoicePhase::Listening);
    for text in [
        "No, I don't consent.",
        "I withdraw consent",
        "stop listening immediately",
        "revoke consent for this session",
        "Abbey, please stop recording me right now",
        "I do not consent to voice recording",
    ] {
        assert_eq!(
            voice_control_intent(text, &active),
            Some(VoiceControlIntent::WithdrawConsent),
            "expected withdrawal: {text}"
        );
        assert!(requests_consent_withdrawal(text, &active));
    }
}

#[test]
fn withdrawal_parser_composes_subject_polarity_action_object_and_modifiers() {
    let refusal =
        parse_withdrawal_clause("No, I don't consent anymore.").expect("first-person refusal");
    assert_eq!(refusal.subject, WithdrawalSubject::Speaker);
    assert_eq!(refusal.polarity, WithdrawalPolarity::ExplicitNegative);
    assert_eq!(refusal.action, WithdrawalAction::Consent);
    assert_eq!(refusal.object, WithdrawalObject::Consent);
    assert_eq!(refusal.scope, None);
    assert!(refusal.modifiers.contains(&WithdrawalModifier::Discourse));
    assert!(refusal.modifiers.contains(&WithdrawalModifier::Persistent));

    let revocation = parse_withdrawal_clause(
        "Abbey, we hereby revoke our consent for this voice session, please.",
    )
    .expect("scoped group revocation");
    assert_eq!(revocation.subject, WithdrawalSubject::SpeakerGroup);
    assert_eq!(revocation.polarity, WithdrawalPolarity::Revocation);
    assert_eq!(revocation.action, WithdrawalAction::Revoke);
    assert_eq!(revocation.object, WithdrawalObject::Consent);
    assert!(
        revocation
            .modifiers
            .contains(&WithdrawalModifier::Addressed)
    );
    assert!(
        revocation
            .modifiers
            .contains(&WithdrawalModifier::SessionScoped)
    );
    assert!(revocation.modifiers.contains(&WithdrawalModifier::Formal));

    let command = parse_withdrawal_clause("Please stop listening immediately")
        .expect("polite urgent imperative");
    assert_eq!(command.subject, WithdrawalSubject::Implied);
    assert_eq!(command.polarity, WithdrawalPolarity::Imperative);
    assert_eq!(command.action, WithdrawalAction::Stop);
    assert_eq!(command.object, WithdrawalObject::Listening);
    assert!(command.modifiers.contains(&WithdrawalModifier::Polite));
    assert!(command.modifiers.contains(&WithdrawalModifier::Urgent));

    let negative_command =
        parse_withdrawal_clause("don't transcribe me").expect("negative media command");
    assert_eq!(
        (
            negative_command.subject,
            negative_command.polarity,
            negative_command.action,
            negative_command.object,
        ),
        (
            WithdrawalSubject::Implied,
            WithdrawalPolarity::ExplicitNegative,
            WithdrawalAction::Transcribe,
            WithdrawalObject::Transcription,
        )
    );
}

#[test]
fn withdrawal_parser_requires_a_fully_consumed_first_party_voice_clause() {
    for text in [
        "how do I stop voice",
        "can you stop recording?",
        "would you please stop listening",
        "stop recording Donald",
        "do not transcribe them",
        "revoke their consent",
        "Donald said I withdraw consent",
        "I do not consent to the code of conduct",
        "I do not agree to the privacy policy",
        "I withdraw consent and restart voice",
    ] {
        assert!(
            parse_withdrawal_clause(text).is_none(),
            "unexpected withdrawal clause: {text}"
        );
    }
}

#[test]
fn withdrawal_parser_accepts_composed_voice_scopes_without_sentence_templates() {
    for text in [
        "I don't consent to voice recording",
        "we do not agree to local speech recognition",
        "I no longer consent to being recorded",
        "withdraw my consent to this voice session",
        "revoke our consent to audio processing",
        "turn the microphone off",
        "turn off the mic",
        "do not listen to me",
        "don't record",
    ] {
        assert!(
            parse_withdrawal_clause(text).is_some(),
            "expected composed withdrawal clause: {text}"
        );
    }
}

#[test]
fn positive_or_ambiguous_text_can_never_withdraw_or_activate() {
    let active = snapshot(VoicePhase::Listening);
    for text in [
        "I consent",
        "we all consent",
        "resume voice",
        "start listening",
        "how do I stop voice?",
        "can you stop transcribing someone else?",
        "I do not consent to the code of conduct",
    ] {
        assert!(!requests_consent_withdrawal(text, &active), "{text}");
    }
    assert_eq!(
        voice_control_intent("resume voice", &active),
        Some(VoiceControlIntent::ObserveState)
    );
}
