//! Stage decisions and snapshot matching for the additive plan engine.
use super::{Change, Report, Scope, Stage, Target, TopicEdit, describe_channel};
use crate::server::observe::{
    COMMUNITY_FEATURE, ChannelState, GuildSnapshot, OverwriteState, OverwriteTarget,
};
use crate::server::plan::{EVERYONE, Overwrite, Plan, PlanCategory, PlanChannel, sorted_unique};
use std::collections::BTreeSet;

/// Compute what `scope` would do to `snapshot` to move it toward `plan`.
#[must_use]
pub fn diff(plan: &Plan, snapshot: &GuildSnapshot, scope: &Scope) -> Report {
    let mut cx = Context {
        plan,
        snapshot,
        report: Report {
            stage: scope.stage,
            preflight: Vec::new(),
            changes: Vec::new(),
            blockers: Vec::new(),
            warnings: Vec::new(),
            manual: Vec::new(),
        },
    };
    cx.preflight_facts();
    match scope.stage {
        Stage::Additive => cx.additive(),
        Stage::Reveal => cx.reveal(),
        Stage::Overwrites => match scope.category.as_deref() {
            Some(category) => cx.overwrites(category),
            None => cx
                .report
                .blockers
                .push("--stage overwrites needs --category NAME: the proposal flips gates one category at a time".into()),
        },
    }
    cx.manual_steps();
    cx.check_bot_capability();
    cx.report
}

struct Context<'a> {
    plan: &'a Plan,
    snapshot: &'a GuildSnapshot,
    report: Report,
}

/// Whether a planned overwrite may land during `reveal`: anything for a role;
/// for `@everyone`, anything that leaves View Channel where it is, plus the
/// one transition from engine-hidden to visible. `engine_hidden` is decided
/// by [`is_engine_hidden`], never by the `@everyone` entry alone.
pub(super) fn reveal_permits(
    current: Option<&Overwrite>,
    planned: &Overwrite,
    engine_hidden: bool,
) -> bool {
    if planned.role != EVERYONE {
        return true;
    }
    let current_denies_view = current.is_some_and(Overwrite::denies_view);
    if current_denies_view == planned.denies_view() {
        return true;
    }
    engine_hidden && !planned.denies_view()
}

fn always_permits(_: Option<&Overwrite>, _: &Overwrite, _: bool) -> bool {
    true
}

/// A channel the engine hid in `additive` and nobody has touched since: the
/// hide marker is its *only* overwrite. A hand-gated channel carries at least
/// one role or member entry beside the `@everyone` deny, so it never reads as
/// engine-hidden and `reveal` leaves its gate alone.
pub(super) fn is_engine_hidden(state: &ChannelState, guild_id: u64) -> bool {
    match state.overwrites.as_slice() {
        [only] => {
            only.target == OverwriteTarget::Role(guild_id)
                && only.allow.is_empty()
                && sorted_unique(&only.deny) == [crate::server::plan::VIEW_CHANNEL]
        }
        _ => false,
    }
}

impl<'a> Context<'a> {
    fn preflight_facts(&mut self) {
        let snap = self.snapshot;
        let bot = if snap.bot_is_administrator() {
            "Administrator".to_string()
        } else {
            let perms = snap.bot_permissions();
            let mut held = Vec::new();
            for needed in ["Manage Channels", "Manage Roles"] {
                held.push(format!(
                    "{needed}: {}",
                    if perms.contains(needed) { "yes" } else { "NO" }
                ));
            }
            held.join(", ")
        };
        self.report.preflight.push(format!(
            "bot user {} holds {bot}; top role position {}",
            snap.bot.user_id,
            snap.bot_top_position()
        ));
        self.report.preflight.push(format!(
            "COMMUNITY feature: {}{}",
            if snap.has_feature(COMMUNITY_FEATURE) {
                "present"
            } else {
                "absent"
            },
            if self.plan.needs_community() {
                " (the plan needs it)"
            } else {
                ""
            }
        ));
        self.report.preflight.push(format!(
            "guild has {} roles and {} channels; plan has {} roles, {} categories, {} channels",
            snap.roles.len(),
            snap.channels.len(),
            self.plan.roles.len(),
            self.plan.categories.len(),
            self.plan.channels().count()
        ));
    }

