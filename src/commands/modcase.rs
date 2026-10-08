//! Private exact-case review/appeal shell. No classifier or moderation actions.
//! Catalog acknowledgment precedes fresh native facts and retained file reads.
use crate::moderation::shadow::{
    AppealDecision, AppealReason, Case, Change, Counts, ExpectedCase, FreshAuthority, Mutation,
    ReviewDecision, ShadowPolicy,
};
use crate::{
    Context, Error,
    command_catalog::{self as catalog, CommandKey},
    commands_help::CatalogBinding,
    runtime::{self, AppState},
};
use serenity::all::{CommandType, InteractionContext};
use std::{path::PathBuf, sync::atomic::Ordering, time::Duration};

mod evidence;
mod rendering;

const DENIED: &str =
    "This shadow case is unavailable or current access could not be confirmed. No action taken.";
const WINDOW: Duration = Duration::from_secs(20);

#[derive(Debug, Clone, Copy)]
pub(super) struct Envelope {
    guild: u64,
    actor: u64,
    origin: u64,
}
fn envelope(ctx: Context<'_>, key: CommandKey) -> Result<Envelope, &'static str> {
    let poise::Context::Application(app) = ctx else {
        return Err(DENIED);
    };
    let interaction = app.interaction;
    let Some(binding) = app.command.custom_data.downcast_ref::<CatalogBinding>() else {
        return Err(DENIED);
    };
    let spec = catalog::command(key);
    let guild = interaction.guild_id.ok_or(DENIED)?.get();
    let actor = interaction.user.id.get();
    let origin = interaction.channel_id.get();
    if app.interaction_type != poise::CommandInteractionType::Command
        || interaction.data.kind != CommandType::ChatInput
        || interaction.data.name != "modcase"
        || interaction.context != Some(InteractionContext::Guild)
        || binding.key != key
        || binding.eligibility != spec.eligibility
        || app.command.qualified_name != spec.name
        || !app.command.guild_only
        || !app.command.ephemeral
        || !app.has_sent_initial_response.load(Ordering::SeqCst)
        || interaction.application_id.get() != app.serenity_context.cache.current_user().id.get()
        || interaction.user.bot
        || ctx.author().bot
        || [guild, actor, origin].contains(&0)
    {
        return Err(DENIED);
    }
    Ok(Envelope {
        guild,
        actor,
        origin,
    })
}
fn id_valid(id: &str) -> bool {
    id.len() == 64
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn projection(policy: &crate::community_ops::Policy) -> ShadowPolicy {
    ShadowPolicy {
        guild: policy.guild,
        owner: policy.owner,
        stopped: policy.mode == crate::community_ops::Mode::Stopped,
        scope: policy.contextual_shadow.clone(),
    }
}
async fn reply(ctx: Context<'_>, content: String) -> Result<(), Error> {
    ctx.send(
        poise::CreateReply::default()
            .content(crate::commands::clamp_message(content))
            .ephemeral(true)
            .allowed_mentions(crate::gateway::no_mentions()),
    )
    .await?;
    Ok(())
}
struct Loaded {
    native: Envelope,
    path: PathBuf,
    digest: String,
    case: Case,
}
async fn load_authorized(
    ctx: Context<'_>,
    key: CommandKey,
    id: &str,
) -> Result<Loaded, &'static str> {
    if !id_valid(id) {
        return Err(DENIED);
    }
    let native = envelope(ctx, key)?;
    let state = &ctx.data().state;
    let path = state.community_policy_path.clone().ok_or(DENIED)?;
    let data = state.data_dir.clone().ok_or(DENIED)?;
    // Positive current membership/view proof before any private case snapshot.
    let preliminary = evidence::basic(ctx.http(), native).await?;
    let (policy, digest) = state.community_policy(path.clone()).await?;
    if policy.guild != native.guild || policy.owner != preliminary.owner {
        return Err(DENIED);
    }
    let policy = projection(&policy);
    let current = evidence::basic(ctx.http(), native).await?;
    let authority = current.authority(None, None)?;
    let current_policy = policy.clone();
    let current_path = path.clone();
    let current_digest = digest.clone();
    let id = id.to_owned();
    let case = state
        .community_filesystem(move || {
            let (loaded, observed) = crate::persist::community_ops::load_policy(&current_path)?;
            if observed != current_digest
                || projection(&loaded).guild != current_policy.guild
                || loaded.owner != current_policy.owner
            {
                return Err(DENIED);
            }
            // Return only the exact authorized record, never the full case store.
            authority.authorize_snapshot(&projection(&loaded), runtime::now())?;
            let cases = crate::persist::moderation_shadow::load_existing(&data)?.ok_or(DENIED)?;
            cases
                .inspect(&id, &authority, &projection(&loaded), runtime::now())
                .cloned()
        })
        .await?;
    Ok(Loaded {
        native,
        path,
        digest,
        case,
    })
}
async fn inspect_latest(
    state: &AppState,
    loaded: &Loaded,
    authority: FreshAuthority,
) -> Result<(Case, Option<Counts>), &'static str> {
    let data = state.data_dir.clone().ok_or(DENIED)?;
    let path = loaded.path.clone();
    let digest = loaded.digest.clone();
    let id = loaded.case.id.clone();
    state
        .community_filesystem(move || {
            let (policy, observed) = crate::persist::community_ops::load_policy(&path)?;
            if observed != digest {
                return Err(DENIED);
            }
            let policy = projection(&policy);
            authority.authorize_snapshot(&policy, runtime::now())?;
            let cases = crate::persist::moderation_shadow::load_existing(&data)?.ok_or(DENIED)?;
            let now = runtime::now();
            let case = cases.inspect(&id, &authority, &policy, now)?.clone();
            // The pure aggregate API independently requires current staff at
            // the exact review origin. Subjects receive only their own case.
            let counts = cases.measured_counts(&authority, &policy, now).ok();
            Ok((case, counts))
        })
        .await
}

