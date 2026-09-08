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

impl AdminPage {
    /// Navigable dashboard pages exposed by the classic page select.
    pub const NAV: [Self; 4] = [
        Self::Overview,
        Self::Conversation,
        Self::Learning,
        Self::Operations,
    ];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Overview => "Overview",
            Self::Conversation => "Conversation",
            Self::Learning => "Learning",
            Self::Operations => "Operations",
            Self::ConfirmReset => "Confirm reset",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdminAction {
    /// Classic String Select sentinel: option values carry `View(...)` slugs.
    SelectPage,
    View(AdminPage),
    SetLearning(bool),
    SetVision(bool),
    SetUnsolicited(bool),
    SetPersona(Persona),
    SetCooldown(u32),
    SetBudget(u32),
    SetEpsilon(u16),
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
    SetEpsilon(u16),
    Persist,
    Export,
    ResetChannel,
    None,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AdminViewInput {
    pub settings: GuildSettings,
    pub effective_policy: String,
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
            Self::SelectPage => "page-select",
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
            Self::SetEpsilon(5) => "epsilon-5",
            Self::SetEpsilon(20) => "epsilon-20",
            Self::SetEpsilon(_) => "epsilon-50",
            Self::Flush => "flush",
            Self::Export => "export",
            Self::ConfirmReset => "confirm-reset",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "page-select" => Self::SelectPage,
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
            "epsilon-5" => Self::SetEpsilon(5),
            "epsilon-20" => Self::SetEpsilon(20),
            "epsilon-50" => Self::SetEpsilon(50),
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
            AdminAction::SelectPage => AdminPage::Overview,
            AdminAction::RequestReset | AdminAction::ConfirmReset => AdminPage::ConfirmReset,
            AdminAction::SetLearning(_) => AdminPage::Learning,
            AdminAction::SetVision(_)
            | AdminAction::SetUnsolicited(_)
            | AdminAction::SetPersona(_)
            | AdminAction::SetCooldown(_) => AdminPage::Conversation,
            AdminAction::SetBudget(_) | AdminAction::SetEpsilon(_) => AdminPage::Learning,
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

/// Resolve a classic String Select choice into a concrete admin action.
///
/// The select menu custom id carries [`AdminAction::SelectPage`]; the chosen
/// option value must be a navigable `View(...)` slug. Anything else fails closed.
#[must_use]
pub fn resolve_select_action(
    action: AdminAction,
    values: &[String],
) -> Result<AdminAction, Rejection> {
    match action {
        AdminAction::SelectPage => match values {
            [value] => match AdminAction::parse(value) {
                Some(resolved @ AdminAction::View(page)) if AdminPage::NAV.contains(&page) => {
                    Ok(resolved)
                }
                _ => Err(Rejection::Malformed),
            },
            _ => Err(Rejection::Malformed),
        },
        _ => Err(Rejection::Malformed),
    }
}

#[must_use]
pub fn reduce(action: AdminAction, settings: &GuildSettings) -> AdminEffect {
    match action {
        AdminAction::View(page) => AdminEffect::View(page),
        // SelectPage is remapped via resolve_select_action before reduce.
        AdminAction::SelectPage => AdminEffect::None,
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
        AdminAction::SetEpsilon(value)
            if settings.epsilon_override
                != Some(crate::guild::clamp_epsilon(f64::from(value) / 100.0)) =>
        {
            AdminEffect::SetEpsilon(value)
        }
        AdminAction::SetLearning(_)
        | AdminAction::SetVision(_)
        | AdminAction::SetUnsolicited(_)
        | AdminAction::SetPersona(_)
        | AdminAction::SetCooldown(_)
        | AdminAction::SetBudget(_)
        | AdminAction::SetEpsilon(_) => AdminEffect::None,
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
    let result = format!("\n{}{}", input.effective_policy, result);
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

/// Requested guild policy and the blockers enforced before ordinary unsolicited replies.
#[must_use]
pub fn unsolicited_status(settings: &GuildSettings, quiet: bool, generation: bool) -> String {
    let mut blockers = Vec::new();
    if quiet {
        blockers.push("host quiet is on; ask the host operator to review it");
    }
    if !settings.unsolicited {
        blockers.push("server opt-in is off (`/admin act on`)");
    }
    if !settings.learning_enabled {
        blockers.push("learning is off (`/admin learning on`)");
    }
    if !generation {
        blockers.push("generation provider is unavailable; ask the operator to check readiness");
    }
    let effective = if blockers.is_empty() {
        "eligible for policy selection; hourly budget and channel cooldown still apply".into()
    } else {
        format!("blocked: {}", blockers.join("; "))
    };
    format!(
        "Unsolicited requested: {} · Effective replies: {effective}. Budget: {}/h; cooldown: {}s (checked when acting).",
        on_off(settings.unsolicited),
        settings.unsolicited_per_hour,
        settings.reply_cooldown_seconds
    )
}

#[must_use]
pub fn vision_status(enabled: bool, description: bool, ocr: bool) -> String {
    let readiness = |available| {
        if !enabled {
            "blocked by server setting (`/admin vision on`)"
        } else if available {
            "provider eligible; checked again on use"
        } else {
            "provider unavailable"
        }
    };
    format!(
        "Vision requested: {} · Description: {} · OCR: {}",
        on_off(enabled),
        readiness(description),
        readiness(ocr)
    )
}

const fn on_off(value: bool) -> &'static str {
    if value { "on" } else { "off" }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vision_policy_and_operation_readiness_remain_distinct() {
        assert!(vision_status(false, true, true).contains("blocked by server setting"));
        let text = vision_status(true, true, false);
        assert!(text.contains("Description: provider eligible"));
        assert!(text.contains("OCR: provider unavailable"));
    }

    #[test]
    fn requested_act_on_exposes_learning_and_quiet_blockers() {
        let settings = GuildSettings {
            unsolicited: true,
            learning_enabled: false,
            ..GuildSettings::default()
        };
        let text = unsolicited_status(&settings, true, false);
        assert!(text.contains("requested: on"));
        assert!(text.contains("learning is off"));
        assert!(text.contains("host quiet"));
        assert!(text.contains("provider"));
    }

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
        let settings = GuildSettings {
            epsilon_override: Some(crate::guild::clamp_epsilon(0.2)),
            ..settings
        };
        assert_eq!(
            reduce(AdminAction::SetEpsilon(20), &settings),
            AdminEffect::None
        );
        assert_eq!(
            reduce(AdminAction::SetEpsilon(50), &settings),
            AdminEffect::SetEpsilon(50)
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

    #[test]
    fn page_select_resolves_only_navigable_views() {
        let session = AdminSession {
            owner: 7,
            guild: 8,
            expiry: 9,
            page: AdminPage::Overview,
        };
        let id = session.custom_id(AdminAction::SelectPage);
        assert!(id.len() <= 100);
        assert_eq!(
            AdminSession::parse(&id, 7, Some(8), 9).unwrap().1,
            AdminAction::SelectPage
        );
        assert_eq!(
            resolve_select_action(
                AdminAction::SelectPage,
                &["view-conversation".into()]
            )
            .unwrap(),
            AdminAction::View(AdminPage::Conversation)
        );
        assert_eq!(
            resolve_select_action(AdminAction::SelectPage, &["confirm-reset".into()]),
            Err(Rejection::Malformed)
        );
        assert_eq!(
            resolve_select_action(AdminAction::SelectPage, &[]),
            Err(Rejection::Malformed)
        );
        assert_eq!(
            resolve_select_action(AdminAction::Flush, &["view-overview".into()]),
            Err(Rejection::Malformed)
        );
        assert_eq!(reduce(AdminAction::SelectPage, &GuildSettings::default()), AdminEffect::None);
    }
}