    fn additive(&mut self) {
        for role in &self.plan.roles {
            match self.role_lookup(&role.name) {
                RoleLookup::Missing => self.report.changes.push(Change::CreateRole {
                    name: role.name.clone(),
                    colour: role.colour,
                }),
                RoleLookup::Found(_) => {}
                RoleLookup::Managed => self.report.blockers.push(Self::managed_blocker(&role.name)),
                RoleLookup::Ambiguous(n) => self.report.blockers.push(format!(
                    "role {:?} exists {n} times; rename the extras before the engine can match it",
                    role.name
                )),
            }
        }
        for category in &self.plan.categories {
            match self.category_lookup(&category.name) {
                Lookup::Missing => self.report.changes.push(Change::CreateCategory {
                    name: category.name.clone(),
                    overwrites: vec![Overwrite::hide_marker()],
                }),
                Lookup::Found(_) => {}
                Lookup::Ambiguous(n) => self.report.blockers.push(format!(
                    "category {:?} exists {n} times; merge or rename before the engine can match it",
                    category.name
                )),
            }
            for channel in &category.channels {
                match self.channel_lookup(channel) {
                    Lookup::Missing => {
                        if channel.kind.needs_community()
                            && !self.snapshot.has_feature(COMMUNITY_FEATURE)
                        {
                            self.report.blockers.push(format!(
                                "{} needs Community mode, which this guild lacks (Server Settings → Community)",
                                describe_channel(channel.kind, &channel.name)
                            ));
                        }
                        self.report.changes.push(Change::CreateChannel {
                            name: channel.name.clone(),
                            kind: channel.kind,
                            category: category.name.clone(),
                            topic: channel.topic.clone(),
                            slowmode_secs: channel.slowmode_secs,
                            tags: channel.tags.clone(),
                            overwrites: vec![Overwrite::hide_marker()],
                        });
                    }
                    Lookup::Found(_) => {}
                    Lookup::Ambiguous(n) => self.report.blockers.push(format!(
                        "{} exists {n} times; merge or rename before the engine can match it",
                        describe_channel(channel.kind, &channel.name)
                    )),
                }
            }
        }
    }

    fn reveal(&mut self) {
        for role in &self.plan.roles {
            match self.role_lookup(&role.name) {
                RoleLookup::Missing => self.report.blockers.push(format!(
                    "role {:?} does not exist yet; run --stage additive first",
                    role.name
                )),
                RoleLookup::Ambiguous(n) => self
                    .report
                    .blockers
                    .push(format!("role {:?} exists {n} times", role.name)),
                RoleLookup::Managed => self.report.blockers.push(Self::managed_blocker(&role.name)),
                RoleLookup::Found(id) => {
                    let current = self
                        .snapshot
                        .role_by_id(id)
                        .expect("lookup returned a live id");
                    let wanted = (role.hoist, role.mentionable, role.colour.unwrap_or(0));
                    if (current.hoist, current.mentionable, current.colour) == wanted {
                        continue;
                    }
                    if current.position >= self.snapshot.bot_top_position() {
                        self.report.blockers.push(format!(
                            "role {:?} sits at position {}, at or above the bot's top role ({}); Discord would refuse the edit",
                            role.name,
                            current.position,
                            self.snapshot.bot_top_position()
                        ));
                        continue;
                    }
                    self.report.changes.push(Change::EditRole {
                        name: role.name.clone(),
                        hoist: role.hoist,
                        mentionable: role.mentionable,
                        colour: role.colour,
                    });
                }
            }
        }
        let categories: Vec<&PlanCategory> = self.plan.categories.iter().collect();
        for category in categories {
            self.reveal_category(category);
        }
    }

    fn reveal_category(&mut self, category: &PlanCategory) {
        let category_id = match self.category_lookup(&category.name) {
            Lookup::Missing => {
                self.report.blockers.push(format!(
                    "category {:?} does not exist yet; run --stage additive first",
                    category.name
                ));
                return;
            }
            Lookup::Ambiguous(n) => {
                self.report
                    .blockers
                    .push(format!("category {:?} exists {n} times", category.name));
                return;
            }
            Lookup::Found(id) => id,
        };
        let state = self
            .snapshot
            .channels
            .iter()
            .find(|c| c.id == category_id)
            .expect("live id");
        self.overwrite_changes(
            Target::Category(category.name.clone()),
            &category.overwrites,
            state,
            reveal_permits,
        );
        for channel in &category.channels {
            let Some(state) = self.require_channel(channel) else {
                continue;
            };
            let topic = channel
                .topic
                .as_deref()
                .map(str::trim)
                .filter(|t| !t.is_empty());
            let current_topic = state
                .topic
                .as_deref()
                .map(str::trim)
                .filter(|t| !t.is_empty());
            // A plan without a topic leaves the live topic alone; otherwise a
            // channel with a hand-written topic would be "edited" on every diff
            // with an edit that sends nothing, and --apply could never verify.
            let topic_differs =
                channel.kind.has_topic() && topic.is_some() && topic != current_topic;
            if state.parent != Some(category_id) || topic_differs {
                self.report.changes.push(Change::EditChannel {
                    name: channel.name.clone(),
                    kind: channel.kind,
                    category: category.name.clone(),
                    topic: topic
                        .filter(|_| topic_differs)
                        .map_or(TopicEdit::Unchanged, |topic| {
                            TopicEdit::Set(topic.to_string())
                        }),
                });
            }
            let planned = self.plan.effective_overwrites(category, channel);
            let target = Target::Channel {
                name: channel.name.clone(),
                kind: channel.kind,
            };
            self.overwrite_changes(target, planned, state, reveal_permits);
        }
    }

