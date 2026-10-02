//! Scheduled maintenance through the retained Discord transport; no second gateway.
use crate::community_ops::{Action, Mode, Operation, Receipt, Status};
pub(crate) mod assessment;
mod extended;
use crate::persist::community_ops::Lease;
use serenity::all::{
    ChannelId, ChannelType, CreateChannel, EditChannel, GuildChannel, GuildId, Http, Permissions,
};
use std::{path::Path, time::Duration};
use tokio_util::sync::CancellationToken;

/// Dropping a request cannot undo remote I/O. Callers retain the reservation and
/// reconcile fresh readback when this watcher stops an in-flight operation.
async fn watched_mutation<
    T,
    P: std::future::Future<Output = Result<(crate::community_ops::Policy, String), &'static str>>,
>(
    mutation: impl std::future::Future<Output = Result<T, &'static str>>,
    policy: &crate::community_ops::Policy,
    digest: &str,
    cancel: &CancellationToken,
    mut load: impl FnMut() -> P,
) -> Result<T, &'static str> {
    // Check synchronously before the mutation future is ever polled. Tokio's
    // first interval tick can be pending on its initial poll, allowing the
    // HTTP future to begin before a stop is observed if this check is omitted.
    if cancel.is_cancelled() {
        return Err("cancelled before mutation");
    }
    match tokio::time::timeout(Duration::from_millis(250), load()).await {
        Ok(Ok((current, current_digest))) if current == *policy && current_digest == digest => {}
        _ => return Err("owner policy changed before mutation"),
    }
    let mutation = tokio::time::timeout(Duration::from_secs(30), mutation);
    tokio::pin!(mutation);
    let mut watch = tokio::time::interval(Duration::from_millis(250));
    loop {
        tokio::select! {
            biased;
            () = cancel.cancelled() => return Err("cancelled; mutation outcome unknown"),
            _ = watch.tick() => {
                match tokio::time::timeout(Duration::from_millis(250), load()).await {
                    Ok(Ok((current, current_digest))) if current == *policy && current_digest == digest => {},
                    _ => return Err("owner policy changed; mutation outcome unknown"),
                }
            },
            result = &mut mutation => return result.unwrap_or(Err("mutation timed out")),
        }
    }
}

fn access(channel: &GuildChannel) -> serde_json::Value {
    let mut overwrites: Vec<_> = channel
        .permission_overwrites
        .iter()
        .map(|o| {
            serde_json::json!({
                "target": format!("{:?}", o.kind), "allow": o.allow.bits(), "deny": o.deny.bits()
            })
        })
        .collect();
    overwrites.sort_by_key(|v| v["target"].as_str().unwrap_or_default().to_owned());
    serde_json::json!({"nsfw": channel.nsfw, "overwrites": overwrites})
}

fn record(channel: &GuildChannel) -> serde_json::Value {
    serde_json::json!({"id":channel.id.get(), "name":channel.name, "parent":channel.parent_id.map(|id|id.get()),
        "topic":channel.topic, "access":access(channel)})
}

fn public_category(channel: &GuildChannel, everyone: Permissions) -> bool {
    if channel.kind != ChannelType::Category || channel.nsfw {
        return false;
    }
    let mut permissions = everyone;
    for overwrite in &channel.permission_overwrites {
        if overwrite.kind
            == serenity::all::PermissionOverwriteType::Role(channel.guild_id.everyone_role())
        {
            permissions.remove(overwrite.deny);
            permissions.insert(overwrite.allow);
        }
        // Any member/role view deny creates a boundary this adapter cannot prove.
        if overwrite.deny.contains(Permissions::VIEW_CHANNEL) {
            return false;
        }
    }
    permissions.contains(Permissions::VIEW_CHANNEL)
}