/// Show one private operational case or record a human review/appeal receipt.
#[poise::command(
    slash_command,
    guild_only,
    subcommands("show", "review", "appeal", "resolve_appeal")
)]
pub async fn modcase(_ctx: Context<'_>) -> Result<(), Error> {
    Ok(())
}

/// Show an exact case to its current subject or currently authorized staff.
#[poise::command(slash_command, guild_only, ephemeral)]
pub async fn show(
    ctx: Context<'_>,
    #[description = "Exact saved shadow case ID"] case: String,
) -> Result<(), Error> {
    let result = tokio::time::timeout(WINDOW, async {
        let loaded = load_authorized(ctx, CommandKey::ModcaseShow, &case).await?;
        let fresh = evidence::basic(ctx.http(), loaded.native)
            .await?
            .authority(None, None)?;
        let (current, counts) = inspect_latest(&ctx.data().state, &loaded, fresh).await?;
        Ok::<_, &'static str>(rendering::case(&current, counts))
    })
    .await;
    reply(
        ctx,
        result
            .ok()
            .and_then(Result::ok)
            .unwrap_or_else(|| DENIED.into()),
    )
    .await
}

#[derive(Debug, Clone, Copy, poise::ChoiceParameter)]
pub enum ReviewChoice {
    #[name = "agree with the human assessment"]
    Agree,
    #[name = "disagree with the human assessment"]
    Disagree,
    #[name = "needs context or current evidence is unavailable"]
    NeedsContext,
}
impl From<ReviewChoice> for ReviewDecision {
    fn from(choice: ReviewChoice) -> Self {
        match choice {
            ReviewChoice::Agree => Self::Agree,
            ReviewChoice::Disagree => Self::Disagree,
            ReviewChoice::NeedsContext => Self::NeedsContext,
        }
    }
}
#[derive(Debug, Clone, Copy, poise::ChoiceParameter)]
pub enum AppealChoice {
    #[name = "context is missing"]
    ContextMissing,
    #[name = "attribution is wrong"]
    AttributionWrong,
    #[name = "human assessment is disputed"]
    AssessmentDisputed,
}
impl From<AppealChoice> for AppealReason {
    fn from(choice: AppealChoice) -> Self {
        match choice {
            AppealChoice::ContextMissing => Self::ContextMissing,
            AppealChoice::AttributionWrong => Self::AttributionWrong,
            AppealChoice::AssessmentDisputed => Self::AssessmentDisputed,
        }
    }
}
#[derive(Debug, Clone, Copy, poise::ChoiceParameter)]
pub enum ResolutionChoice {
    #[name = "uphold the subject appeal"]
    Upheld,
    #[name = "reject the subject appeal"]
    Rejected,
    #[name = "needs context or current evidence is unavailable"]
    NeedsContext,
}
impl From<ResolutionChoice> for AppealDecision {
    fn from(choice: ResolutionChoice) -> Self {
        match choice {
            ResolutionChoice::Upheld => Self::Upheld,
            ResolutionChoice::Rejected => Self::Rejected,
            ResolutionChoice::NeedsContext => Self::NeedsContext,
        }
    }
}

