//! Pure text-control policy for the voice runtime.
//!
//! This module may describe or stop an existing/pending session. It never has
//! an activation result: positive consent text cannot open the media gate.

use super::{VoicePhase, VoiceSnapshot};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoiceControlIntent {
    ObserveState,
    WithdrawConsent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WithdrawalSubject {
    Speaker,
    SpeakerGroup,
    Implied,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WithdrawalPolarity {
    ExplicitNegative,
    Revocation,
    Imperative,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WithdrawalAction {
    Consent,
    Withdraw,
    Revoke,
    Stop,
    Pause,
    Leave,
    Disable,
    TurnOff,
    Listen,
    Record,
    Transcribe,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WithdrawalObject {
    Consent,
    Voice,
    Listening,
    AudioProcessing,
    Recording,
    Transcription,
    Microphone,
    SpeechRecognition,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VoiceScope {
    Voice,
    Session,
    Listening,
    AudioProcessing,
    Recording,
    Transcription,
    Microphone,
    SpeechRecognition,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WithdrawalModifier {
    Addressed,
    Polite,
    Discourse,
    Urgent,
    SessionScoped,
    Formal,
    Persistent,
}

/// Fully consumed, unambiguous negative clause. Keeping the semantic parts
/// typed prevents a substring such as `stop recording` in a question or a
/// third-party statement from accidentally becoming an active-session stop.
#[derive(Debug, Clone, PartialEq, Eq)]
struct WithdrawalClause {
    subject: WithdrawalSubject,
    polarity: WithdrawalPolarity,
    action: WithdrawalAction,
    object: WithdrawalObject,
    scope: Option<VoiceScope>,
    modifiers: Vec<WithdrawalModifier>,
}

impl WithdrawalClause {
    fn is_semantically_valid(&self) -> bool {
        let core_is_valid = matches!(
            (self.polarity, self.action, self.object, self.subject),
            (
                WithdrawalPolarity::ExplicitNegative,
                WithdrawalAction::Consent,
                WithdrawalObject::Consent,
                WithdrawalSubject::Speaker | WithdrawalSubject::SpeakerGroup,
            ) | (
                WithdrawalPolarity::Revocation,
                WithdrawalAction::Withdraw | WithdrawalAction::Revoke,
                WithdrawalObject::Consent,
                _,
            ) | (
                WithdrawalPolarity::Imperative,
                WithdrawalAction::Stop
                    | WithdrawalAction::Pause
                    | WithdrawalAction::Disable
                    | WithdrawalAction::TurnOff,
                WithdrawalObject::Voice
                    | WithdrawalObject::Listening
                    | WithdrawalObject::AudioProcessing
                    | WithdrawalObject::Recording
                    | WithdrawalObject::Transcription
                    | WithdrawalObject::Microphone
                    | WithdrawalObject::SpeechRecognition,
                WithdrawalSubject::Implied,
            ) | (
                WithdrawalPolarity::Imperative,
                WithdrawalAction::Leave,
                WithdrawalObject::Voice,
                WithdrawalSubject::Implied,
            ) | (
                WithdrawalPolarity::ExplicitNegative,
                WithdrawalAction::Listen,
                WithdrawalObject::Listening,
                WithdrawalSubject::Implied,
            ) | (
                WithdrawalPolarity::ExplicitNegative,
                WithdrawalAction::Record,
                WithdrawalObject::Recording,
                WithdrawalSubject::Implied,
            ) | (
                WithdrawalPolarity::ExplicitNegative,
                WithdrawalAction::Transcribe,
                WithdrawalObject::Transcription,
                WithdrawalSubject::Implied,
            )
        );
        let scope_is_valid = self.scope.is_none() || self.object == WithdrawalObject::Consent;
        let formal_is_valid = !self.modifiers.contains(&WithdrawalModifier::Formal)
            || matches!(
                self.action,
                WithdrawalAction::Withdraw | WithdrawalAction::Revoke
            );
        core_is_valid && scope_is_valid && formal_is_valid
    }
}

/// Classify the deliberately small operational grammar. Positive language is
/// observation-only; activation remains an authenticated slash-command path.
#[must_use]
pub fn voice_control_intent(text: &str, snapshot: &VoiceSnapshot) -> Option<VoiceControlIntent> {
    if parse_withdrawal_clause(text).is_some() {
        return Some(VoiceControlIntent::WithdrawConsent);
    }
    if is_voice_control_text(text)
        || (snapshot.phase == VoicePhase::AwaitingConsent && is_standalone_consent_response(text))
    {
        return Some(VoiceControlIntent::ObserveState);
    }
    None
}

/// Return authoritative public copy for explicit voice-consent/control text.
/// All rendered state comes from `snapshot`; provider prose and the free-form
/// internal status string are deliberately excluded.
#[must_use]
pub fn authoritative_text_reply(text: &str, snapshot: &VoiceSnapshot) -> Option<String> {
    voice_control_intent(text, snapshot).map(|_| voice_state_copy(snapshot))
}

/// A negative voice command may close an in-flight/active consent epoch, but
/// positive prose may never start one. Caller presence remains a Discord-shell
/// authorization check.
#[must_use]
pub fn requests_consent_withdrawal(text: &str, snapshot: &VoiceSnapshot) -> bool {
    voice_control_intent(text, snapshot) == Some(VoiceControlIntent::WithdrawConsent)
}

/// Recognition of a negative choice is independent of the current call phase:
/// an already-paused member must still be able to remove their saved agreement.
pub fn withdrawal_requested(text: &str) -> bool {
    parse_withdrawal_clause(text).is_some()
}

fn contains_phrase(normalized: &str, phrase: &str) -> bool {
    let mut haystack = String::with_capacity(normalized.len() + 2);
    haystack.push(' ');
    haystack.push_str(normalized);
    haystack.push(' ');

    let mut needle = String::with_capacity(phrase.len() + 2);
    needle.push(' ');
    needle.push_str(phrase);
    needle.push(' ');
    haystack.contains(&needle)
}

fn contains_any_phrase(normalized: &str, phrases: &[&str]) -> bool {
    phrases
        .iter()
        .any(|phrase| contains_phrase(normalized, phrase))
}

/// Match explicit voice-consent or control language that must be answered from
/// runtime truth instead of generative prose.
fn is_voice_control_text(text: &str) -> bool {
    const VOICE_CONTEXT: &[&str] = &[
        "voice",
        "listening",
        "listen",
        "audio",
        "microphone",
        "mic",
        "speech",
        "recognition",
        "transcription",
        "recording",
    ];
    const CONSENT_WORDS: &[&str] = &["consent", "consented", "consenting"];
    const CONSENT_PHRASES: &[&str] = &[
        "agree to audio processing",
        "agree to voice processing",
        "agree to local recognition",
        "agree to speech recognition",
        "agree to abbey listening",
        "agree to being recorded",
        "agree to be recorded",
        "agreed to audio processing",
        "agreed to voice processing",
        "agreed to local recognition",
        "agreed to speech recognition",
        "agreed to abbey listening",
        "agreed to being recorded",
        "permission to listen",
        "permission to process audio",
        "permission for voice processing",
        "give permission to listen",
        "grant permission to listen",
        "you may listen",
        "opt in to voice",
        "opt into voice",
        "opt in to listening",
        "opt in to speech recognition",
        "opted in to voice",
        "opted into voice",
    ];
    const CONTROL_PHRASES: &[&str] = &[
        "resume voice",
        "resume the voice",
        "voice resume",
        "voice resumed",
        "resume listening",
        "listening resumed",
        "restart voice",
        "restart the voice",
        "restart listening",
        "start voice",
        "start the voice",
        "voice started",
        "start listening",
        "started listening",
        "start audio processing",
        "stop voice",
        "stop the voice",
        "voice stopped",
        "stop listening",
        "stopped listening",
        "stop audio processing",
        "pause voice",
        "pause the voice",
        "voice paused",
        "pause listening",
        "listening paused",
        "pause audio processing",
        "join voice",
        "join the voice",
        "leave voice",
        "leave the voice",
        "enable voice",
        "enable the voice",
        "voice enabled",
        "enable listening",
        "disable voice",
        "disable the voice",
        "voice disabled",
        "disable listening",
        "turn voice on",
        "turn the voice on",
        "turn on voice",
        "turn on the voice",
        "turn listening on",
        "turn on listening",
        "turn voice off",
        "turn the voice off",
        "turn off voice",
        "turn off the voice",
        "turn listening off",
        "turn off listening",
        "voice is on",
        "voice is off",
        "voice is active",
        "voice is inactive",
        "voice is back on",
        "voice is back off",
        "voice back on",
        "voice back off",
        "listening is on",
        "listening is off",
        "listening is active",
        "listening is inactive",
        "listening is back on",
        "listening is back off",
        "audio processing is on",
        "audio processing is off",
        "audio processing is active",
        "audio processing is inactive",
        "audio processing resumed",
        "audio processing paused",
        "audio processing stopped",
        "speech components are back on",
        "speech components are back off",
        "speech recognition is on",
        "speech recognition is off",
        "turn microphone on",
        "turn microphone off",
        "turn mic on",
        "turn mic off",
        "do not listen",
        "dont listen",
        "are you listening",
        "is abbey listening",
        "voice status",
        "listening status",
        "has voice resumed",
        "did voice resume",
        "did listening resume",
        "is voice active",
        "is voice on",
        "is listening active",
        "is audio processing active",
    ];

    let normalized = crate::text::normalize(text);
    if normalized.is_empty() || !contains_any_phrase(&normalized, VOICE_CONTEXT) {
        return false;
    }
    contains_any_phrase(&normalized, CONSENT_WORDS)
        || contains_any_phrase(&normalized, CONSENT_PHRASES)
        || contains_any_phrase(&normalized, CONTROL_PHRASES)
}

/// Short consent responses are observation-only and are recognized only while
/// renewed consent is being collected. A bare `yes` stays on the social path.
fn is_standalone_consent_response(text: &str) -> bool {
    const RESPONSES: &[&str] = &[
        "i agree",
        "i consent",
        "we consent",
        "we all consent",
        "everyone consents",
        "everyone consent",
        "all consent",
        "i opt in",
        "we opt in",
    ];

    let normalized = crate::text::normalize(text);
    let normalized = normalized.strip_prefix("abbey ").unwrap_or(&normalized);
    RESPONSES.contains(&normalized)
}

fn parse_withdrawal_clause(text: &str) -> Option<WithdrawalClause> {
    // A question is not an operative withdrawal. This also preserves the
    // distinction between `stop listening` and `can you stop listening?`
    // after punctuation normalization.
    if text
        .chars()
        .any(|character| matches!(character, '?' | '\u{061f}' | '\u{ff1f}'))
    {
        return None;
    }

    let normalized = crate::text::normalize(text);
    let tokens = normalized.split_whitespace().collect::<Vec<_>>();
    let (core, modifiers) = peel_withdrawal_modifiers(&tokens);
    if core.is_empty() || is_question_stem(core[0]) {
        return None;
    }

    parse_consent_withdrawal(core, &modifiers)
        .or_else(|| parse_media_withdrawal(core, &modifiers))
        .filter(WithdrawalClause::is_semantically_valid)
}

fn peel_withdrawal_modifiers<'a>(tokens: &'a [&str]) -> (&'a [&'a str], Vec<WithdrawalModifier>) {
    let mut start = 0;
    let mut end = tokens.len();
    let mut modifiers = Vec::new();

    loop {
        let remaining = &tokens[start..end];
        if remaining.starts_with(&["hey", "abbey"]) {
            modifiers.push(WithdrawalModifier::Addressed);
            start += 2;
        } else if remaining.first().is_some_and(|word| *word == "abbey") {
            modifiers.push(WithdrawalModifier::Addressed);
            start += 1;
        } else if remaining.first().is_some_and(|word| *word == "please") {
            modifiers.push(WithdrawalModifier::Polite);
            start += 1;
        } else if remaining
            .first()
            .is_some_and(|word| matches!(*word, "no" | "actually" | "sorry"))
        {
            modifiers.push(WithdrawalModifier::Discourse);
            start += 1;
        } else {
            break;
        }
    }

    loop {
        let remaining = &tokens[start..end];
        if remaining.ends_with(&["for", "this", "voice", "session"]) {
            modifiers.push(WithdrawalModifier::SessionScoped);
            end -= 4;
        } else if remaining.ends_with(&["for", "this", "session"]) {
            modifiers.push(WithdrawalModifier::SessionScoped);
            end -= 3;
        } else if remaining.ends_with(&["right", "now"]) {
            modifiers.push(WithdrawalModifier::Urgent);
            end -= 2;
        } else if remaining.ends_with(&["any", "longer"]) {
            modifiers.push(WithdrawalModifier::Persistent);
            end -= 2;
        } else if remaining
            .last()
            .is_some_and(|word| matches!(*word, "now" | "immediately"))
        {
            modifiers.push(WithdrawalModifier::Urgent);
            end -= 1;
        } else if remaining.last().is_some_and(|word| *word == "anymore") {
            modifiers.push(WithdrawalModifier::Persistent);
            end -= 1;
        } else if remaining.last().is_some_and(|word| *word == "please") {
            modifiers.push(WithdrawalModifier::Polite);
            end -= 1;
        } else {
            break;
        }
    }

    (&tokens[start..end], modifiers)
}

fn is_question_stem(word: &str) -> bool {
    matches!(
        word,
        "are"
            | "can"
            | "could"
            | "did"
            | "does"
            | "how"
            | "is"
            | "may"
            | "should"
            | "what"
            | "when"
            | "where"
            | "who"
            | "why"
            | "will"
            | "would"
    )
}

fn parse_subject<'a>(tokens: &'a [&'a str]) -> (WithdrawalSubject, &'a [&'a str]) {
    match tokens.split_first() {
        Some((&"i", rest)) => (WithdrawalSubject::Speaker, rest),
        Some((&"we", rest)) => (WithdrawalSubject::SpeakerGroup, rest),
        _ => (WithdrawalSubject::Implied, tokens),
    }
}

fn parse_consent_withdrawal(
    tokens: &[&str],
    modifiers: &[WithdrawalModifier],
) -> Option<WithdrawalClause> {
    let (mut subject, mut rest) = parse_subject(tokens);
    let mut clause_modifiers = modifiers.to_vec();
    if rest.first().is_some_and(|word| *word == "hereby") {
        clause_modifiers.push(WithdrawalModifier::Formal);
        rest = &rest[1..];
    }

    let negative_remainder = if rest.starts_with(&["do", "not"]) {
        Some(&rest[2..])
    } else if rest.starts_with(&["dont"]) {
        Some(&rest[1..])
    } else if rest.starts_with(&["no", "longer"]) {
        Some(&rest[2..])
    } else {
        None
    };
    if let Some(negative_remainder) = negative_remainder {
        if subject == WithdrawalSubject::Implied {
            return None;
        }
        let (&verb, scope_tokens) = negative_remainder.split_first()?;
        if !matches!(verb, "consent" | "agree") {
            return None;
        }
        let scope = parse_consent_scope(scope_tokens)?;
        // `I do not agree` is generic disagreement, not voice withdrawal.
        if verb == "agree" && scope.is_none() {
            return None;
        }
        return Some(WithdrawalClause {
            subject,
            polarity: WithdrawalPolarity::ExplicitNegative,
            action: WithdrawalAction::Consent,
            object: WithdrawalObject::Consent,
            scope,
            modifiers: clause_modifiers,
        });
    }

    let (&verb, mut object_tokens) = rest.split_first()?;
    let action = match verb {
        "withdraw" => WithdrawalAction::Withdraw,
        "revoke" => WithdrawalAction::Revoke,
        _ => return None,
    };
    if let Some((&possessive, remaining)) = object_tokens.split_first()
        && matches!(possessive, "my" | "our")
    {
        let possessive_subject = if possessive == "my" {
            WithdrawalSubject::Speaker
        } else {
            WithdrawalSubject::SpeakerGroup
        };
        if subject != WithdrawalSubject::Implied && subject != possessive_subject {
            return None;
        }
        subject = possessive_subject;
        object_tokens = remaining;
    }
    let (&object, scope_tokens) = object_tokens.split_first()?;
    if object != "consent" {
        return None;
    }
    let scope = parse_consent_scope(scope_tokens)?;
    Some(WithdrawalClause {
        subject,
        polarity: WithdrawalPolarity::Revocation,
        action,
        object: WithdrawalObject::Consent,
        scope,
        modifiers: clause_modifiers,
    })
}

fn parse_consent_scope(tokens: &[&str]) -> Option<Option<VoiceScope>> {
    if tokens.is_empty() {
        return Some(None);
    }
    let (&connector, scope_tokens) = tokens.split_first()?;
    if !matches!(connector, "to" | "for") {
        return None;
    }
    parse_media_object_exact(scope_tokens).map(|(_, scope)| Some(scope))
}

fn parse_media_withdrawal(
    tokens: &[&str],
    modifiers: &[WithdrawalModifier],
) -> Option<WithdrawalClause> {
    let (&first, rest) = tokens.split_first()?;
    let (polarity, action, object_tokens) = if first == "dont" {
        let (&verb, object_tokens) = rest.split_first()?;
        (
            WithdrawalPolarity::ExplicitNegative,
            negative_media_action(verb)?,
            object_tokens,
        )
    } else if first == "do" && rest.starts_with(&["not"]) {
        let (&verb, object_tokens) = rest[1..].split_first()?;
        (
            WithdrawalPolarity::ExplicitNegative,
            negative_media_action(verb)?,
            object_tokens,
        )
    } else if first == "turn" {
        let (object, scope) = parse_turn_off(rest)?;
        return Some(WithdrawalClause {
            subject: WithdrawalSubject::Implied,
            polarity: WithdrawalPolarity::Imperative,
            action: WithdrawalAction::TurnOff,
            object,
            scope: None,
            modifiers: modifiers.to_vec(),
        })
        .filter(|clause| scope != VoiceScope::Session || clause.object == WithdrawalObject::Voice);
    } else {
        let action = match first {
            "stop" => WithdrawalAction::Stop,
            "pause" => WithdrawalAction::Pause,
            "leave" => WithdrawalAction::Leave,
            "disable" => WithdrawalAction::Disable,
            _ => return None,
        };
        (WithdrawalPolarity::Imperative, action, rest)
    };

    let (object, _) = if polarity == WithdrawalPolarity::ExplicitNegative {
        negative_media_object(action, object_tokens)?
    } else {
        parse_media_object_exact(object_tokens)?
    };
    Some(WithdrawalClause {
        subject: WithdrawalSubject::Implied,
        polarity,
        action,
        object,
        scope: None,
        modifiers: modifiers.to_vec(),
    })
}

fn negative_media_action(verb: &str) -> Option<WithdrawalAction> {
    match verb {
        "listen" => Some(WithdrawalAction::Listen),
        "record" => Some(WithdrawalAction::Record),
        "transcribe" => Some(WithdrawalAction::Transcribe),
        _ => None,
    }
}

fn negative_media_object(
    action: WithdrawalAction,
    tokens: &[&str],
) -> Option<(WithdrawalObject, VoiceScope)> {
    match action {
        WithdrawalAction::Listen if tokens.is_empty() || tokens == ["to", "me"] => {
            Some((WithdrawalObject::Listening, VoiceScope::Listening))
        }
        WithdrawalAction::Record if tokens.is_empty() || tokens == ["me"] => {
            Some((WithdrawalObject::Recording, VoiceScope::Recording))
        }
        WithdrawalAction::Transcribe if tokens.is_empty() || tokens == ["me"] => {
            Some((WithdrawalObject::Transcription, VoiceScope::Transcription))
        }
        _ => None,
    }
}

fn parse_turn_off(tokens: &[&str]) -> Option<(WithdrawalObject, VoiceScope)> {
    if let Some(object_tokens) = tokens.strip_prefix(&["off"]) {
        return parse_media_object_exact(object_tokens);
    }
    let object_tokens = tokens.strip_suffix(&["off"])?;
    parse_media_object_exact(object_tokens)
}

fn parse_media_object_exact(tokens: &[&str]) -> Option<(WithdrawalObject, VoiceScope)> {
    let mut tokens = tokens;
    while tokens.first().is_some_and(|word| {
        matches!(
            *word,
            "the" | "this" | "current" | "local" | "my" | "our" | "abbey"
        )
    }) {
        tokens = &tokens[1..];
    }

    let (object, scope, remainder) = match tokens {
        ["voice"] => (
            WithdrawalObject::Voice,
            VoiceScope::Voice,
            &tokens[tokens.len()..],
        ),
        ["voice", "session"] => (
            WithdrawalObject::Voice,
            VoiceScope::Session,
            &tokens[tokens.len()..],
        ),
        ["voice", "recording"] => (
            WithdrawalObject::Recording,
            VoiceScope::Recording,
            &tokens[tokens.len()..],
        ),
        ["voice", "processing"] | ["audio"] | ["audio", "processing"] => (
            WithdrawalObject::AudioProcessing,
            VoiceScope::AudioProcessing,
            &tokens[tokens.len()..],
        ),
        ["listening"] => (
            WithdrawalObject::Listening,
            VoiceScope::Listening,
            &tokens[tokens.len()..],
        ),
        ["listening", "to", "me"] => (
            WithdrawalObject::Listening,
            VoiceScope::Listening,
            &tokens[tokens.len()..],
        ),
        ["recording"] | ["recording", "me"] | ["being", "recorded"] => (
            WithdrawalObject::Recording,
            VoiceScope::Recording,
            &tokens[tokens.len()..],
        ),
        ["transcription"] | ["transcribing"] | ["transcribing", "me"] => (
            WithdrawalObject::Transcription,
            VoiceScope::Transcription,
            &tokens[tokens.len()..],
        ),
        ["microphone"] | ["mic"] => (
            WithdrawalObject::Microphone,
            VoiceScope::Microphone,
            &tokens[tokens.len()..],
        ),
        ["speech", "recognition"] => (
            WithdrawalObject::SpeechRecognition,
            VoiceScope::SpeechRecognition,
            &tokens[tokens.len()..],
        ),
        _ => return None,
    };
    if remainder.is_empty() {
        Some((object, scope))
    } else {
        None
    }
}

fn voice_state_copy(snapshot: &VoiceSnapshot) -> String {
    if snapshot.phase.processes_audio() && snapshot.media_enabled {
        let replacement = if snapshot.start_pending {
            " A replacement start is pending, but the current consented session remains active until it is explicitly stopped or safely replaced."
        } else {
            ""
        };
        return format!(
            "Voice is active for the current consent epoch. Current phase: {} · consent epoch: {} · participants recorded: {}. Participant audio processing is enabled; `/voice leave` stops it.{replacement}",
            snapshot.phase.label(),
            snapshot.consent_epoch,
            snapshot.participant_count
        );
    }
    if snapshot.start_pending {
        return format!(
            "A voice start is pending, but the media gate is closed and no participant audio is being processed. An explicit withdrawal or `/voice leave` cancels it; only final membership, permission, and consent checks can activate it. Consent epoch: {} · participants recorded: {}.",
            snapshot.consent_epoch, snapshot.participant_count
        );
    }
    match snapshot.phase {
        VoicePhase::Disconnected => format!(
            "Voice is disconnected, and no participant audio is being processed. Starting it requires consent from everyone currently present and a manager's `/voice join consent:true`. Consent epoch: {} · participants recorded: {}.",
            snapshot.consent_epoch, snapshot.participant_count
        ),
        VoicePhase::PresenceOnly => format!(
            "Abbey is present only in muted/self-deafened no-audio mode. Capture, recognition, reasoning, synthesis, and playback are disabled. Starting voice requires everyone-present consent and a manager's `/voice join consent:true`. Consent epoch: {} · participants recorded: {}.",
            snapshot.consent_epoch, snapshot.participant_count
        ),
        VoicePhase::Connecting => format!(
            "Voice is still connecting and has not become active. The software media gate is closed, so no participant audio is being processed. Check `/voice status` before treating voice as started. Consent epoch: {} · participants recorded: {}.",
            snapshot.consent_epoch, snapshot.participant_count
        ),
        VoicePhase::Listening | VoicePhase::Thinking | VoicePhase::Speaking => format!(
            "Voice is stopping or paused. Its media gate is closed, so no participant audio is being processed while the runtime leaves phase {}. Check `/voice status` for the settled state. Consent epoch: {} · participants recorded: {}.",
            snapshot.phase.label(),
            snapshot.consent_epoch,
            snapshot.participant_count
        ),
        VoicePhase::AwaitingConsent => format!(
            "Voice has not resumed. Abbey is paused with its media gate closed, so no participant audio is being processed; the pause procedure also tears down any existing conversational connection. Renewed consent from everyone currently present plus a manager's `/voice resume consent:true` are required before voice can restart. Consent epoch: {} · participants recorded: {}.",
            snapshot.consent_epoch, snapshot.participant_count
        ),
        VoicePhase::Failed => format!(
            "Voice failed safe, and no participant audio is being processed. A manager can inspect `/voice status`; any new start still requires everyone-present consent and `/voice join consent:true`. Consent epoch: {} · participants recorded: {}.",
            snapshot.consent_epoch, snapshot.participant_count
        ),
    }
}

#[cfg(test)]
mod tests;