    fn overwrites(&mut self, category_name: &str) {
        let Some(category) = self.plan.category(category_name) else {
            let names: Vec<&str> = self
                .plan
                .categories
                .iter()
                .map(|c| c.name.as_str())
                .collect();
            self.report.blockers.push(format!(
                "the plan has no category {category_name:?}; it has: {}",
                names.join(", ")
            ));
            return;
        };
        let category_id = match self.category_lookup(&category.name) {
            Lookup::Found(id) => id,
            Lookup::Missing => {
                self.report.blockers.push(format!(
                    "category {:?} does not exist yet; run --stage additive first",
                    category.name
                ));
                return;
            }
            Lookup::Ambiguous(n) => {
                self.report
                    .blockers
                    .push(format!("category {:?} exists {n} times", category.name));
                return;
            }
        };
        let state = self
            .snapshot
            .channels
            .iter()
            .find(|c| c.id == category_id)
            .expect("live id");
        self.lockout_warning(
            &Target::Category(category.name.clone()),
            &category.overwrites,
            state,
        );
        self.overwrite_changes(
            Target::Category(category.name.clone()),
            &category.overwrites,
            state,
            always_permits,
        );
        for channel in &category.channels {
            let Some(state) = self.require_channel(channel) else {
                continue;
            };
            let planned = self.plan.effective_overwrites(category, channel);
            let target = Target::Channel {
                name: channel.name.clone(),
                kind: channel.kind,
            };
            self.lockout_warning(&target, planned, state);
            self.overwrite_changes(target, planned, state, always_permits);
        }
    }

    /// 3.B.9 in both directions: a channel people can see is about to be
    /// gated, or a channel they cannot see is about to open.
    fn lockout_warning(&mut self, target: &Target, planned: &[Overwrite], state: &ChannelState) {
        let Some(planned_everyone) = planned.iter().find(|o| o.role == EVERYONE) else {
            return;
        };
        let current = state.overwrite_for_role(self.snapshot.guild_id);
        let current_denies_view = current.is_some_and(|o| {
            o.deny
                .iter()
                .any(|p| p == crate::server::plan::VIEW_CHANNEL)
        });
        let current_is_marker = is_engine_hidden(state, self.snapshot.guild_id);
        if planned_everyone.denies_view() && !current_denies_view {
            let allowed: Vec<&str> = planned
                .iter()
                .filter(|o| {
                    o.role != EVERYONE
                        && o.allow
                            .iter()
                            .any(|p| p == crate::server::plan::VIEW_CHANNEL)
                })
                .map(|o| o.role.as_str())
                .collect();
            self.report.warnings.push(format!(
                "{} is visible to @everyone now and will be gated to [{}]: every member without one of those roles loses it at that instant (3.B.9). Backfill the roles and verify the count first.",
                target.describe(),
                allowed.join(", ")
            ));
        } else if !planned_everyone.denies_view() && current_denies_view && !current_is_marker {
            self.report.warnings.push(format!(
                "{} is hidden from @everyone by an overwrite the engine did not place, and the plan makes it visible",
                target.describe()
            ));
        }
    }

