//! Owner-only pending proposal review and fresh approval/rejection boundary.
use super::*;
pub(super) fn dashboard_rows_with_mode_controls(
    session: &crate::admin_dashboard::AdminSession,
    current_owner: bool,
) -> Vec<CreateActionRow> {
    let mut rows = dashboard_rows(session);
    if session.page == crate::admin_dashboard::AdminPage::AutonomousOperations && current_owner {
        use crate::{admin_dashboard::AdminAction as A, community_ops::Mode};
        rows.push(CreateActionRow::Buttons(
            [Mode::Stopped, Mode::Propose, Mode::Apply]
                .into_iter()
                .map(|mode| {
                    CreateButton::new(session.custom_id(A::SetCommunityMode(mode)))
                        .label(match mode {
                            Mode::Stopped => "Mode: stopped",
                            Mode::Propose => "Mode: propose",
                            Mode::Apply => "Mode: apply",
                        })
                        .style(ButtonStyle::Secondary)
                })
                .chain([CreateButton::new(
                    session.custom_id(crate::admin_dashboard::AdminAction::ReviewProposals),
                )
                .label("Review proposals")
                .style(ButtonStyle::Primary)])
                .collect(),
        ));
    }
    rows
}

pub(super) fn mode_owner_matches(
    actor: u64,
    guild: u64,
    discord_owner: u64,
    policy_guild: u64,
    policy_owner: u64,
) -> bool {
    actor == discord_owner && guild == policy_guild && actor == policy_owner
}

pub(super) async fn mode_controls_authorized(
    http: &serenity::all::Http,
    state: &AppState,
    session: &crate::admin_dashboard::AdminSession,
) -> bool {
    if session.page != crate::admin_dashboard::AdminPage::AutonomousOperations {
        return false;
    }
    let Ok(guild) = serenity::all::GuildId::new(session.guild)
        .to_partial_guild(http)
        .await
    else {
        return false;
    };
    let Some(path) = std::env::var_os("ABBEY_COMMUNITY_POLICY") else {
        return false;
    };
    let Ok((policy, _)) = state.community_policy(PathBuf::from(path)).await else {
        return false;
    };
    mode_owner_matches(
        session.owner,
        session.guild,
        guild.owner_id.get(),
        policy.guild,
        policy.owner,
    )
}

pub(super) async fn apply_community_mode(
    http: &serenity::all::Http,
    state: &AppState,
    session: &crate::admin_dashboard::AdminSession,
    mode: crate::community_ops::Mode,
) -> Result<String, &'static str> {
    // Always refresh ownership; displayed controls and Manage Server are not authority.
    let guild = serenity::all::GuildId::new(session.guild)
        .to_partial_guild(http)
        .await
        .map_err(|_| "Current guild owner could not be confirmed. Mode unchanged.")?;
    let path = std::env::var_os("ABBEY_COMMUNITY_POLICY")
        .ok_or("No owner-authored policy is configured. Mode unchanged.")?;
    let path = PathBuf::from(path);
    let (policy, digest) = state.community_policy(path.clone()).await?;
    if !mode_owner_matches(
        session.owner,
        session.guild,
        guild.owner_id.get(),
        policy.guild,
        policy.owner,
    ) {
        return Err(
            "Only the current guild owner matching the policy may change its mode. Mode unchanged.",
        );
    }
    if policy.mode == mode {
        return Ok(format!(
            "Policy mode is already {mode:?}. No new actions approved."
        ));
    }
    let (guild, owner) = (session.guild, session.owner);
    state
        .community_filesystem(move || {
            crate::persist::community_ops::set_mode(&path, &digest, guild, owner, mode)
        })
        .await?;
    Ok(format!(
        "Saved policy mode: {mode:?}. This changes mode only; no new actions were approved. Execution remains guarded."
    ))
}

use crate::community_ops::Policy;
use crate::community_ops::proposals::{
    FreshOperationProof, FreshOwnerProof, PendingProposal, ProposalStatus, ProposalStore,
    scope_digest,
};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

const PREFIX: &str = "abbey:admin:proposal:";
const MAX_SESSIONS: usize = 128;