fn already_matches(
    action: &Action,
    channels: &std::collections::HashMap<ChannelId, GuildChannel>,
) -> bool {
    match &action.operation {
        Operation::Topic { channel, topic } => channels
            .get(&ChannelId::new(*channel))
            .is_some_and(|c| c.topic.as_deref() == Some(topic)),
        Operation::Move { channel, category } => channels
            .get(&ChannelId::new(*channel))
            .is_some_and(|c| c.parent_id == Some(ChannelId::new(*category))),
        Operation::CreateText {
            category,
            name,
            topic,
        } => {
            let matching: Vec<_> = channels
                .values()
                .filter(|c| c.parent_id == Some(ChannelId::new(*category)) && c.name == *name)
                .collect();
            matching.len() == 1
                && matching[0].kind == ChannelType::Text
                && matching[0].topic.as_deref() == Some(topic)
        }
        _ => false,
    }
}
fn settled_action(
    ledger: &crate::community_ops::Ledger,
    action: &Action,
) -> Result<bool, &'static str> {
    match ledger.receipts.get(&action.key) {
        Some(receipt) if receipt.action != *action => Err("idempotency identity drift"),
        Some(receipt) if receipt.status == Status::Verified => Ok(true),
        Some(receipt) if matches!(receipt.status, Status::Reserved | Status::ReviewRequired) => {
            Err("unresolved action requires owner review")
        }
        _ => Ok(false),
    }
}

fn observed_matches(action: &Action, observed: &GuildChannel) -> bool {
    match &action.operation {
        Operation::Topic { channel, topic } => {
            observed.id.get() == *channel && observed.topic.as_deref() == Some(topic)
        }
        Operation::Move { channel, category } => {
            observed.id.get() == *channel && observed.parent_id == Some(ChannelId::new(*category))
        }
        Operation::CreateText {
            category,
            name,
            topic,
        } => {
            observed.kind == ChannelType::Text
                && observed.name == *name
                && observed.topic.as_deref() == Some(topic)
                && observed.parent_id == Some(ChannelId::new(*category))
        }
        _ => false,
    }
}

async fn execute(
    http: &Http,
    guild: GuildId,
    action: &Action,
    before: &GuildChannel,
) -> Result<GuildChannel, &'static str> {
    let reason = "Abbey approved community maintenance";
    match &action.operation {
        Operation::Topic { channel, topic } => {
            ChannelId::new(*channel)
                .edit(
                    http,
                    EditChannel::new().topic(topic).audit_log_reason(reason),
                )
                .await
        }
        Operation::Move { channel, category } => {
            ChannelId::new(*channel)
                .edit(
                    http,
                    EditChannel::new()
                        .category(ChannelId::new(*category))
                        .audit_log_reason(reason),
                )
                .await
        }
        Operation::CreateText {
            category,
            name,
            topic,
        } => {
            guild
                .create_channel(
                    http,
                    CreateChannel::new(name)
                        .kind(ChannelType::Text)
                        .category(ChannelId::new(*category))
                        .topic(topic)
                        .permissions(before.permission_overwrites.clone())
                        .audit_log_reason(reason),
                )
                .await
        }
        _ => return Err("unsupported operation dispatch"),
    }
    .map_err(|_| "Discord outcome requires readback")
}

