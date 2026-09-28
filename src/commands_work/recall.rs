//! Private scope policy controls. Catalog dispatch defers before REST access;
//! canonical authorization is repeated by the retained policy transaction.
use super::{access, reply};
use crate::{Context, Error, runtime::RecallPolicyStatus};

#[poise::command(slash_command, subcommands("configure", "show"))]
pub async fn recall(_ctx: Context<'_>) -> Result<(), Error> {
    Ok(())
}

#[poise::command(slash_command, ephemeral)]
pub async fn configure(
    ctx: Context<'_>,
    #[description = "Enable recall for the current scope"] enabled: bool,
    #[description = "Policy revision from recall show; 0 for first configuration"] revision: u64,
) -> Result<(), Error> {
    let access = access(ctx).await?;
    let status = ctx
        .data()
        .state
        .configure_work_recall(access, enabled, revision)
        .await?;
    reply(ctx, render(&status, true)).await
}

#[poise::command(slash_command, ephemeral)]
pub async fn show(ctx: Context<'_>) -> Result<(), Error> {
    let access = access(ctx).await?;
    let status = ctx.data().state.work_recall_policy(access)?;
    reply(ctx, render(&status, false)).await
}

fn render(status: &RecallPolicyStatus, saved: bool) -> String {
    let policy = &status.policy;
    let prefix = if saved {
        "Saved recall policy."
    } else {
        "Recall policy."
    };
    let state = if policy.enabled {
        "enabled"
    } else {
        "disabled"
    };
    let availability = if status.operator_available {
        "available"
    } else {
        "unavailable"
    };
    let principal = if policy.configured_by == 0 {
        "No configuring principal yet.".into()
    } else {
        format!("Configured by user {}.", policy.configured_by)
    };
    format!(
        "{prefix} Scope opt-in: {state}. Operator rollout: {availability}. Revision {}. {principal}\nConfiguration does not index history or delete retained evidence. Recall admission and briefing integration are not active in this implementation stage.",
        policy.revision
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn read_policy_controls_without_claiming_recall_or_erasure() {
        for (enabled, available, revision, saved) in [
            (false, false, 0, false),
            (false, true, 1, true),
            (true, true, 2, true),
            (true, false, 2, false),
        ] {
            let text = render(
                &RecallPolicyStatus {
                    policy: crate::work::recall_policy::RecallPolicy {
                        enabled,
                        configured_by: if revision == 0 { 0 } else { 42 },
                        revision,
                    },
                    operator_available: available,
                },
                saved,
            );
            println!("{text}");
            assert!(text.contains("does not index history or delete retained evidence"));
            assert!(text.contains("not active"));
            assert_eq!(text.starts_with("Saved"), saved);
        }
        println!("Denied: {}", crate::work::WorkError::Denied);
        println!("Not saved: {}", crate::work::WorkError::Persistence);
    }
}