/// Full reviewed identity stays server-side; Discord IDs have a 100-byte limit.
/// Tokens grant no authority. Restart, expiry, owner/origin drift and replay all fail closed.
#[derive(Clone)]
struct ReviewBinding {
    session: crate::admin_dashboard::AdminSession,
    origin: u64,
    message: u64,
    policy_digest: String,
    store_revision: u64,
    proposal_id: String,
    proposal_revision: u64,
    reviewed_hash: String,
    index: usize,
}
impl ReviewBinding {
    fn check(
        &self,
        actor: u64,
        guild: Option<u64>,
        origin: u64,
        message: u64,
        now: u64,
    ) -> Result<(), &'static str> {
        if actor != self.session.owner
            || guild != Some(self.session.guild)
            || origin != self.origin
            || message != self.message
        {
            return Err("This proposal review belongs to another owner, guild or channel.");
        }
        if now > self.session.expiry {
            return Err("This proposal review expired. Open Autonomous Operations again.");
        }
        Ok(())
    }
    fn check_snapshot(&self, digest: &str, store: &ProposalStore) -> Result<(), &'static str> {
        if digest != self.policy_digest || store.revision != self.store_revision {
            return Err("Policy or proposal inventory changed. Review the current proposal again.");
        }
        let p = store
            .proposals
            .get(&self.proposal_id)
            .ok_or("Reviewed proposal is unavailable.")?;
        p.validate()?;
        if p.hash != self.reviewed_hash || p.revision != self.proposal_revision {
            return Err("Reviewed proposal changed. Review its current operation again.");
        }
        Ok(())
    }
}
#[derive(Default)]
struct ReviewSessions {
    bindings: BTreeMap<String, ReviewBinding>,
}
impl ReviewSessions {
    fn insert(
        &mut self,
        token: String,
        binding: ReviewBinding,
        now: u64,
    ) -> Result<(), &'static str> {
        self.bindings.retain(|_, b| b.session.expiry >= now);
        if self.bindings.len() >= MAX_SESSIONS || self.bindings.contains_key(&token) {
            return Err(
                "Proposal review capacity is busy. Reopen the dashboard after existing sessions expire.",
            );
        }
        self.bindings.insert(token, binding);
        Ok(())
    }
    fn take(
        &mut self,
        token: &str,
        actor: u64,
        guild: Option<u64>,
        origin: u64,
        message: u64,
        now: u64,
    ) -> Result<ReviewBinding, &'static str> {
        let binding = self
            .bindings
            .get(token)
            .ok_or("Proposal review expired or was already used. Reopen the dashboard.")?;
        // An unrelated actor cannot consume the owner's valid control.
        binding.check(actor, guild, origin, message, now)?;
        self.bindings
            .remove(token)
            .ok_or("Proposal review unavailable.")
    }
}
static REVIEWS: OnceLock<Mutex<ReviewSessions>> = OnceLock::new();
fn reviews() -> &'static Mutex<ReviewSessions> {
    REVIEWS.get_or_init(|| Mutex::new(ReviewSessions::default()))
}
fn policy_path() -> Result<PathBuf, &'static str> {
    std::env::var_os("ABBEY_COMMUNITY_POLICY")
        .map(PathBuf::from)
        .ok_or("No owner-authored policy is configured.")
}
fn data_path(data: &crate::Data) -> Result<&Path, &'static str> {
    data.state
        .data_dir
        .as_deref()
        .ok_or("Durable community operations storage is unavailable.")
}
async fn current_policy(
    state: &AppState,
    guild: u64,
) -> Result<(PathBuf, Policy, String), &'static str> {
    let path = policy_path()?;
    let (policy, digest) = state.community_policy(path.clone()).await?;
    if policy.guild != guild {
        return Err("Configured policy targets another guild.");
    }
    Ok((path, policy, digest))
}

pub(super) async fn status(data: &crate::Data, guild: u64) -> String {
    let local = data.state.providers.local_only();
    let ready = data
        .state
        .providers
        .request_readiness(crate::provider::RequestClass::TextReadOnly)
        .is_ok();
    let locality = format!(
        "Assessment local-only: {local}; qualified text route eligible: {ready} (rechecked before generation)."
    );
    let Ok((_, policy, _)) = current_policy(&data.state, guild).await else {
        return format!(
            "Requested: stopped or policy unavailable. Effective operations disabled.\n{locality}"
        );
    };
    let base = format!(
        "Mode: {:?}. Daily change/creation ceilings: {}/{}. Approved inventory: {}. Assessment opt-in: {}.\n{locality}",
        policy.mode,
        policy.daily_limit,
        policy.daily_creations,
        policy.actions.len(),
        policy.assessment.enabled
    );
    let store = match data_path(data) {
        Ok(path) => {
            let path = path.to_owned();
            data.state
                .community_filesystem(move || crate::persist::community_proposals::load(&path))
                .await
        }
        Err(reason) => Err(reason),
    };
    match store {
        Ok(store) => {
            let pending = store
                .proposals
                .values()
                .filter(|p| p.status == ProposalStatus::Pending)
                .count();
            let unresolved = store
                .proposals
                .values()
                .filter(|p| {
                    matches!(
                        p.status,
                        ProposalStatus::ApprovalPrepared | ProposalStatus::ReviewRequired
                    )
                })
                .count();
            let last = store
                .attempts
                .last()
                .map(|a| format!("{:?}", a.outcome))
                .unwrap_or_else(|| "none".into());
            let next = store.last_attempt_at.map(|t| t.saturating_add(86400));
            format!(
                "{base}\nPending: {pending}; unresolved approvals: {unresolved} (block execution). Last assessment: {last}. Next daily eligibility: {}.",
                next.map(|t| t.to_string())
                    .unwrap_or_else(|| "not yet charged".into())
            )
        }
        Err(_) => format!(
            "{base}\nPending store unavailable; approval/execution require verified durable storage."
        ),
    }
}

