//! The pure half of the plan engine: plan + snapshot + stage → changes.
//!
//! `diff` never touches Discord. It produces a [`Report`] whose `changes` are
//! exactly what `apply` would perform, plus the reasons it refuses
//! (`blockers`), the things it wants a human to know first (`warnings`), and
//! the work it leaves to a human on purpose (`manual`).
//!
//! The vocabulary is deliberately additive. [`Change`] has no variant that
//! deletes a role, a channel, or a category, and none that edits an existing
//! role's permissions (`@everyone`'s included): the proposal's stages 12 and
//! 13 are irreversible and stay human, and a role permission edit is a
//! hierarchy decision. A test enumerates the variants so a `Delete` cannot
//! slip in quietly.
//!
//! Stages mirror the proposal's rollout order (section 4):
//! - [`Stage::Additive`] creates what is missing and nothing else: roles with
//!   zero permissions, categories and channels hidden from `@everyone`
//!   ([`Overwrite::hide_marker`]). Nothing existing is touched.
//! - [`Stage::Reveal`] sets role cosmetics, moves plan channels into their
//!   categories, sets topics, and applies planned overwrites, with one
//!   guard: it changes what `@everyone` can *see* only from the engine's own
//!   hide marker to visible. Any other view-gate change on `@everyone` is
//!   deferred, because that is the 3.B.9 lockout.
//! - [`Stage::Overwrites`] applies every planned overwrite for one named
//!   category ("one category per day", stage 8), warning about each
//!   view-gate flip on a channel people can currently see.

use std::collections::BTreeSet;

use super::ChannelKind;
use super::observe::{
    COMMUNITY_FEATURE, ChannelState, GuildSnapshot, OverwriteState, OverwriteTarget,
};
use super::plan::{
    EVERYONE, Overwrite, Plan, PlanCategory, PlanChannel, colour_hex, sorted_unique,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Additive,
    Reveal,
    Overwrites,
}

impl Stage {
    /// Every stage, for the tests that must not silently skip one.
    #[cfg(test)]
    pub const ALL: [Self; 3] = [Self::Additive, Self::Reveal, Self::Overwrites];

    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "additive" => Some(Self::Additive),
            "reveal" => Some(Self::Reveal),
            "overwrites" => Some(Self::Overwrites),
            _ => None,
        }
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Additive => "additive",
            Self::Reveal => "reveal",
            Self::Overwrites => "overwrites",
        }
    }
}

/// What to diff: the stage, and for [`Stage::Overwrites`] the one category.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scope {
    pub stage: Stage,
    pub category: Option<String>,
}

/// A channel-like thing an overwrite lands on, by plan name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Category(String),
    Channel { name: String, kind: ChannelKind },
}

impl Target {
    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            Self::Category(name) => format!("category {name:?}"),
            Self::Channel { name, kind } => describe_channel(*kind, name),
        }
    }
}

fn describe_channel(kind: ChannelKind, name: &str) -> String {
    match kind {
        ChannelKind::Text | ChannelKind::Forum | ChannelKind::Announcement => {
            format!("{} #{name}", kind.label())
        }
        ChannelKind::Voice | ChannelKind::Stage => format!("{} {name:?}", kind.label()),
    }
}

/// One thing `apply` may do. Additive by construction: see the module doc.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    /// Zero permissions, unhoisted, not mentionable (proposal stage 1).
    CreateRole { name: String, colour: Option<u32> },
    /// Cosmetics only; permissions are never edited.
    EditRole {
        name: String,
        hoist: bool,
        mentionable: bool,
        colour: Option<u32>,
    },
    CreateCategory {
        name: String,
        overwrites: Vec<Overwrite>,
    },
    CreateChannel {
        name: String,
        kind: ChannelKind,
        category: String,
        topic: Option<String>,
        slowmode_secs: Option<u16>,
        tags: Vec<String>,
        overwrites: Vec<Overwrite>,
    },
    /// Parent category and topic; never the name, never the kind.
    EditChannel {
        name: String,
        kind: ChannelKind,
        category: String,
        topic: Option<String>,
    },
    /// One `PUT` for one role on one channel. Other roles' and members'
    /// entries on that channel are untouched (this is what keeps 3.B.6 out).
    SetOverwrite {
        target: Target,
        overwrite: Overwrite,
    },
}

