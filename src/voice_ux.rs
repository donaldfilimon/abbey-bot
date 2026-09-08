//! Pure classic voice Action Row protocol: custom ids, session bind, phase reduce.
//!
//! Presentation and Discord I/O live in `commands_voice::ux`. This module never
//! starts STT, never acts as consent, and never embeds guild/user in custom ids.

use serde::{Deserialize, Serialize};

pub const SESSION_SECONDS: u64 = 15 * 60;
pub const CUSTOM_ID_PREFIX: &str = "abbey:v:";
const SID_ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz234567";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Status,
    ConfirmLeave,
    Left,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Act {
    Ref,
    Leave,
    Ok,
    Cancel,
    Play,
    Stop,
    Skip,
}

impl Act {
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::Ref => "ref",
            Self::Leave => "leave",
            Self::Ok => "ok",
            Self::Cancel => "cancel",
            Self::Play => "play",
            Self::Stop => "stop",
            Self::Skip => "skip",
        }
    }

    fn parse(token: &str) -> Option<Self> {
        Some(match token {
            "ref" => Self::Ref,
            "leave" => Self::Leave,
            "ok" => Self::Ok,
            "cancel" => Self::Cancel,
            "play" => Self::Play,
            "stop" => Self::Stop,
            "skip" => Self::Skip,
            _ => return None,
        })
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Ref => "Refresh",
            Self::Leave => "Leave",
            Self::Ok => "Confirm leave",
            Self::Cancel => "Cancel",
            Self::Play => "Play",
            Self::Stop => "Stop",
            Self::Skip => "Skip",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rejection {
    Malformed,
    Missing,
    ForeignOwner,
    ForeignGuild,
    Expired,
    WrongPhase,
}

impl Rejection {
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::ForeignOwner => "These voice controls belong to someone else.",
            Self::ForeignGuild => "These voice controls belong to another server.",
            Self::Expired => "These voice controls expired. Use `/voice join consent:true` again.",
            Self::Missing => "These voice controls are gone. Use `/voice status` or join again.",
            Self::WrongPhase => {
                "That voice control is out of date. Tap Refresh or reopen the panel."
            }
            Self::Malformed => "That voice control is stale. Use `/voice join consent:true` again.",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    pub sid: String,
    pub guild: u64,
    pub user: u64,
    pub channel: u64,
    pub expiry: u64,
    pub phase: Phase,
}

/// Mint an opaque sid short enough for Discord's 100-character custom id limit.
#[must_use]
pub fn mint_sid() -> String {
    let mut bytes = [0u8; 10];
    if getrandom::fill(&mut bytes).is_err() {
        // Fail soft to a time-derived id rather than panicking in production.
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let le = now.to_le_bytes();
        for (idx, slot) in bytes.iter_mut().enumerate() {
            *slot = le[idx % le.len()];
        }
    }
    encode_sid(&bytes)
}

fn encode_sid(bytes: &[u8; 10]) -> String {
    let mut out = String::with_capacity(16);
    // 10 bytes → 16 base32 chars (80 bits).
    let mut buffer = 0u64;
    let mut bits = 0u32;
    for &byte in bytes {
        buffer = (buffer << 8) | u64::from(byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            let idx = ((buffer >> bits) & 31) as usize;
            out.push(SID_ALPHABET[idx] as char);
        }
    }
    if bits > 0 {
        let idx = ((buffer << (5 - bits)) & 31) as usize;
        out.push(SID_ALPHABET[idx] as char);
    }
    out
}

#[must_use]
pub fn format_custom_id(sid: &str, act: Act) -> String {
    let id = format!("{CUSTOM_ID_PREFIX}{sid}:{}", act.token());
    debug_assert!(id.is_ascii() && id.len() <= 100);
    id
}

pub fn parse_custom_id(id: &str) -> Result<(String, Act), Rejection> {
    let rest = id
        .strip_prefix(CUSTOM_ID_PREFIX)
        .ok_or(Rejection::Malformed)?;
    let (sid, act) = rest.split_once(':').ok_or(Rejection::Malformed)?;
    if sid.is_empty()
        || !sid.is_ascii()
        || !sid
            .bytes()
            .all(|b| b.is_ascii_lowercase() || (b'2'..=b'7').contains(&b))
        || act.is_empty()
        || rest.matches(':').count() != 1
    {
        return Err(Rejection::Malformed);
    }
    let act = Act::parse(act).ok_or(Rejection::Malformed)?;
    Ok((sid.to_owned(), act))
}

pub fn authorize(
    session: &Session,
    actor: u64,
    guild: Option<u64>,
    now: u64,
) -> Result<(), Rejection> {
    if actor != session.user {
        return Err(Rejection::ForeignOwner);
    }
    match guild {
        Some(guild) if guild == session.guild => {}
        _ => return Err(Rejection::ForeignGuild),
    }
    if now >= session.expiry {
        return Err(Rejection::Expired);
    }
    Ok(())
}

pub fn reduce(phase: Phase, act: Act) -> Result<Phase, Rejection> {
    match (phase, act) {
        (Phase::Status, Act::Ref) => Ok(Phase::Status),
        (Phase::Status, Act::Leave) => Ok(Phase::ConfirmLeave),
        (Phase::Status, Act::Play | Act::Stop | Act::Skip) => Ok(Phase::Status),
        (Phase::ConfirmLeave, Act::Ok) => Ok(Phase::Left),
        (Phase::ConfirmLeave, Act::Cancel) => Ok(Phase::Status),
        (Phase::Left, Act::Ref) => Ok(Phase::Left),
        _ => Err(Rejection::WrongPhase),
    }
}

/// Live registry projection used for Play/Stop/Skip enablement.
#[must_use]
pub fn playable(
    has_runtime: bool,
    media_enabled: bool,
    phase_failed: bool,
    start_pending: bool,
) -> bool {
    has_runtime && media_enabled && !phase_failed && !start_pending
}

#[must_use]
pub fn status_buttons(is_playable: bool) -> &'static [Act] {
    if is_playable {
        &[Act::Ref, Act::Leave, Act::Play, Act::Stop, Act::Skip]
    } else {
        &[Act::Ref, Act::Leave]
    }
}