fn eligible(p: &PendingProposal, policy: &Policy, now: u64) -> bool {
    p.status == ProposalStatus::Pending
        && p.created_at <= now
        && p.expires_at > now
        && p.guild == policy.guild
        && p.policy_owner == policy.owner
        && scope_digest(policy, &policy.assessment).is_ok_and(|d| d == p.source.scope_digest)
        && p.validate().is_ok()
}

/// Review content is ephemeral. The exact complete source and access commitment
/// is attached; no permission snapshots, member content or raw provider errors.
fn detail(
    p: &PendingProposal,
    index: usize,
    count: usize,
    approveable: bool,
) -> (String, serenity::all::CreateEmbed, Vec<u8>) {
    let state = if p.status == ProposalStatus::Pending && !approveable {
        "Stale (requires a new reviewed assessment)".to_owned()
    } else {
        format!("{:?}", p.status)
    };
    let content = format!(
        "**Autonomous Operations · proposal {}/{count}**\nStatus: **{state}** · proposal revision: {}\nID: `{}`\nReviewed SHA-256: `{}`\nSource captured: <t:{}:F> · expiry: <t:{}:F>\nOne exact operation is reviewed. Approval appends it to policy; execution and mode remain separately guarded. Full source IDs, access commitment and decision history are in the private receipt.",
        index + 1,
        p.revision,
        p.id,
        p.hash,
        p.source.captured_at,
        p.expires_at
    );
    // JSON quotes distinguish untrusted topic/name/rationale from instructions.
    let operation = serde_json::to_string_pretty(&p.draft.operation)
        .unwrap_or_else(|_| "encoding unavailable".into());
    let reason = serde_json::to_string(&p.draft.reason).unwrap_or_default();
    let description = format!(
        "Operation (JSON):\n{operation}\n\nModel-authored rationale (quoted JSON):\n{reason}"
    );
    let description = if description.chars().count() > 3800 {
        format!(
            "{}\nPreview shortened. The private receipt contains the exact complete operation and rationale.",
            description.chars().take(3800).collect::<String>()
        )
    } else {
        description
    };
    let embed = serenity::all::CreateEmbed::new()
        .title("Proposed operation; complete exact review in private receipt")
        .description(description);
    let receipt = serde_json::to_vec_pretty(p).unwrap_or_default();
    (content, embed, receipt)
}