/// Disabled without an owner-only policy file. Daily assessment skips missed runs.
pub(crate) async fn maintain(
    http: &Http,
    state: std::sync::Arc<crate::runtime::AppState>,
    cancel: CancellationToken,
    now: u64,
) -> Result<(), &'static str> {
    let Some(path) = std::env::var_os("ABBEY_COMMUNITY_POLICY") else {
        return Ok(());
    };
    let Some(data) = &state.data_dir else {
        return Ok(());
    };
    let (policy, _) = state
        .community_policy(std::path::PathBuf::from(&path))
        .await?;
    if policy.mode == Mode::Stopped || cancel.is_cancelled() {
        return Ok(());
    }
    let reconcile_data = data.clone();
    let reconcile_path = std::path::PathBuf::from(&path);
    let scope = policy.assessment.clone();
    state
        .community_filesystem(move || {
            crate::persist::community_proposals::reconcile(&reconcile_data, &reconcile_path, &scope)
        })
        .await?;
    assessment::run(http, state.clone(), Path::new(&path), cancel.clone(), now).await?;
    let (policy, digest) = state
        .community_policy(std::path::PathBuf::from(&path))
        .await?;
    if policy.mode == Mode::Stopped || cancel.is_cancelled() {
        return Ok(());
    }
    let initial_data = data.clone();
    let (proposals, lease, mut ledger) = state
        .community_filesystem(move || {
            let proposals = crate::persist::community_proposals::load(&initial_data)?;
            let lease = std::sync::Arc::new(Lease::acquire(&initial_data)?);
            let ledger = lease.load()?;
            Ok((proposals, lease, ledger))
        })
        .await?;
    if ledger.last_policy_digest.as_deref() == Some(digest.as_str())
        && ledger
            .last_assessment
            .is_some_and(|last| now.saturating_sub(last) < 86400)
    {
        return Ok(());
    }
    let guild_id = GuildId::new(policy.guild);
    let guild = guild_id
        .to_partial_guild(http)
        .await
        .map_err(|_| "guild observation unavailable")?;
    if guild.owner_id.get() != policy.owner {
        return Err("policy owner does not match live owner");
    }
    let everyone = guild
        .roles
        .get(&guild_id.everyone_role())
        .ok_or("everyone role unavailable")?
        .permissions;
    // A fresh role snapshot is required before and after each operation.
    let role_signature =
        serde_json::to_value(&guild.roles).map_err(|_| "role observation invalid")?;
    ledger.last_assessment = Some(now);
    ledger.last_policy_digest = Some(digest.clone());
    save_receipts(&state, &lease, &ledger).await?;
    for action in &policy.actions {
        if cancel.is_cancelled() {
            break;
        }
        if !proposals.executable(action) {
            return Err("pending or unresolved model action cannot execute");
        }
        if settled_action(&ledger, action)? {
            continue;
        }
        // Re-read policy on every action, including the global stop mode.
        let (current, current_digest) = state
            .community_policy(std::path::PathBuf::from(&path))
            .await?;
        if current != policy || current_digest != digest {
            return Err("policy changed; execution stopped");
        }
        if extended::handles(&action.operation) {
            extended::run(
                &state,
                http,
                Path::new(&path),
                &policy,
                &digest,
                action,
                &lease,
                &mut ledger,
                cancel.clone(),
                now,
            )
            .await?;
            continue;
        }
        let channels = guild_id
            .channels(http)
            .await
            .map_err(|_| "channel observation unavailable")?;
        if already_matches(action, &channels) {
            continue;
        }
        let before = match &action.operation {
            Operation::Topic { channel, .. } | Operation::Move { channel, .. } => {
                channels.get(&ChannelId::new(*channel))
            }
            Operation::CreateText { category, .. } => channels.get(&ChannelId::new(*category)),
            _ => return Err("unsupported operation dispatch"),
        }
        .ok_or("maintenance target unavailable")?;
        let origin_category = if before.kind == ChannelType::Category {
            before.id
        } else {
            before.parent_id.ok_or("target has no approved category")?
        };
        if before
            .permission_overwrites
            .iter()
            .any(|o| o.deny.contains(Permissions::VIEW_CHANNEL))
        {
            return Err("target channel has private access boundaries");
        }
        let mut target_everyone = everyone;
        for overwrite in &before.permission_overwrites {
            if overwrite.kind
                == serenity::all::PermissionOverwriteType::Role(guild_id.everyone_role())
            {
                target_everyone.remove(overwrite.deny);
                target_everyone.insert(overwrite.allow);
            }
        }
        if !target_everyone.contains(Permissions::VIEW_CHANNEL) {
            return Err("target ordinary visibility cannot be proven");
        }
        match &action.operation {
            Operation::Topic { .. }
                if !matches!(
                    before.kind,
                    ChannelType::Text | ChannelType::News | ChannelType::Forum
                ) =>
            {
                return Err("target has no topic capability");
            }
            Operation::CreateText { category, name, .. }
                if channels
                    .values()
                    .any(|c| c.parent_id == Some(ChannelId::new(*category)) && c.name == *name) =>
            {
                return Err("creation name collides with an existing channel");
            }
            _ => {}
        }
        if before.nsfw
            || !policy.public_categories.contains(&origin_category.get())
            || !channels
                .get(&origin_category)
                .is_some_and(|c| public_category(c, everyone))
        {
            return Err("target crosses a private or sensitive boundary");
        }
        if let Operation::Move { category, .. } = action.operation
            && !channels
                .get(&ChannelId::new(category))
                .is_some_and(|c| public_category(c, everyone))
        {
            return Err("destination access cannot be proven public");
        }
        if !matches!(
            before.kind,
            ChannelType::Text | ChannelType::News | ChannelType::Forum | ChannelType::Category
        ) {
            return Err("unsupported target kind");
        }
        if ledger
            .receipts
            .get(&action.key)
            .is_some_and(|r| r.action != *action)
        {
            return Err("idempotency identity drift");
        }
        if policy.mode == Mode::Propose {
            ledger
                .receipts
                .entry(action.key.clone())
                .or_insert(Receipt {
                    action: action.clone(),
                    policy_digest: digest.clone(),
                    at: now,
                    status: Status::Proposed,
                    before: record(before),
                    observed: None,
                    detail: "owner policy proposal; no mutation".into(),
                });
            save_receipts(&state, &lease, &ledger).await?;
            continue;
        }
        ledger.authorize(&policy, action, now)?;
        let fresh_guild = guild_id
            .to_partial_guild(http)
            .await
            .map_err(|_| "preflight guild observation unavailable")?;
        if fresh_guild.owner_id != guild.owner_id
            || serde_json::to_value(&fresh_guild.roles).map_err(|_| "role observation invalid")?
                != role_signature
        {
            return Err("guild permissions changed; execution stopped");
        }
        let current_channels = guild_id
            .channels(http)
            .await
            .map_err(|_| "preflight channels unavailable")?;
        if serde_json::to_value(&current_channels).map_err(|_| "preflight invalid")?
            != serde_json::to_value(&channels).map_err(|_| "baseline invalid")?
        {
            return Err("channel state changed; execution stopped");
        }
        let (final_policy, final_digest) = state
            .community_policy(std::path::PathBuf::from(&path))
            .await?;
        if final_policy != policy || final_digest != digest || cancel.is_cancelled() {
            return Err("policy or stop state changed; execution stopped");
        }
        ledger.receipts.insert(
            action.key.clone(),
            Receipt {
                action: action.clone(),
                policy_digest: digest.clone(),
                at: now,
                status: Status::Reserved,
                before: record(before),
                observed: None,
                detail: "reserved before Discord I/O".into(),
            },
        );
        save_receipts(&state, &lease, &ledger).await?;
        let result = watched_mutation(
            execute(http, guild_id, action, before),
            &policy,
            &digest,
            &cancel,
            || state.community_policy(std::path::PathBuf::from(&path)),
        )
        .await;
        let receipt = ledger
            .receipts
            .get_mut(&action.key)
            .ok_or("reservation unavailable")?;
        receipt.status = Status::ReviewRequired;
        receipt.detail = "remote outcome requires review; do not replay".into();
        let readback = tokio::time::timeout(Duration::from_secs(30), async {
            let all = guild_id
                .channels(http)
                .await
                .map_err(|_| "post-apply observation unavailable")?;
            let roles_after = guild_id
                .to_partial_guild(http)
                .await
                .map_err(|_| "post-apply role observation unavailable")?;
            Ok::<_, &'static str>((all, roles_after))
        })
        .await;
        if let Ok(Ok((all, roles_after))) = readback {
            let observed = match &action.operation {
                Operation::Topic { channel, .. } | Operation::Move { channel, .. } => {
                    all.get(&ChannelId::new(*channel))
                }
                Operation::CreateText { category, name, .. } => {
                    let mut matches = all.values().filter(|c| {
                        c.parent_id == Some(ChannelId::new(*category)) && c.name == *name
                    });
                    let first = matches.next();
                    if matches.next().is_none() {
                        first
                    } else {
                        None
                    }
                }
                _ => None,
            };
            if let Some(observed) = observed {
                receipt.observed = Some(record(observed));
                if result
                    .as_ref()
                    .is_ok_and(|returned| returned.id == observed.id)
                    && access(observed) == access(before)
                    && observed_matches(action, observed)
                    && already_matches(action, &all)
                    && serde_json::to_value(&roles_after.roles).ok().as_ref()
                        == Some(&role_signature)
                {
                    receipt.status = Status::Verified;
                    receipt.detail =
                        "readback verified; IDs/history retained and access unchanged".into();
                }
            } else {
                receipt.observed = serde_json::to_value(&all).ok();
            }
        }
        let verified = receipt.status == Status::Verified;
        save_receipts(&state, &lease, &ledger).await?;
        if !verified {
            return Err("operation requires owner reconciliation; remaining work stopped");
        }
    }
    Ok(())
}