#[must_use]
pub fn confirm_buttons() -> &'static [Act] {
    &[Act::Ok, Act::Cancel]
}

#[must_use]
pub fn panel_content(phase: Phase, status_body: &str, is_playable: bool) -> String {
    match phase {
        Phase::Status => {
            let play = if is_playable {
                "Play/Stop/Skip are available while Abbey can transmit music."
            } else {
                "Play controls stay off until a playable voice session is active."
            };
            format!(
                "{status_body}\n\n{play}\nRefresh re-reads live voice state. Leave asks for confirmation before disconnecting."
            )
        }
        Phase::ConfirmLeave => {
            "Confirm leave? This stops capture, provider work, queued audio, and playback."
                .to_owned()
        }
        Phase::Left => {
            "Left voice. Capture, provider work, queued audio, and playback are stopped.".to_owned()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Session {
        Session {
            sid: "abcdefghijklmnop".into(),
            guild: 10,
            user: 20,
            channel: 30,
            expiry: 1_000,
            phase: Phase::Status,
        }
    }

    #[test]
    fn custom_id_round_trips_every_act() {
        for act in [
            Act::Ref,
            Act::Leave,
            Act::Ok,
            Act::Cancel,
            Act::Play,
            Act::Stop,
            Act::Skip,
        ] {
            let id = format_custom_id("abcdefghijklmnop", act);
            assert!(id.len() <= 100);
            assert_eq!(
                parse_custom_id(&id).unwrap(),
                ("abcdefghijklmnop".into(), act)
            );
        }
    }

    #[test]
    fn mint_sid_is_short_ascii_custom_id_safe() {
        let sid = mint_sid();
        assert!(sid.len() <= 16);
        assert!(
            sid.bytes()
                .all(|b| b.is_ascii_lowercase() || (b'2'..=b'7').contains(&b))
        );
        let id = format_custom_id(&sid, Act::Leave);
        assert!(id.is_ascii() && id.len() <= 100);
        assert_eq!(parse_custom_id(&id).unwrap().0, sid);
    }

    #[test]
    fn malformed_and_unknown_acts_fail_closed() {
        for id in [
            "abbey:admin:v1:1:1:1:flush",
            "abbey:voice:withdraw",
            "abbey:v:",
            "abbey:v::ref",
            "abbey:v:ABC:ref",
            "abbey:v:abcdefghijklmnop:",
            "abbey:v:abcdefghijklmnop:nope",
            "abbey:v:abcdefghijklmnop:ref:extra",
            "abbey:v:abcdefghijklmnop:REF",
        ] {
            assert_eq!(parse_custom_id(id), Err(Rejection::Malformed), "{id}");
        }
    }

    #[test]
    fn authorize_denies_wrong_user_guild_expired() {
        let session = sample();
        assert_eq!(
            authorize(&session, 99, Some(10), 500),
            Err(Rejection::ForeignOwner)
        );
        assert_eq!(
            authorize(&session, 20, Some(11), 500),
            Err(Rejection::ForeignGuild)
        );
        assert_eq!(
            authorize(&session, 20, None, 500),
            Err(Rejection::ForeignGuild)
        );
        assert_eq!(
            authorize(&session, 20, Some(10), 1_000),
            Err(Rejection::Expired)
        );
        assert!(authorize(&session, 20, Some(10), 999).is_ok());
    }

    #[test]
    fn reduce_status_leave_confirm_cancel_and_left() {
        assert_eq!(
            reduce(Phase::Status, Act::Leave).unwrap(),
            Phase::ConfirmLeave
        );
        assert_eq!(
            reduce(Phase::ConfirmLeave, Act::Cancel).unwrap(),
            Phase::Status
        );
        assert_eq!(reduce(Phase::ConfirmLeave, Act::Ok).unwrap(), Phase::Left);
        assert_eq!(reduce(Phase::Status, Act::Ref).unwrap(), Phase::Status);
        assert_eq!(reduce(Phase::Left, Act::Ref).unwrap(), Phase::Left);
        assert_eq!(
            reduce(Phase::ConfirmLeave, Act::Leave),
            Err(Rejection::WrongPhase)
        );
        assert_eq!(reduce(Phase::Status, Act::Ok), Err(Rejection::WrongPhase));
        assert_eq!(reduce(Phase::Left, Act::Leave), Err(Rejection::WrongPhase));
    }

    #[test]
    fn play_acts_only_reduce_on_status_phase() {
        for act in [Act::Play, Act::Stop, Act::Skip] {
            assert_eq!(reduce(Phase::Status, act).unwrap(), Phase::Status);
            assert_eq!(reduce(Phase::ConfirmLeave, act), Err(Rejection::WrongPhase));
            assert_eq!(reduce(Phase::Left, act), Err(Rejection::WrongPhase));
        }
    }

    #[test]
    fn playable_matrix_and_status_buttons() {
        assert!(!playable(false, true, false, false));
        assert!(!playable(true, false, false, false));
        assert!(!playable(true, true, true, false));
        assert!(!playable(true, true, false, true));
        assert!(playable(true, true, false, false));
        assert_eq!(status_buttons(false), &[Act::Ref, Act::Leave]);
        assert_eq!(
            status_buttons(true),
            &[Act::Ref, Act::Leave, Act::Play, Act::Stop, Act::Skip]
        );
        assert_eq!(confirm_buttons(), &[Act::Ok, Act::Cancel]);
    }

    #[test]
    fn leave_confirm_once_cancel_restores_without_left() {
        // Pure transition trace: Leave → Cancel never reaches Left; only Ok does.
        let mut phase = Phase::Status;
        phase = reduce(phase, Act::Leave).unwrap();
        phase = reduce(phase, Act::Cancel).unwrap();
        assert_eq!(phase, Phase::Status);
        phase = reduce(phase, Act::Leave).unwrap();
        phase = reduce(phase, Act::Ok).unwrap();
        assert_eq!(phase, Phase::Left);
        // Refresh on status never leaves.
        assert_eq!(reduce(Phase::Status, Act::Ref).unwrap(), Phase::Status);
    }
}