pub(super) async fn show(
    ctx: &serenity::all::Context,
    interaction: &ComponentInteraction,
    data: &crate::Data,
    session: &crate::admin_dashboard::AdminSession,
    index: usize,
    result: Option<String>,
) {
    let view = async {
        let (_, policy, digest) = current_policy(&data.state, session.guild).await?;
        let live = serenity::all::GuildId::new(session.guild)
            .to_partial_guild(&ctx.http)
            .await
            .map_err(|_| "Current guild owner could not be confirmed.")?;
        let proof = FreshOwnerProof::verified(
            session.guild,
            live.owner_id.get(),
            session.owner,
            interaction.channel_id.get(),
            runtime::now(),
        )?;
        proof.check(&policy, &policy.assessment, runtime::now())?;
        let inspect_data = data_path(data)?.to_owned();
        let inspect_policy = policy.clone();
        let store = data
            .state
            .community_filesystem(move || {
                crate::persist::community_proposals::inspect_pending(
                    &inspect_data,
                    &inspect_policy,
                    runtime::now(),
                )
            })
            .await?;
        let entries: Vec<_> = store
            .proposals
            .values()
            .filter(|p| p.guild == policy.guild && p.policy_owner == policy.owner)
            .collect();
        let input = dashboard_input(
            data,
            session.guild,
            interaction.channel_id.get(),
            result.clone(),
        )
        .await;
        let mut rows = dashboard_rows_with_mode_controls(session, true);
        if entries.is_empty() {
            return Ok::<_, &'static str>((
                crate::admin_dashboard::render(session.page, &input),
                rows,
                None,
            ));
        }
        let index = index.min(entries.len() - 1);
        let p = entries[index];
        let approveable = eligible(p, &policy, runtime::now());
        let mut nonce = [0_u8; 32];
        getrandom::fill(&mut nonce).map_err(|_| "Secure proposal session creation failed.")?;
        let token: String = nonce.iter().map(|b| format!("{b:02x}")).collect();
        let binding = ReviewBinding {
            session: session.clone(),
            origin: interaction.channel_id.get(),
            // The deferred ephemeral response has a new message ID. Bind only
            // after successful Discord readback, never to the clicked message.
            message: 0,
            policy_digest: digest,
            store_revision: store.revision,
            proposal_id: p.id.clone(),
            proposal_revision: p.revision,
            reviewed_hash: p.hash.clone(),
            index,
        };
        AppState::lock(reviews()).insert(token.clone(), binding, runtime::now())?;
        let button = |action: &str, label: &str, disabled| {
            CreateButton::new(format!("{PREFIX}{token}:{action}"))
                .label(label)
                .style(ButtonStyle::Secondary)
                .disabled(disabled)
        };
        rows.push(CreateActionRow::Buttons(vec![
            button("previous", "Previous", index == 0),
            button("next", "Next", index + 1 == entries.len()),
            button("approve", "Approve exact operation", !approveable),
            button("reject", "Reject exact operation", !approveable),
        ]));
        let (mut content, embed, receipt) = detail(p, index, entries.len(), approveable);
        if let Some(result) = result {
            content.push_str(&format!("\n\n{result}"));
        }
        content.push_str(&format!(
            "\nMode {:?}; limits {}/day, {} creations/day. Local-only {}; text readiness {}.",
            policy.mode,
            policy.daily_limit,
            policy.daily_creations,
            data.state.providers.local_only(),
            data.state
                .providers
                .request_readiness(crate::provider::RequestClass::TextReadOnly)
                .is_ok()
        ));
        Ok((content, rows, Some((embed, receipt, token))))
    }
    .await;
    match view {
        Ok((content, rows, extra)) => {
            let mut reply = EditInteractionResponse::new()
                .content(clamp_message(content))
                .components(rows)
                .allowed_mentions(crate::gateway::no_mentions())
                .embeds(Vec::new())
                .attachments(serenity::all::EditAttachments::new());
            let token = extra.as_ref().map(|(_, _, token)| token.clone());
            if let Some((embed, receipt, _)) = extra {
                reply = reply
                    .add_embed(embed)
                    .new_attachment(CreateAttachment::bytes(
                        receipt,
                        "reviewed-community-proposal.json",
                    ));
            }
            match interaction.edit_response(&ctx.http, reply).await {
                Ok(message) => {
                    if let Some(token) = token
                        && let Some(binding) = AppState::lock(reviews()).bindings.get_mut(&token)
                    {
                        binding.message = message.id.get();
                    }
                }
                Err(error) => {
                    if let Some(token) = token {
                        AppState::lock(reviews()).bindings.remove(&token);
                    }
                    crate::gateway::interaction_outcomes::delivery_failed_from(&data.state, &error);
                }
            }
        }
        Err(reason) => {
            let input = dashboard_input(
                data,
                session.guild,
                interaction.channel_id.get(),
                Some(format!("Owner proposal review unavailable: {reason}")),
            )
            .await;
            edit_admin(
                ctx,
                interaction,
                data,
                &crate::admin_dashboard::render(session.page, &input),
                dashboard_rows_with_mode_controls(session, false),
            )
            .await;
        }
    }
}

fn parse_control(id: &str) -> Option<(&str, &str)> {
    let rest = id.strip_prefix(PREFIX)?;
    let (token, action) = rest.split_once(':')?;
    if !matches!(action, "approve" | "reject" | "next" | "previous") {
        return None;
    }
    if token.len() != 64 || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    Some((token, action))
}