    fn require_channel(&mut self, channel: &PlanChannel) -> Option<&'a ChannelState> {
        let snapshot: &'a GuildSnapshot = self.snapshot;
        match self.channel_lookup(channel) {
            Lookup::Found(id) => snapshot.channels.iter().find(|c| c.id == id),
            Lookup::Missing => {
                self.report.blockers.push(format!(
                    "{} does not exist yet; run --stage additive first",
                    describe_channel(channel.kind, &channel.name)
                ));
                None
            }
            Lookup::Ambiguous(n) => {
                self.report.blockers.push(format!(
                    "{} exists {n} times",
                    describe_channel(channel.kind, &channel.name)
                ));
                None
            }
        }
    }

    fn state_to_overwrite(&self, state: &OverwriteState) -> Overwrite {
        let role = match state.target {
            OverwriteTarget::Role(id) if id == self.snapshot.guild_id => EVERYONE.to_string(),
            OverwriteTarget::Role(id) => self
                .snapshot
                .role_by_id(id)
                .map_or_else(|| format!("role {id}"), |r| r.name.clone()),
            OverwriteTarget::Member(id) => format!("member {id}"),
        };
        Overwrite {
            role,
            allow: sorted_unique(&state.allow),
            deny: sorted_unique(&state.deny),
        }
    }

    fn overwrite_changes(
        &mut self,
        target: Target,
        planned: &[Overwrite],
        state: &ChannelState,
        permits: fn(Option<&Overwrite>, &Overwrite, bool) -> bool,
    ) {
        let mut wanted: Vec<Overwrite> = planned.iter().map(Overwrite::normalized).collect();
        let engine_hidden = is_engine_hidden(state, self.snapshot.guild_id);
        // Engine-hidden with no planned @everyone entry means "public": clear
        // the marker (set it empty; nothing is deleted).
        if !wanted.iter().any(|o| o.role == EVERYONE) && engine_hidden {
            wanted.push(Overwrite {
                role: EVERYONE.into(),
                allow: Vec::new(),
                deny: Vec::new(),
            });
        }
        for overwrite in wanted {
            let role_id = if overwrite.role == EVERYONE {
                self.snapshot.guild_id
            } else {
                match self.role_lookup(&overwrite.role) {
                    RoleLookup::Found(id) => id,
                    RoleLookup::Missing => {
                        self.report.blockers.push(format!(
                            "{}: overwrite for role {:?}, which does not exist yet; run --stage additive first",
                            target.describe(),
                            overwrite.role
                        ));
                        continue;
                    }
                    RoleLookup::Managed => {
                        self.report
                            .blockers
                            .push(Self::managed_blocker(&overwrite.role));
                        continue;
                    }
                    RoleLookup::Ambiguous(n) => {
                        self.report.blockers.push(format!(
                            "{}: role {:?} exists {n} times",
                            target.describe(),
                            overwrite.role
                        ));
                        continue;
                    }
                }
            };
            let current = state
                .overwrite_for_role(role_id)
                .map(|s| self.state_to_overwrite(s));
            let current_cmp = current.as_ref().map(|c| Overwrite {
                role: overwrite.role.clone(),
                ..c.clone()
            });
            if current_cmp.as_ref() == Some(&overwrite)
                || (current.is_none() && overwrite.is_empty())
            {
                continue;
            }
            if permits(current_cmp.as_ref(), &overwrite, engine_hidden) {
                self.report.changes.push(Change::SetOverwrite {
                    target: target.clone(),
                    overwrite,
                });
            } else {
                self.report.warnings.push(format!(
                    "{}: the @everyone view gate differs from the plan; --stage reveal leaves it alone (3.B.9), use --stage overwrites --category for that category",
                    target.describe()
                ));
            }
        }
    }

    fn manual_steps(&mut self) {
        let plan = self.plan;
        let snap = self.snapshot;
        if let Some(everyone) = snap.everyone_role() {
            let current: BTreeSet<&str> = everyone.permissions.iter().map(String::as_str).collect();
            let wanted: BTreeSet<&str> = plan
                .everyone
                .permissions
                .iter()
                .map(String::as_str)
                .collect();
            let grant: Vec<&str> = wanted.difference(&current).copied().collect();
            let revoke: Vec<&str> = current.difference(&wanted).copied().collect();
            if !grant.is_empty() || !revoke.is_empty() {
                self.report.manual.push(format!(
                    "@everyone guild permissions differ from the plan: missing [{}]; extra [{}] (the engine never edits an existing role's permissions; decide each one by hand)",
                    grant.join(", "),
                    revoke.join(", ")
                ));
            }
        }
        for role in &plan.roles {
            let wanted: BTreeSet<&str> = role.permissions.iter().map(String::as_str).collect();
            match self.role_lookup(&role.name) {
                RoleLookup::Found(id) => {
                    let current = snap.role_by_id(id).expect("live id");
                    let held: BTreeSet<&str> =
                        current.permissions.iter().map(String::as_str).collect();
                    let grant: Vec<&str> = wanted.difference(&held).copied().collect();
                    let revoke: Vec<&str> = held.difference(&wanted).copied().collect();
                    if !grant.is_empty() || !revoke.is_empty() {
                        self.report.manual.push(format!(
                            "role {:?} permissions differ from the plan: missing [{}]; extra [{}] (review by hand; guild permissions add to @everyone's, so an extra is not always wrong)",
                            role.name,
                            grant.join(", "),
                            revoke.join(", ")
                        ));
                    }
                }
                RoleLookup::Missing if !wanted.is_empty() => self.report.manual.push(format!(
                    "after --stage additive, grant role {:?}: [{}]",
                    role.name,
                    role.permissions.join(", ")
                )),
                RoleLookup::Missing | RoleLookup::Managed | RoleLookup::Ambiguous(_) => {}
            }
        }
        let positions: Vec<(String, Option<u16>)> = plan
            .roles
            .iter()
            .map(|r| {
                let position = match self.role_lookup(&r.name) {
                    RoleLookup::Found(id) => snap.role_by_id(id).map(|s| s.position),
                    RoleLookup::Missing | RoleLookup::Managed | RoleLookup::Ambiguous(_) => None,
                };
                (r.name.clone(), position)
            })
            .collect();
        let known: Vec<u16> = positions.iter().filter_map(|(_, p)| *p).collect();
        let ordered = known.windows(2).all(|w| w[0] > w[1]);
        if plan.roles.len() > 1 && (!ordered || known.len() < positions.len()) {
            let order: Vec<&str> = positions.iter().map(|(n, _)| n.as_str()).collect();
            self.report.manual.push(format!(
                "order roles highest to lowest: {} (the engine never repositions roles, 3.B.8; keep every integration role below Team)",
                order.join(" > ")
            ));
        }
        if plan.needs_community() && !snap.has_feature(COMMUNITY_FEATURE) {
            self.report.manual.push(
                "enable Community mode (Server Settings → Community); it needs a rules channel and a public updates channel to exist, and forums, announcement channels, and stages cannot be created without it".into(),
            );
        }
    }

    fn check_bot_capability(&mut self) {
        if self.report.changes.is_empty() || self.snapshot.bot_is_administrator() {
            return;
        }
        let held = self.snapshot.bot_permissions();
        if !held.contains("Manage Channels") {
            self.report
                .blockers
                .push("the bot lacks Manage Channels".into());
        }
        if self.report.changes.iter().any(Change::needs_manage_roles)
            && !held.contains("Manage Roles")
        {
            self.report.blockers.push(
                "the bot lacks Manage Roles (needed for roles and permission overwrites)".into(),
            );
        }
        let mut missing: BTreeSet<&str> = BTreeSet::new();
        for change in &self.report.changes {
            for name in change.overwrite_permission_names() {
                if !held.contains(name) {
                    missing.insert(name);
                }
            }
        }
        if !missing.is_empty() {
            let missing: Vec<&str> = missing.into_iter().collect();
            self.report.blockers.push(format!(
                "the bot cannot grant or deny permissions it does not hold itself: [{}]",
                missing.join(", ")
            ));
        }
    }

    fn role_lookup(&self, name: &str) -> RoleLookup {
        let matches = self.snapshot.roles_named(name);
        match matches.as_slice() {
            [] => RoleLookup::Missing,
            [one] if one.managed => RoleLookup::Managed,
            [one] => RoleLookup::Found(one.id),
            many => RoleLookup::Ambiguous(many.len()),
        }
    }

    fn managed_blocker(name: &str) -> String {
        format!(
            "role {name:?} is an integration-managed role (a bot's); the plan role of that name can never be it. Rename the plan role"
        )
    }

    fn category_lookup(&self, name: &str) -> Lookup {
        let matches = self.snapshot.categories_named(name);
        match matches.as_slice() {
            [] => Lookup::Missing,
            [one] => Lookup::Found(one.id),
            many => Lookup::Ambiguous(many.len()),
        }
    }

    fn channel_lookup(&self, channel: &PlanChannel) -> Lookup {
        let matches = self.snapshot.channels_matching(&channel.name, channel.kind);
        match matches.as_slice() {
            [] => Lookup::Missing,
            [one] => Lookup::Found(one.id),
            many => Lookup::Ambiguous(many.len()),
        }
    }
}

enum Lookup {
    Missing,
    Found(u64),
    Ambiguous(usize),
}

enum RoleLookup {
    Missing,
    Found(u64),
    /// Exactly one match, owned by an integration. A plan role can never be
    /// that role, so every stage refuses rather than editing a bot's role.
    Managed,
    Ambiguous(usize),
}
