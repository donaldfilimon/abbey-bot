//! Pure classic-dashboard state, protocol, reducer, and rendering.

use crate::guild::GuildSettings;
use crate::persona::Persona;

pub const SESSION_SECONDS: u64 = 15 * 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdminPage {
    Overview,
    Conversation,
    Learning,
    Operations,
    ConfirmReset,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdminAction {
    View(AdminPage),
    SetLearning(bool),
    SetVision(bool),
    SetUnsolicited(bool),
    SetPersona(Persona),
    SetCooldown(u32),
    SetBudget(u32),
    Flush,
    Export,
    RequestReset,
    ConfirmReset,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdminEffect {
    View(AdminPage),
    SetLearning(bool),
    SetVision(bool),
    SetUnsolicited(bool),
    SetPersona(Persona),
    SetCooldown(u32),
    SetBudget(u32),
    Persist,
    Export,
    ResetChannel,
    None,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AdminViewInput {
    pub settings: GuildSettings,
    pub epsilon: f32,
    pub brain_summary: String,
    pub capabilities: Vec<&'static str>,
    pub operation_result: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdminSession {
    pub owner: u64,
    pub guild: u64,
    pub expiry: u64,
    pub page: AdminPage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rejection {
    Malformed,
    ForeignOwner,
    ForeignGuild,
    Expired,
}

impl Rejection {
    pub const fn message(self) -> &'static str {
        match self {
            Self::ForeignOwner => "This administration control belongs to someone else.",
            Self::ForeignGuild => "This administration control belongs to another server.",
            Self::Expired => "This administration control expired. Open `/admin dashboard` again.",
            Self::Malformed => {
                "This administration control is stale. Open `/admin dashboard` again."
            }
        }
    }
}

impl AdminAction {
    #[must_use]
    pub const fn slug(self) -> &'static str {
        match self {
            Self::View(AdminPage::Overview) => "view-overview",
            Self::View(AdminPage::Conversation) => "view-conversation",
            Self::View(AdminPage::Learning) => "view-learning",
            Self::View(AdminPage::Operations) => "view-operations",
            Self::View(AdminPage::ConfirmReset) | Self::RequestReset => "request-reset",
            Self::SetLearning(true) => "learning-on",
            Self::SetLearning(false) => "learning-off",
            Self::SetVision(true) => "vision-on",
            Self::SetVision(false) => "vision-off",
            Self::SetUnsolicited(true) => "act-on",
            Self::SetUnsolicited(false) => "act-off",
            Self::SetPersona(Persona::Abbey) => "persona-abbey",
            Self::SetPersona(Persona::Aviva) => "persona-aviva",
            Self::SetPersona(Persona::Abi) => "persona-abi",
            Self::SetCooldown(0) => "cooldown-0",
            Self::SetCooldown(20) => "cooldown-20",
            Self::SetCooldown(_) => "cooldown-60",
            Self::SetBudget(1) => "budget-1",
            Self::SetBudget(6) => "budget-6",
            Self::SetBudget(_) => "budget-60",
            Self::Flush => "flush",
            Self::Export => "export",
            Self::ConfirmReset => "confirm-reset",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "view-overview" => Self::View(AdminPage::Overview),
            "view-conversation" => Self::View(AdminPage::Conversation),
            "view-learning" => Self::View(AdminPage::Learning),
            "view-operations" => Self::View(AdminPage::Operations),
            "request-reset" => Self::RequestReset,
            "learning-on" => Self::SetLearning(true),
            "learning-off" => Self::SetLearning(false),
            "vision-on" => Self::SetVision(true),
            "vision-off" => Self::SetVision(false),
            "act-on" => Self::SetUnsolicited(true),
            "act-off" => Self::SetUnsolicited(false),
            "persona-abbey" => Self::SetPersona(Persona::Abbey),
            "persona-aviva" => Self::SetPersona(Persona::Aviva),
            "persona-abi" => Self::SetPersona(Persona::Abi),
            "cooldown-0" => Self::SetCooldown(0),
            "cooldown-20" => Self::SetCooldown(20),
            "cooldown-60" => Self::SetCooldown(60),
            "budget-1" => Self::SetBudget(1),
            "budget-6" => Self::SetBudget(6),
            "budget-60" => Self::SetBudget(60),
            "flush" => Self::Flush,
            "export" => Self::Export,
            "confirm-reset" => Self::ConfirmReset,
            _ => return None,
        })
    }
}

impl AdminSession {
    #[must_use]
    pub fn custom_id(&self, action: AdminAction) -> String {
        let id = format!(
            "abbey:admin:v1:{}:{}:{}:{}",
            self.owner,
            self.guild,
            self.expiry,
            action.slug()
        );
        debug_assert!(id.is_ascii() && id.len() <= 100);
        id
    }

    pub fn parse(
        id: &str,
        actor: u64,
        guild: Option<u64>,
        now: u64,
    ) -> Result<(Self, AdminAction), Rejection> {
        let mut parts = id.split(':');
        if parts.next() != Some("abbey")
            || parts.next() != Some("admin")
            || parts.next() != Some("v1")
        {
            return Err(Rejection::Malformed);
        }
        let owner = parts
            .next()
            .and_then(|v| v.parse().ok())
            .ok_or(Rejection::Malformed)?;
        let session_guild = parts
            .next()
            .and_then(|v| v.parse().ok())
            .ok_or(Rejection::Malformed)?;
        let expiry = parts
            .next()
            .and_then(|v| v.parse().ok())
            .ok_or(Rejection::Malformed)?;
        let action = parts
            .next()
            .filter(|_| parts.next().is_none())
            .and_then(AdminAction::parse)
            .ok_or(Rejection::Malformed)?;
        if owner != actor {
            return Err(Rejection::ForeignOwner);
        }
        if guild != Some(session_guild) {
            return Err(Rejection::ForeignGuild);
        }
        if now > expiry {
            return Err(Rejection::Expired);
        }
        let page = match action {
            AdminAction::View(page) => page,
            AdminAction::RequestReset | AdminAction::ConfirmReset => AdminPage::ConfirmReset,
            AdminAction::SetLearning(_) => AdminPage::Learning,
            AdminAction::SetVision(_)
            | AdminAction::SetUnsolicited(_)
            | AdminAction::SetPersona(_)
            | AdminAction::SetCooldown(_) => AdminPage::Conversation,
            AdminAction::SetBudget(_) => AdminPage::Learning,
            AdminAction::Flush | AdminAction::Export => AdminPage::Operations,
        };
        Ok((
            Self {
                owner,
                guild: session_guild,
                expiry,
                page,
            },
            action,
        ))
    }
}

#[must_use]
pub fn reduce(action: AdminAction, settings: &GuildSettings) -> AdminEffect {
    match action {
        AdminAction::View(page) => AdminEffect::View(page),
        AdminAction::RequestReset => AdminEffect::View(AdminPage::ConfirmReset),
        AdminAction::SetLearning(value) if settings.learning_enabled != value => {
            AdminEffect::SetLearning(value)
        }
        AdminAction::SetVision(value) if settings.vision_enabled != value => {
            AdminEffect::SetVision(value)
        }
        AdminAction::SetUnsolicited(value) if settings.unsolicited != value => {
            AdminEffect::SetUnsolicited(value)
        }
        AdminAction::SetPersona(value) if settings.default_persona != value => {
            AdminEffect::SetPersona(value)
        }
        AdminAction::SetCooldown(value)
            if settings.reply_cooldown_seconds
                != crate::guild::clamp_cooldown(i64::from(value)) =>
        {
            AdminEffect::SetCooldown(crate::guild::clamp_cooldown(i64::from(value)))
        }
        AdminAction::SetBudget(value)
            if settings.unsolicited_per_hour != crate::guild::clamp_budget(i64::from(value)) =>
        {
            AdminEffect::SetBudget(crate::guild::clamp_budget(i64::from(value)))
        }
        AdminAction::SetLearning(_)
        | AdminAction::SetVision(_)
        | AdminAction::SetUnsolicited(_)
        | AdminAction::SetPersona(_)
        | AdminAction::SetCooldown(_)
        | AdminAction::SetBudget(_) => AdminEffect::None,
        AdminAction::Flush => AdminEffect::Persist,
        AdminAction::Export => AdminEffect::Export,
        AdminAction::ConfirmReset => AdminEffect::ResetChannel,
    }
}

#[must_use]
pub fn render(page: AdminPage, input: &AdminViewInput) -> String {
    let result = input
        .operation_result
        .as_ref()
        .map_or(String::new(), |value| format!("\n\n{value}"));
    match page {
        AdminPage::Overview => format!(
            "**Administration · Overview**\nPersona: **{}**\nLearning: **{}** · Vision: **{}** · Unsolicited: **{}**\nCapabilities: {}{result}",
            crate::guild::persona_name(input.settings.default_persona),
            on_off(input.settings.learning_enabled),
            on_off(input.settings.vision_enabled),
            on_off(input.settings.unsolicited),
            input.capabilities.join(", ")
        ),
        AdminPage::Conversation => format!(
            "**Administration · Conversation**\nDefault persona: **{}**\nVision: **{}**\nUnsolicited action: **{}**\nCooldown: **{}s**{result}",
            crate::guild::persona_name(input.settings.default_persona),
            on_off(input.settings.vision_enabled),
            on_off(input.settings.unsolicited),
            input.settings.reply_cooldown_seconds,
        ),
        AdminPage::Learning => format!(
            "**Administration · Learning**\nLearning: **{}**\nHourly budget: **{}/h**\nEpsilon: **{:.3}**\n{}{result}",
            on_off(input.settings.learning_enabled),
            input.settings.unsolicited_per_hour,
            input.epsilon,
            input.brain_summary,
        ),
        AdminPage::Operations => format!(
            "**Administration · Operations**\nFlush reports the actual canonical and WDBX persistence outcomes. Export is a private attachment. Reset requires a second interaction and affects this channel's transcript only.{result}"
        ),
        AdminPage::ConfirmReset => format!(
            "**Administration · Confirm reset**\nConfirming clears only this channel's multi-turn transcript. Facts, policy, reputation, other channels, and other servers remain intact.{result}"
        ),
    }
}

const fn on_off(value: bool) -> &'static str {
    if value { "on" } else { "off" }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_binds_owner_guild_expiry_and_stays_within_discord_limit() {
        let session = AdminSession {
            owner: u64::MAX,
            guild: u64::MAX,
            expiry: u64::MAX,
            page: AdminPage::Overview,
        };
        let id = session.custom_id(AdminAction::ConfirmReset);
        assert!(id.is_ascii());
        assert!(id.len() <= 100);
        assert_eq!(
            AdminSession::parse(&id, u64::MAX, Some(u64::MAX), u64::MAX)
                .unwrap()
                .1,
            AdminAction::ConfirmReset
        );
        assert_eq!(
            AdminSession::parse(&id, 1, Some(u64::MAX), 0),
            Err(Rejection::ForeignOwner)
        );
        assert_eq!(
            AdminSession::parse(&id, u64::MAX, Some(1), 0),
            Err(Rejection::ForeignGuild)
        );
    }

    #[test]
    fn desired_state_actions_are_idempotent_against_current_settings() {
        let settings = GuildSettings {
            learning_enabled: true,
            ..GuildSettings::default()
        };
        assert_eq!(
            reduce(AdminAction::SetLearning(true), &settings),
            AdminEffect::None
        );
        assert_eq!(
            reduce(AdminAction::SetLearning(false), &settings),
            AdminEffect::SetLearning(false)
        );
    }

    #[test]
    fn reset_requires_navigation_then_a_distinct_confirmation() {
        let settings = GuildSettings::default();
        assert_eq!(
            reduce(AdminAction::RequestReset, &settings),
            AdminEffect::View(AdminPage::ConfirmReset)
        );
        assert_eq!(
            reduce(AdminAction::ConfirmReset, &settings),
            AdminEffect::ResetChannel
        );
    }
}