async fn decide(
    ctx: &serenity::all::Context,
    interaction: &ComponentInteraction,
    data: &crate::Data,
    b: &ReviewBinding,
    action: &str,
) -> Result<String, &'static str> {
    let (path, policy, digest) = current_policy(&data.state, b.session.guild).await?;
    let store_data = data_path(data)?.to_owned();
    let store = data
        .state
        .community_filesystem(move || crate::persist::community_proposals::load(&store_data))
        .await?;
    b.check_snapshot(&digest, &store)?;
    let p = store
        .proposals
        .get(&b.proposal_id)
        .ok_or("Reviewed proposal unavailable.")?;
    let guild = serenity::all::GuildId::new(b.session.guild)
        .to_partial_guild(&ctx.http)
        .await
        .map_err(|_| "Current guild owner could not be confirmed.")?;
    let owner = FreshOwnerProof::verified(
        b.session.guild,
        guild.owner_id.get(),
        interaction.user.id.get(),
        interaction.channel_id.get(),
        runtime::now(),
    )?;
    owner.check(&policy, &policy.assessment, runtime::now())?;
    if action == "approve" {
        let proofs = crate::gateway::community_ops::assessment::fresh_proofs(
            &ctx.http,
            &policy,
            runtime::now(),
        )
        .await?;
        let operation =
            FreshOperationProof::verified(&policy, &policy.assessment, p, &proofs, runtime::now())?;
        let decision_data = data_path(data)?.to_owned();
        let binding = b.clone();
        let approved = data
            .state
            .community_filesystem(move || {
                crate::persist::community_proposals::approve(
                    crate::persist::community_proposals::ApprovalRequest {
                        policy_path: &path,
                        data: &decision_data,
                        scope: &policy.assessment,
                        expected_policy_digest: &binding.policy_digest,
                        expected_store_revision: binding.store_revision,
                        proposal_id: &binding.proposal_id,
                        reviewed_hash: &binding.reviewed_hash,
                        owner,
                        operation,
                    },
                    runtime::now,
                )
            })
            .await?;
        Ok(format!(
            "Saved and read back exact approved action `{}`. Policy mode was preserved. Discord execution has not been claimed; runtime guards and receipts decide its outcome.",
            approved.key
        ))
    } else {
        let decision_data = data_path(data)?.to_owned();
        let binding = b.clone();
        data.state
            .community_filesystem(move || {
                crate::persist::community_proposals::reject(
                    &decision_data,
                    &path,
                    &policy.assessment,
                    &binding.policy_digest,
                    binding.store_revision,
                    &binding.proposal_id,
                    &binding.reviewed_hash,
                    owner,
                    runtime::now,
                )
            })
            .await?;
        Ok("Saved exact proposal rejection. Policy actions and execution receipts were not changed.".into())
    }
}

pub(super) async fn dispatch(
    ctx: &serenity::all::Context,
    interaction: &ComponentInteraction,
    data: &crate::Data,
) -> bool {
    if !interaction.data.custom_id.starts_with(PREFIX) {
        return false;
    }
    if let Err(error) = interaction
        .create_response(
            &ctx.http,
            CreateInteractionResponse::Defer(
                CreateInteractionResponseMessage::new().ephemeral(true),
            ),
        )
        .await
    {
        crate::gateway::interaction_outcomes::delivery_failed_from(&data.state, &error);
        return true;
    }
    let validated = (|| {
        if interaction.user.bot
            || interaction.message.author.id != ctx.cache.current_user().id
            || !matches!(interaction.data.kind, ComponentInteractionDataKind::Button)
        {
            return Err("Malformed proposal review control.");
        }
        let (token, action) = parse_control(&interaction.data.custom_id)
            .ok_or("Malformed proposal review control.")?;
        let b = AppState::lock(reviews()).take(
            token,
            interaction.user.id.get(),
            interaction.guild_id.map(|g| g.get()),
            interaction.channel_id.get(),
            interaction.message.id.get(),
            runtime::now(),
        )?;
        Ok((b, action))
    })();
    match validated {
        Err(reason) => edit_admin(ctx, interaction, data, reason, Vec::new()).await,
        Ok((binding, action)) => {
            let index = match action {
                "previous" => binding.index.saturating_sub(1),
                "next" => binding.index.saturating_add(1),
                _ => binding.index,
            };
            let result = if matches!(action, "approve" | "reject") {
                Some(decide(ctx, interaction, data, &binding, action).await.unwrap_or_else(|e| format!("Decision refused or requires reconciliation: {e}. No retry or speculative republish was attempted; review current durable status.")))
            } else {
                None
            };
            show(ctx, interaction, data, &binding.session, index, result).await;
        }
    }
    true
}

#[cfg(test)]
mod tests;