async fn save_receipts(
    state: &crate::runtime::AppState,
    lease: &std::sync::Arc<Lease>,
    ledger: &crate::community_ops::Ledger,
) -> Result<(), &'static str> {
    let lease = lease.clone();
    let ledger = ledger.clone();
    state
        .community_filesystem(move || lease.save(&ledger))
        .await
}

/// Explicit scheduler dependency; independent of Work delivery transport.
pub(crate) struct DiscordCommunityMaintenance(pub std::sync::Arc<serenity::all::Http>);
impl crate::runtime::scheduler::CommunityMaintenance for DiscordCommunityMaintenance {
    async fn maintain(
        &self,
        state: std::sync::Arc<crate::runtime::AppState>,
        cancel: tokio_util::sync::CancellationToken,
        now: u64,
    ) -> Result<(), &'static str> {
        maintain(&self.0, state, cancel, now).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn verified_extended_inventory_does_not_block_later_action_and_uncertain_is_not_skipped() {
        let old = Action {
            key: "old-role".into(),
            reason: "owner interest".into(),
            operation: Operation::CreateInterestRole {
                name: "Research".into(),
            },
        };
        let next = Action {
            key: "next-role".into(),
            reason: "next owner interest".into(),
            operation: Operation::CreateInterestRole {
                name: "Robotics".into(),
            },
        };
        let mut policy:crate::community_ops::Policy=serde_json::from_value(serde_json::json!({"version":1,"guild":1,"owner":2,"mode":"apply","daily_limit":5,"daily_creations":2,"public_categories":[],"protected_channels":[],"actions":[]})).unwrap();
        policy.actions = vec![old.clone(), next.clone()];
        let mut ledger = crate::community_ops::Ledger::default();
        ledger.receipts.insert(
            old.key.clone(),
            Receipt {
                action: old.clone(),
                policy_digest: "past".into(),
                at: 10,
                status: Status::Verified,
                before: serde_json::Value::Null,
                observed: Some(serde_json::json!({"role":5})),
                detail: "verified".into(),
            },
        );
        let mut dispatched = Vec::new();
        for action in &policy.actions {
            if settled_action(&ledger, action).unwrap() {
                continue;
            }
            ledger.authorize(&policy, action, 11).unwrap();
            dispatched.push(action.key.clone());
        }
        assert_eq!(dispatched, vec![next.key]);
        let mut drifted = old.clone();
        drifted.operation = Operation::CreateInterestRole {
            name: "Changed".into(),
        };
        assert!(settled_action(&ledger, &drifted).is_err());
        for status in [Status::Reserved, Status::ReviewRequired] {
            ledger.receipts.get_mut(&old.key).unwrap().status = status;
            assert!(settled_action(&ledger, &old).is_err());
        }
    }
    #[tokio::test]
    async fn owner_stop_after_reservation_and_during_io_is_observed() {
        for after in [1, 3] {
            let policy: crate::community_ops::Policy = serde_json::from_value(serde_json::json!({"version":1,"guild":1,"owner":2,"mode":"apply","daily_limit":5,"daily_creations":2,"public_categories":[],"protected_channels":[],"actions":[]})).unwrap();
            let calls = std::cell::Cell::new(0);
            let began = std::cell::Cell::new(false);
            let result = watched_mutation(
                async {
                    began.set(true);
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    Ok(())
                },
                &policy,
                "digest",
                &CancellationToken::new(),
                || {
                    calls.set(calls.get() + 1);
                    let mut current = policy.clone();
                    if calls.get() >= after {
                        current.mode = Mode::Stopped;
                    }
                    std::future::ready(Ok((current, "digest".into())))
                },
            )
            .await;
            assert!(result.is_err());
            assert_eq!(began.get(), after > 1);
        }
    }
    #[test]
    fn sensitive_category_is_not_public_even_with_everyone_view() {
        let channel: GuildChannel = serde_json::from_value(serde_json::json!({
            "id":"3", "guild_id":"1", "type":4, "name":"adult", "position":0,
            "permission_overwrites":[], "nsfw":true
        }))
        .unwrap();
        assert!(!public_category(&channel, Permissions::VIEW_CHANNEL));
    }
}