impl Change {
    /// Every variant, spelled out. The exhaustiveness test walks this list.
    #[cfg(test)]
    pub const KIND_NAMES: [&'static str; 6] = [
        "CreateRole",
        "EditRole",
        "CreateCategory",
        "CreateChannel",
        "EditChannel",
        "SetOverwrite",
    ];

    #[cfg(test)]
    #[must_use]
    pub const fn kind_name(&self) -> &'static str {
        match self {
            Self::CreateRole { .. } => "CreateRole",
            Self::EditRole { .. } => "EditRole",
            Self::CreateCategory { .. } => "CreateCategory",
            Self::CreateChannel { .. } => "CreateChannel",
            Self::EditChannel { .. } => "EditChannel",
            Self::SetOverwrite { .. } => "SetOverwrite",
        }
    }

    /// Editing overwrites and roles needs Manage Roles on top of Manage Channels.
    #[must_use]
    pub const fn needs_manage_roles(&self) -> bool {
        matches!(
            self,
            Self::CreateRole { .. }
                | Self::EditRole { .. }
                | Self::SetOverwrite { .. }
                | Self::CreateCategory { .. }
                | Self::CreateChannel { .. }
        )
    }

    /// Permission names this change would grant or deny through overwrites.
    #[must_use]
    pub fn overwrite_permission_names(&self) -> BTreeSet<&str> {
        let overwrites: &[Overwrite] = match self {
            Self::CreateCategory { overwrites, .. } | Self::CreateChannel { overwrites, .. } => {
                overwrites
            }
            Self::SetOverwrite { overwrite, .. } => std::slice::from_ref(overwrite),
            Self::CreateRole { .. } | Self::EditRole { .. } | Self::EditChannel { .. } => &[],
        };
        overwrites
            .iter()
            .flat_map(|o| o.allow.iter().chain(o.deny.iter()))
            .map(String::as_str)
            .collect()
    }

    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            Self::CreateRole { name, colour } => format!(
                "create role {name:?} ({}; zero permissions, unhoisted, not mentionable)",
                colour.map_or_else(
                    || "no colour".to_string(),
                    |c| format!("colour {}", colour_hex(c))
                )
            ),
            Self::EditRole {
                name,
                hoist,
                mentionable,
                colour,
            } => format!(
                "edit role {name:?}: hoist {}, mentionable {}, {}",
                on_off(*hoist),
                on_off(*mentionable),
                colour.map_or_else(|| "no colour".to_string(), colour_hex)
            ),
            Self::CreateCategory { name, overwrites } => {
                format!(
                    "create category {name:?} ({})",
                    describe_overwrites(overwrites)
                )
            }
            Self::CreateChannel {
                name,
                kind,
                category,
                topic,
                slowmode_secs,
                tags,
                overwrites,
            } => {
                let mut extras = vec![describe_overwrites(overwrites)];
                if topic.is_some() {
                    extras.push("topic set".into());
                }
                if let Some(secs) = slowmode_secs {
                    extras.push(format!("slowmode {secs}s"));
                }
                if !tags.is_empty() {
                    extras.push(format!("{} tags", tags.len()));
                }
                format!(
                    "create {} in {category:?} ({})",
                    describe_channel(*kind, name),
                    extras.join("; ")
                )
            }
            Self::EditChannel {
                name,
                kind,
                category,
                topic,
            } => format!(
                "edit {}: parent {category:?}{}",
                describe_channel(*kind, name),
                if topic.is_some() { ", topic set" } else { "" }
            ),
            Self::SetOverwrite { target, overwrite } => {
                if overwrite.is_empty() {
                    format!(
                        "clear the {} overwrite on {}",
                        overwrite.role,
                        target.describe()
                    )
                } else {
                    format!(
                        "set the {} overwrite on {}: allow [{}], deny [{}]",
                        overwrite.role,
                        target.describe(),
                        overwrite.allow.join(", "),
                        overwrite.deny.join(", ")
                    )
                }
            }
        }
    }
}

