//! Pure, server-scoped one-time voice choices. No membership implies agreement.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::voice::VoiceMode;

pub const POLICY_VERSION: u32 = 1;
pub const STOP_ID: &str = "abbey:voice:withdraw";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Choice {
    Agree(VoiceMode),
    Withdraw,
    WithdrawSpoken,
}

impl Choice {
    pub fn withdraws(self) -> bool {
        matches!(self, Self::Withdraw | Self::WithdrawSpoken)
    }
}

pub fn button_id(mode: VoiceMode) -> Option<&'static str> {
    match mode {
        VoiceMode::Local => Some("abbey:voice:agree:1:local"),
        VoiceMode::OpenAi => Some("abbey:voice:agree:1:openai"),
        VoiceMode::Disabled => None,
    }
}

pub fn parse_button(id: &str) -> Option<Choice> {
    match id {
        "abbey:voice:agree:1:local" => Some(Choice::Agree(VoiceMode::Local)),
        "abbey:voice:agree:1:openai" => Some(Choice::Agree(VoiceMode::OpenAi)),
        STOP_ID => Some(Choice::Withdraw),
        _ => None,
    }
}

pub fn notice(mode: VoiceMode, channel: u64) -> String {
    let processing = match mode {
        VoiceMode::Local => {
            "Local processing: Discord transports the call. Speech recognition, reasoning and speech synthesis run on Donald's Mac. Abbey can use your existing saved context for clearly attributed speech and save addressed conversation text as server-scoped context; raw audio is not retained by Abbey."
        }
        VoiceMode::OpenAi => {
            "OpenAI processing: Discord transports the call and participant audio is sent to OpenAI Realtime for recognition and replies, subject to that provider's data handling. Abbey does not retain raw audio locally. This mode does not use local persona routing or WDBX context."
        }
        VoiceMode::Disabled => {
            "Voice processing is disabled. Abbey's automatic presence is muted and self-deafened."
        }
    };
    format!(
        "**Abbey voice — your choice in this server**\n{processing}\n\nChoose **Agree** only if you want this processing in <#{channel}>. Your own agreement is remembered across visits and restarts for this processing mode and notice version; membership, silence and output mute are not agreement. Voice starts only when everyone present has agreed and a manager uses `/voice join consent:true` or `/voice resume consent:true`. New arrivals pause the call.\n\n**Stop / withdraw** clears your saved agreement for both modes. If you are in the call, it stops audio processing and disconnects Abbey. You can also mention Abbey and type `stop listening` in this voice channel. `/voice leave` stops the current call without deleting your saved choice. Use `/voice consent` anytime to review or change your choice. Saying a wake name starts a question only while voice is active.\n\nNotice version {POLICY_VERSION}."
    )
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Receipt {
    pub policy: u32,
    pub interaction: u64,
    pub acknowledged_at: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct MemberChoice {
    pub last_event: u64,
    pub local: Option<Receipt>,
    pub openai: Option<Receipt>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Ledger {
    pub version: u32,
    pub guild: u64,
    pub members: BTreeMap<u64, MemberChoice>,
}

impl Ledger {
    pub fn new(guild: u64) -> Self {
        Self {
            version: 1,
            guild,
            members: BTreeMap::new(),
        }
    }

    pub fn agrees(&self, user: u64, mode: VoiceMode) -> bool {
        if mode == VoiceMode::Disabled {
            return true;
        }
        self.members
            .get(&user)
            .and_then(|member| match mode {
                VoiceMode::Local => member.local.as_ref(),
                VoiceMode::OpenAi => member.openai.as_ref(),
                VoiceMode::Disabled => None,
            })
            .is_some_and(|receipt| receipt.policy == POLICY_VERSION && receipt.interaction != 0)
    }

    pub fn apply(&mut self, user: u64, event: u64, choice: Choice, now: u64) -> bool {
        if user == 0
            || (event == 0 && choice != Choice::WithdrawSpoken)
            || choice == Choice::Agree(VoiceMode::Disabled)
        {
            return false;
        }
        let member = self.members.entry(user).or_default();
        let event = if choice == Choice::WithdrawSpoken {
            event.max(member.last_event.saturating_add(1))
        } else {
            event
        };
        if event <= member.last_event && choice != Choice::WithdrawSpoken {
            return false;
        }
        member.last_event = event;
        let receipt = Some(Receipt {
            policy: POLICY_VERSION,
            interaction: event,
            acknowledged_at: now,
        });
        match choice {
            Choice::Agree(VoiceMode::Local) => member.local = receipt,
            Choice::Agree(VoiceMode::OpenAi) => member.openai = receipt,
            Choice::Withdraw | Choice::WithdrawSpoken => {
                member.local = None;
                member.openai = None;
            }
            Choice::Agree(VoiceMode::Disabled) => unreachable!(),
        }
        true
    }
}

/// A local spoken withdrawal has no Discord event ID. Reject older Discord
/// choices through the end of this millisecond, using a caller-supplied clock.
pub fn withdrawal_watermark(unix_millis: u64) -> u64 {
    unix_millis
        .saturating_sub(1_420_070_400_000)
        .saturating_mul(1 << 22)
        | ((1 << 22) - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn choices_are_explicit_versioned_scoped_and_ordered() {
        let mut ledger = Ledger::new(10);
        assert!(!ledger.agrees(20, VoiceMode::Local));
        assert!(ledger.apply(20, 100, Choice::Agree(VoiceMode::Local), 1));
        assert!(ledger.agrees(20, VoiceMode::Local));
        assert!(!ledger.agrees(21, VoiceMode::Local));
        assert!(!ledger.agrees(20, VoiceMode::OpenAi));
        assert!(ledger.apply(20, 102, Choice::Withdraw, 2));
        assert!(!ledger.apply(20, 101, Choice::Agree(VoiceMode::OpenAi), 1));
        assert!(!ledger.agrees(20, VoiceMode::Local));
        assert!(!ledger.agrees(20, VoiceMode::OpenAi));
        assert!(ledger.apply(20, 103, Choice::Agree(VoiceMode::Local), 3));
        ledger
            .members
            .get_mut(&20)
            .unwrap()
            .local
            .as_mut()
            .unwrap()
            .policy = 0;
        assert!(!ledger.agrees(20, VoiceMode::Local));
        assert_eq!(parse_button("abbey:voice:agree:0:local"), None);
        assert_eq!(
            parse_button(button_id(VoiceMode::OpenAi).unwrap()),
            Some(Choice::Agree(VoiceMode::OpenAi))
        );
    }

    #[test]
    fn spoken_withdrawal_wins_even_when_local_clock_is_behind_discord() {
        let mut ledger = Ledger::new(10);
        ledger.apply(20, 999_999, Choice::Agree(VoiceMode::Local), 10);
        assert!(ledger.apply(20, 100, Choice::WithdrawSpoken, 9));
        assert!(!ledger.agrees(20, VoiceMode::Local));
        assert_eq!(ledger.members[&20].last_event, 1_000_000);
        assert!(!ledger.apply(20, 999_999, Choice::Agree(VoiceMode::OpenAi), 10));
    }

    #[test]
    fn consent_ledger_v1_wire_format_remains_byte_compatible() {
        let mut ledger = Ledger::new(10);
        assert!(ledger.apply(20, 100, Choice::Agree(VoiceMode::Local), 5));
        let encoded = serde_json::to_string(&ledger).expect("consent ledger serializes");
        assert_eq!(
            encoded,
            r#"{"version":1,"guild":10,"members":{"20":{"last_event":100,"local":{"policy":1,"interaction":100,"acknowledged_at":5},"openai":null}}}"#
        );
        let decoded: Ledger = serde_json::from_str(&encoded).expect("v1 ledger remains readable");
        assert!(decoded.agrees(20, VoiceMode::Local));
        assert_eq!(decoded.version, POLICY_VERSION);
    }

    #[test]
    fn complete_public_notice_and_private_status_fit_discord() {
        for mode in [VoiceMode::Local, VoiceMode::OpenAi, VoiceMode::Disabled] {
            let text = notice(mode, u64::MAX);
            assert!(text.chars().count() + 160 < 2000);
            assert!(text.contains("Notice version 1."));
            assert!(text.contains("Stop / withdraw"));
            assert!(text.contains("mention Abbey and type `stop listening`"));
        }
    }
}