enum Decision {
    Review(ReviewDecision),
    Appeal(AppealReason),
    Resolve(AppealDecision),
}
async fn record(
    ctx: Context<'_>,
    key: CommandKey,
    id: String,
    revision: u64,
    decision: Decision,
) -> Result<String, &'static str> {
    if !(1..=4).contains(&revision) {
        return Err(DENIED);
    }
    let loaded = load_authorized(ctx, key, &id).await?;
    let is_staff = !matches!(decision, Decision::Appeal(_));
    let (observed, source_at) = if is_staff {
        evidence::source(
            ctx.http(),
            loaded.native,
            &loaded.case.source,
            loaded.case.captured.owner,
        )
        .await?
    } else {
        (None, None)
    };
    // Repeat current origin/role/owner permission facts after source hydration.
    let authority = evidence::basic(ctx.http(), loaded.native)
        .await?
        .authority(observed, source_at)?;
    // Scope + independence + source evidence + exact expected revision are
    // checked once more inside the retained transaction and immediately before
    // rename. Stop/default-off does not refuse existing review/appeal receipts.
    let expected = ExpectedCase { id, revision };
    let label;
    let mutation = match decision {
        Decision::Review(decision) => {
            label = "Independent human review";
            Mutation::Review {
                expected,
                authority,
                decision,
            }
        }
        Decision::Appeal(reason) => {
            label = "Subject appeal";
            Mutation::Appeal {
                expected,
                authority,
                reason,
            }
        }
        Decision::Resolve(decision) => {
            label = "Independent appeal resolution";
            Mutation::ResolveAppeal {
                expected,
                authority,
                decision,
            }
        }
    };
    let receipt = ctx
        .data()
        .state
        .publish_moderation_shadow(loaded.path, loaded.digest, mutation)
        .await?;
    if receipt.store_revision < receipt.case_revision {
        return Err(DENIED);
    }
    let status = match receipt.change {
        Change::Changed => label.to_owned(),
        Change::AlreadyObserved => format!("Existing {}", label.to_lowercase()),
    };
    Ok(format!(
        "{status} receipt saved for shadow case {} at revision {}. No action taken. This is an operational record, not independent classification or a qualified live moderation pilot.",
        receipt.case_id, receipt.case_revision
    ))
}
async fn finish(
    ctx: Context<'_>,
    work: impl std::future::Future<Output = Result<String, &'static str>>,
) -> Result<(), Error> {
    let text = tokio::time::timeout(WINDOW, work)
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or_else(|| DENIED.into());
    reply(ctx, text).await
}

/// Record an independent current staff review of the exact source version.
#[poise::command(slash_command, guild_only, ephemeral)]
pub async fn review(
    ctx: Context<'_>,
    #[description = "Exact saved shadow case ID"] case: String,
    #[description = "Case revision shown by /modcase show"] revision: u64,
    #[description = "Your independent human review"] decision: ReviewChoice,
) -> Result<(), Error> {
    finish(
        ctx,
        record(
            ctx,
            CommandKey::ModcaseReview,
            case,
            revision,
            Decision::Review(decision.into()),
        ),
    )
    .await
}
/// Appeal your own exact case in its original source channel.
#[poise::command(slash_command, guild_only, ephemeral)]
pub async fn appeal(
    ctx: Context<'_>,
    #[description = "Exact saved shadow case ID"] case: String,
    #[description = "Case revision shown by /modcase show"] revision: u64,
    #[description = "Your closed appeal reason; no source text is retained"] reason: AppealChoice,
) -> Result<(), Error> {
    finish(
        ctx,
        record(
            ctx,
            CommandKey::ModcaseAppeal,
            case,
            revision,
            Decision::Appeal(reason.into()),
        ),
    )
    .await
}
/// Resolve a subject appeal as a different currently authorized staff member.
#[poise::command(slash_command, guild_only, ephemeral)]
pub async fn resolve_appeal(
    ctx: Context<'_>,
    #[description = "Exact saved shadow case ID"] case: String,
    #[description = "Case revision shown by /modcase show"] revision: u64,
    #[description = "Your independent human appeal decision"] decision: ResolutionChoice,
) -> Result<(), Error> {
    finish(
        ctx,
        record(
            ctx,
            CommandKey::ModcaseResolveAppeal,
            case,
            revision,
            Decision::Resolve(decision.into()),
        ),
    )
    .await
}