const fn on_off(value: bool) -> &'static str {
    if value { "on" } else { "off" }
}

fn describe_overwrites(overwrites: &[Overwrite]) -> String {
    if overwrites.len() == 1 && overwrites[0].is_hide_marker() {
        "hidden from @everyone until --stage reveal".into()
    } else if overwrites.is_empty() {
        "no overwrites".into()
    } else {
        format!("{} overwrites", overwrites.len())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub stage: Stage,
    /// Facts the decision rested on, for the operator's eyes.
    pub preflight: Vec<String>,
    pub changes: Vec<Change>,
    /// Any of these means `apply` is refused.
    pub blockers: Vec<String>,
    pub warnings: Vec<String>,
    /// Work the engine leaves to a human on purpose.
    pub manual: Vec<String>,
}

impl Report {
    #[must_use]
    pub fn is_clear(&self) -> bool {
        self.blockers.is_empty()
    }

    /// Operator-facing rendering. `heading` names the plan, guild, and mode.
    #[must_use]
    pub fn render(&self, heading: &str) -> String {
        let mut out = String::new();
        out.push_str(heading);
        out.push('\n');
        for line in &self.preflight {
            out.push_str(&format!("preflight: {line}\n"));
        }
        out.push_str(&format!("\nchanges ({})\n", self.changes.len()));
        for (index, change) in self.changes.iter().enumerate() {
            out.push_str(&format!("  {}. {}\n", index + 1, change.describe()));
        }
        if !self.blockers.is_empty() {
            out.push_str(&format!(
                "\nblockers ({}): nothing will be applied\n",
                self.blockers.len()
            ));
            for line in &self.blockers {
                out.push_str(&format!("  - {line}\n"));
            }
        }
        if !self.warnings.is_empty() {
            out.push_str(&format!("\nwarnings ({})\n", self.warnings.len()));
            for line in &self.warnings {
                out.push_str(&format!("  - {line}\n"));
            }
        }
        if !self.manual.is_empty() {
            out.push_str(&format!(
                "\nmanual steps ({}): the engine never does these\n",
                self.manual.len()
            ));
            for (index, line) in self.manual.iter().enumerate() {
                out.push_str(&format!("  {}. {line}\n", index + 1));
            }
        }
        out
    }
}

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
fn reveal_permits(current: Option<&Overwrite>, planned: &Overwrite, engine_hidden: bool) -> bool {
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
fn is_engine_hidden(state: &ChannelState, guild_id: u64) -> bool {
    match state.overwrites.as_slice() {
        [only] => {
            only.target == OverwriteTarget::Role(guild_id)
                && only.allow.is_empty()
                && sorted_unique(&only.deny) == [super::plan::VIEW_CHANNEL]
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
            let topic_differs = channel.kind.has_topic() && topic != current_topic;
            if state.parent != Some(category_id) || topic_differs {
                self.report.changes.push(Change::EditChannel {
                    name: channel.name.clone(),
                    kind: channel.kind,
                    category: category.name.clone(),
                    topic: if topic_differs {
                        topic.map(str::to_string)
                    } else {
                        None
                    },
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
        let current_denies_view =
            current.is_some_and(|o| o.deny.iter().any(|p| p == super::plan::VIEW_CHANNEL));
        let current_is_marker = is_engine_hidden(state, self.snapshot.guild_id);
        if planned_everyone.denies_view() && !current_denies_view {
            let allowed: Vec<&str> = planned
                .iter()
                .filter(|o| {
                    o.role != EVERYONE && o.allow.iter().any(|p| p == super::plan::VIEW_CHANNEL)
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
                    "@everyone guild permissions: grant [{}]; revoke [{}] (the engine never edits an existing role's permissions)",
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
                            "role {:?} permissions: grant [{}]; revoke [{}]",
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::observe::{BotState, ChannelClass, RoleState};
    use crate::server::plan::{MLAI_COMMUNITY, VIEW_CHANNEL};

    const GUILD: u64 = 500;

    fn role(id: u64, name: &str, position: u16, permissions: &[&str]) -> RoleState {
        RoleState {
            id,
            name: name.into(),
            position,
            managed: false,
            hoist: false,
            mentionable: false,
            colour: 0,
            permissions: permissions.iter().map(|p| (*p).into()).collect(),
        }
    }

    fn guild(bot_permissions: &[&str]) -> GuildSnapshot {
        GuildSnapshot {
            guild_id: GUILD,
            features: vec![COMMUNITY_FEATURE.into()],
            roles: vec![
                role(GUILD, "@everyone", 0, &["View Channel"]),
                RoleState {
                    managed: true,
                    ..role(501, "Abbey Bot", 4, bot_permissions)
                },
            ],
            channels: Vec::new(),
            bot: BotState {
                user_id: 1,
                role_ids: vec![501],
            },
        }
    }

    fn mlai() -> Plan {
        Plan::from_toml(MLAI_COMMUNITY).unwrap()
    }

    fn sample_changes() -> Vec<Change> {
        vec![
            Change::CreateRole {
                name: "r".into(),
                colour: None,
            },
            Change::EditRole {
                name: "r".into(),
                hoist: true,
                mentionable: false,
                colour: Some(1),
            },
            Change::CreateCategory {
                name: "c".into(),
                overwrites: vec![Overwrite::hide_marker()],
            },
            Change::CreateChannel {
                name: "t".into(),
                kind: ChannelKind::Text,
                category: "c".into(),
                topic: Some("x".into()),
                slowmode_secs: Some(5),
                tags: vec![],
                overwrites: vec![Overwrite::hide_marker()],
            },
            Change::EditChannel {
                name: "t".into(),
                kind: ChannelKind::Text,
                category: "c".into(),
                topic: None,
            },
            Change::SetOverwrite {
                target: Target::Channel {
                    name: "t".into(),
                    kind: ChannelKind::Text,
                },
                overwrite: Overwrite {
                    role: EVERYONE.into(),
                    allow: vec![],
                    deny: vec![],
                },
            },
        ]
    }

    #[test]
    fn every_change_variant_is_additive_and_none_can_delete() {
        // If a variant is added, `KIND_NAMES` and this sample must both grow,
        // and the name check keeps "Delete"/"Remove" out of the vocabulary.
        let sample = sample_changes();
        assert_eq!(sample.len(), Change::KIND_NAMES.len());
        for (change, expected) in sample.iter().zip(Change::KIND_NAMES) {
            assert_eq!(change.kind_name(), expected);
        }
        for name in Change::KIND_NAMES {
            let lower = name.to_ascii_lowercase();
            assert!(
                !lower.contains("delete")
                    && !lower.contains("remove")
                    && !lower.contains("permission"),
                "{name}"
            );
        }
        for change in &sample {
            let text = change.describe();
            assert!(!text.is_empty() && !text.contains("Permissions("), "{text}");
        }
    }

    #[test]
    fn reveal_permits_exactly_the_marker_to_visible_transition_for_everyone() {
        let marker = Overwrite::hide_marker();
        let public = Overwrite {
            role: EVERYONE.into(),
            allow: vec![],
            deny: vec!["Send Messages".into()],
        };
        let gated = Overwrite {
            role: EVERYONE.into(),
            allow: vec![],
            deny: vec![VIEW_CHANNEL.into(), "Send Messages".into()],
        };
        let hand_hidden = Overwrite {
            role: EVERYONE.into(),
            allow: vec!["Connect".into()],
            deny: vec![VIEW_CHANNEL.into()],
        };
        assert!(
            reveal_permits(Some(&marker), &public, true),
            "engine-hidden → visible"
        );
        assert!(
            !reveal_permits(Some(&marker), &public, false),
            "the same entry beside a role allow is a hand gate"
        );
        assert!(reveal_permits(None, &public, false), "no view change");
        assert!(
            reveal_permits(Some(&gated), &gated, false),
            "hidden stays hidden"
        );
        assert!(
            !reveal_permits(None, &gated, false),
            "gating a visible channel is 3.B.9"
        );
        assert!(
            !reveal_permits(Some(&hand_hidden), &public, false),
            "opening a hand-gated channel waits for --stage overwrites"
        );
        let role = Overwrite {
            role: "Team".into(),
            allow: vec![VIEW_CHANNEL.into()],
            deny: vec![],
        };
        assert!(
            reveal_permits(None, &role, false),
            "roles are never a lockout"
        );

        let marker_only = ChannelState {
            id: 1,
            name: "x".into(),
            class: ChannelClass::Kind(ChannelKind::Text),
            parent: None,
            topic: None,
            overwrites: vec![OverwriteState {
                target: OverwriteTarget::Role(GUILD),
                allow: vec![],
                deny: vec![VIEW_CHANNEL.into()],
            }],
        };
        assert!(is_engine_hidden(&marker_only, GUILD));
        let mut with_role = marker_only.clone();
        with_role.overwrites.push(OverwriteState {
            target: OverwriteTarget::Role(9),
            allow: vec![VIEW_CHANNEL.into()],
            deny: vec![],
        });
        assert!(!is_engine_hidden(&with_role, GUILD));
        let mut wider = marker_only;
        wider.overwrites[0].deny.push("Send Messages".into());
        assert!(!is_engine_hidden(&wider, GUILD));
    }

    #[test]
    fn additive_on_an_empty_guild_creates_everything_hidden_and_powerless() {
        let plan = mlai();
        let report = diff(
            &plan,
            &guild(&["Administrator"]),
            &Scope {
                stage: Stage::Additive,
                category: None,
            },
        );
        assert!(report.is_clear(), "{:?}", report.blockers);
        let roles = report
            .changes
            .iter()
            .filter(|c| matches!(c, Change::CreateRole { .. }))
            .count();
        assert_eq!(roles, plan.roles.len());
        for change in &report.changes {
            match change {
                Change::CreateCategory { overwrites, .. }
                | Change::CreateChannel { overwrites, .. } => {
                    assert_eq!(overwrites, &[Overwrite::hide_marker()]);
                }
                Change::CreateRole { .. } => {}
                other => panic!("additive emitted {}", other.kind_name()),
            }
        }
        assert!(
            report
                .manual
                .iter()
                .any(|m| m.contains("after --stage additive, grant role \"Team\"")),
            "{:?}",
            report.manual
        );
        assert!(
            report.manual.iter().any(|m| m.starts_with("order roles")),
            "{:?}",
            report.manual
        );
        assert!(
            report
                .manual
                .iter()
                .any(|m| m.starts_with("@everyone guild permissions")),
            "{:?}",
            report.manual
        );
        let rendered = report.render("heading");
        assert!(
            rendered.starts_with("heading\n") && rendered.contains("manual steps"),
            "{rendered}"
        );
    }

    #[test]
    fn the_bot_needs_manage_channels_and_manage_roles_unless_administrator() {
        let plan = mlai();
        let scope = Scope {
            stage: Stage::Additive,
            category: None,
        };
        let report = diff(&plan, &guild(&["Manage Channels"]), &scope);
        assert!(
            report.blockers.iter().any(|b| b.contains("Manage Roles")),
            "{:?}",
            report.blockers
        );
        let report = diff(&plan, &guild(&["Manage Roles"]), &scope);
        assert!(
            report
                .blockers
                .iter()
                .any(|b| b.contains("Manage Channels")),
            "{:?}",
            report.blockers
        );
        // Holding both but not View Channel: the hide marker denies a
        // permission the bot does not hold, which Discord refuses.
        let mut snapshot = guild(&["Manage Channels", "Manage Roles"]);
        snapshot.roles[0].permissions.clear();
        let report = diff(&plan, &snapshot, &scope);
        assert!(
            report
                .blockers
                .iter()
                .any(|b| b.contains("does not hold itself") && b.contains(VIEW_CHANNEL)),
            "{:?}",
            report.blockers
        );
        let report = diff(&plan, &guild(&["Manage Channels", "Manage Roles"]), &scope);
        assert!(report.is_clear(), "{:?}", report.blockers);
    }

    #[test]
    fn community_kinds_are_blocked_without_the_feature() {
        let plan = mlai();
        let mut snapshot = guild(&["Administrator"]);
        snapshot.features.clear();
        let report = diff(
            &plan,
            &snapshot,
            &Scope {
                stage: Stage::Additive,
                category: None,
            },
        );
        assert!(
            report
                .blockers
                .iter()
                .any(|b| b.contains("forum #help") && b.contains("Community mode")),
            "{:?}",
            report.blockers
        );
        assert!(
            report
                .manual
                .iter()
                .any(|m| m.contains("enable Community mode")),
            "{:?}",
            report.manual
        );
    }

    #[test]
    fn ambiguous_names_block_rather_than_guess() {
        let plan = mlai();
        let mut snapshot = guild(&["Administrator"]);
        snapshot.roles.push(role(600, "Team", 2, &[]));
        snapshot.roles.push(role(601, "Team", 1, &[]));
        let report = diff(
            &plan,
            &snapshot,
            &Scope {
                stage: Stage::Additive,
                category: None,
            },
        );
        assert!(
            report
                .blockers
                .iter()
                .any(|b| b.contains("\"Team\" exists 2 times")),
            "{:?}",
            report.blockers
        );
        assert!(
            !report
                .changes
                .iter()
                .any(|c| matches!(c, Change::CreateRole { name, .. } if name == "Team"))
        );
    }

    #[test]
    fn a_plan_role_that_matches_a_bots_managed_role_is_a_blocker() {
        // The live MLAI guild's bot is named Abbey, and the plan has an
        // interest role named Abbey. Guessing would edit the bot's role.
        let plan = mlai();
        let mut snapshot = guild(&["Administrator"]);
        snapshot.roles.push(RoleState {
            managed: true,
            ..role(650, "Abbey", 3, &[])
        });
        // The overwrites stage checks roles only inside an existing category,
        // so it is covered by the rollout tests in `apply`; the two stages
        // that walk every plan role are checked here.
        for stage in [Stage::Additive, Stage::Reveal] {
            let report = diff(
                &plan,
                &snapshot,
                &Scope {
                    stage,
                    category: None,
                },
            );
            assert!(
                report
                    .blockers
                    .iter()
                    .any(|b| b.contains("\"Abbey\" is an integration-managed role")),
                "{stage:?}: {:?}",
                report.blockers
            );
        }
    }

    #[test]
    fn reveal_and_overwrites_require_additive_to_have_run() {
        let plan = mlai();
        let report = diff(
            &plan,
            &guild(&["Administrator"]),
            &Scope {
                stage: Stage::Reveal,
                category: None,
            },
        );
        assert!(
            report
                .blockers
                .iter()
                .any(|b| b.contains("run --stage additive first")),
            "{:?}",
            report.blockers
        );
        let report = diff(
            &plan,
            &guild(&["Administrator"]),
            &Scope {
                stage: Stage::Overwrites,
                category: Some("STAFF".into()),
            },
        );
        assert!(
            report
                .blockers
                .iter()
                .any(|b| b.contains("run --stage additive first")),
            "{:?}",
            report.blockers
        );
        let report = diff(
            &plan,
            &guild(&["Administrator"]),
            &Scope {
                stage: Stage::Overwrites,
                category: None,
            },
        );
        assert!(
            report
                .blockers
                .iter()
                .any(|b| b.contains("needs --category")),
            "{:?}",
            report.blockers
        );
        let report = diff(
            &plan,
            &guild(&["Administrator"]),
            &Scope {
                stage: Stage::Overwrites,
                category: Some("NOPE".into()),
            },
        );
        assert!(
            report
                .blockers
                .iter()
                .any(|b| b.contains("no category \"NOPE\"") && b.contains("STAFF")),
            "{:?}",
            report.blockers
        );
    }

    #[test]
    fn reveal_refuses_to_edit_a_role_at_or_above_the_bot_and_skips_managed_ones() {
        let plan = mlai();
        let mut snapshot = guild(&["Administrator"]);
        for (index, role_spec) in plan.roles.iter().enumerate() {
            let id = 700 + index as u64;
            snapshot.roles.push(role(id, &role_spec.name, 1, &[]));
        }
        for category in &plan.categories {
            let id = 800 + snapshot.channels.len() as u64;
            snapshot.channels.push(ChannelState {
                id,
                name: category.name.clone(),
                class: ChannelClass::Category,
                parent: None,
                topic: None,
                overwrites: vec![],
            });
            for channel in &category.channels {
                let cid = 900 + snapshot.channels.len() as u64;
                snapshot.channels.push(ChannelState {
                    id: cid,
                    name: channel.name.clone(),
                    class: ChannelClass::Kind(channel.kind),
                    parent: Some(id),
                    topic: channel.topic.clone(),
                    overwrites: vec![],
                });
            }
        }
        // Team sits above the bot; Moderator is integration-managed.
        snapshot
            .roles
            .iter_mut()
            .find(|r| r.name == "Team")
            .unwrap()
            .position = 9;
        snapshot
            .roles
            .iter_mut()
            .find(|r| r.name == "Moderator")
            .unwrap()
            .managed = true;
        let report = diff(
            &plan,
            &snapshot,
            &Scope {
                stage: Stage::Reveal,
                category: None,
            },
        );
        assert!(
            report
                .blockers
                .iter()
                .any(|b| b.contains("\"Team\" sits at position 9")),
            "{:?}",
            report.blockers
        );
        assert!(
            report
                .blockers
                .iter()
                .any(|b| b.contains("\"Moderator\" is an integration-managed role")),
            "{:?}",
            report.blockers
        );
        assert!(!report.changes.iter().any(
            |c| matches!(c, Change::EditRole { name, .. } if name == "Team" || name == "Moderator")
        ));
        assert!(report.changes.iter().any(
            |c| matches!(c, Change::EditRole { name, hoist: true, .. } if name == "Contributor")
        ));
        // Roles that exist with matching permissions produce no manual grant,
        // and the order step names the plan order.
        assert!(
            report
                .manual
                .iter()
                .any(|m| m.starts_with("order roles") && m.contains("Owner > Team > Moderator")),
            "{:?}",
            report.manual
        );
    }

    #[test]
    fn every_stage_reports_the_same_preflight_facts() {
        let plan = mlai();
        for stage in Stage::ALL {
            let scope = Scope {
                stage,
                category: (stage == Stage::Overwrites).then(|| "STAFF".to_string()),
            };
            let report = diff(&plan, &guild(&["Administrator"]), &scope);
            assert_eq!(report.stage, stage);
            assert!(
                report.preflight[0].contains("Administrator"),
                "{:?}",
                report.preflight
            );
            assert!(
                report.preflight[1].contains("COMMUNITY feature: present"),
                "{:?}",
                report.preflight
            );
            assert_eq!(Stage::parse(stage.label()), Some(stage));
        }
        assert_eq!(Stage::parse("delete"), None);
    }
}
