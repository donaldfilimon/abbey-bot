//! Scoped, caller-time feedback attribution. No native identity lookup or I/O.
use super::recovery::{MAX_RECOVERY_ENTRIES, ReactionContribution, ReactionRecord};
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeedbackAttribution {
    ExactReply,
    UniqueScoped,
    Duplicate,
    Ambiguous,
    Expired,
    Unsupported,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct ReactionKey {
    pub scope: String,
    pub message: String,
    pub reactor_hash: u64,
    pub emoji: String,
}

fn open(p: &Pending, now: u64) -> bool {
    !p.settle_immediately && now >= p.created_at && now - p.created_at <= ATTRIBUTION_TTL_SECS
}

impl RewardCollector {
    /// Exact references never fall back to another turn. Without a reference,
    /// all open scoped turns are plausible: refuse ambiguity rather than guess.
    pub fn attribution<'a>(
        &'a self,
        scope: &str,
        reply_to: Option<&str>,
        now: u64,
    ) -> Result<(FeedbackAttribution, &'a str), FeedbackAttribution> {
        if scope.is_empty() {
            return Err(FeedbackAttribution::Unsupported);
        }
        if let Some(id) = reply_to {
            let Some((key, p)) = self.pending.get_key_value(id) else {
                return Err(FeedbackAttribution::Expired);
            };
            if p.scope != scope {
                return Err(FeedbackAttribution::Unsupported);
            }
            return if open(p, now) {
                Ok((FeedbackAttribution::ExactReply, key))
            } else {
                Err(FeedbackAttribution::Expired)
            };
        }
        let mut candidates = self.pending.iter().filter(|(_, p)| {
            p.scope == scope && p.action != BotAction::React.index() && open(p, now)
        });
        let Some((id, _)) = candidates.next() else {
            return Err(FeedbackAttribution::Expired);
        };
        if candidates.next().is_some() {
            return Err(FeedbackAttribution::Ambiguous);
        }
        Ok((FeedbackAttribution::UniqueScoped, id))
    }

    pub fn observe_reply_to(
        &mut self,
        scope: &str,
        id: &str,
        outcome: ReplyOutcome,
        now: u64,
    ) -> bool {
        if self.attribution(scope, Some(id), now).is_err() {
            return false;
        }
        credit(
            self.pending.get_mut(id).expect("attributed pending turn"),
            outcome,
        );
        true
    }

    pub fn observe_in_scope(
        &mut self,
        scope: &str,
        observer: &str,
        outcome: ReplyOutcome,
        now: u64,
    ) -> Option<String> {
        let (_, id) = self.attribution(scope, None, now).ok()?;
        let id = id.to_owned();
        let p = self.pending.get_mut(&id)?;
        if observer.is_empty()
            || (outcome.needs_the_original_asker() && (p.asker.is_empty() || p.asker != observer))
        {
            return None;
        }
        credit(p, outcome);
        Some(id)
    }

    /// Classify only against the selected current turn. Immediate reply credit
    /// and delayed typed outcomes remain separate inputs to the existing blend.
    /// The shell has already verified this observer is a human with access.
    pub fn feedback(
        &mut self,
        scope: &str,
        observer: &str,
        reply_to: Option<&str>,
        text: &str,
        now: u64,
    ) -> FeedbackAttribution {
        if observer.is_empty() {
            return FeedbackAttribution::Unsupported;
        }
        let (source, id) = match self.attribution(scope, reply_to, now) {
            Ok((source, id)) => (source, id.to_owned()),
            Err(reason) => return reason,
        };
        if self.pending.get(&id).is_some_and(|p| {
            self.recovery
                .erasure
                .blocks(&p.scoped_guild_id, observer, now)
        }) {
            return FeedbackAttribution::Expired;
        }
        let observed = outcome::classify_signature(text, self.open_ask(&id))
            .unwrap_or(ReplyOutcome::NoEngagement);
        if source == FeedbackAttribution::ExactReply {
            self.human_replied(scope, &id, now);
            self.observe_reply_to(scope, &id, observed, now);
        } else if self
            .observe_in_scope(scope, observer, observed, now)
            .is_none()
        {
            return FeedbackAttribution::Unsupported;
        }
        source
    }

    /// Admission binds a real prior bot reply, never a reaction's human target.
    #[expect(
        clippy::too_many_arguments,
        reason = "bind correction to authenticated scoped event and caller time"
    )]
    pub(crate) fn correction_source(
        &self,
        scope: &str,
        guild: &str,
        observer: &str,
        reply_to: Option<&str>,
        text: &str,
        now: u64,
        authorized: bool,
    ) -> Option<crate::brain::correction::CorrectionSource> {
        use crate::brain::correction::{
            CorrectionDecision, CorrectionSource, evaluate_correction, explicit_correction,
        };
        if observer.is_empty()
            || !explicit_correction(text)
            || self.recovery.erasure.blocks(guild, observer, now)
        {
            return None;
        }
        let (attribution, id) = self.attribution(scope, reply_to, now).ok()?;
        let p = self.pending.get(id)?;
        if p.action != BotAction::Reply.index()
            || p.scoped_guild_id != guild
            || (attribution == FeedbackAttribution::UniqueScoped && p.asker != observer)
        {
            return None;
        }
        // The numeric token is not identity authority. Native identity below
        // remains bound to the ledger, including nonnumeric platform IDs.
        if !matches!(
            evaluate_correction(
                attribution,
                false,
                Some(crate::wyhash::hash(0, id.as_bytes())),
                authorized
            ),
            CorrectionDecision::Repair { .. }
        ) {
            return None;
        }
        Some(CorrectionSource {
            observer: observer.to_owned(),
            admitted_at: now,
            native_id: id.to_owned(),
            scope: scope.to_owned(),
            guild: guild.to_owned(),
            asker: p.asker.clone(),
            created_at: p.created_at,
            signature: p.ask_signature.clone(),
        })
    }

    pub(crate) fn correction_current(
        &self,
        source: &crate::brain::correction::CorrectionSource,
        now: u64,
    ) -> bool {
        !self
            .recovery
            .erasure
            .blocks(&source.guild, &source.observer, source.admitted_at)
            && self.pending.get(&source.native_id).is_some_and(|p| {
                open(p, now)
                    && p.action == BotAction::Reply.index()
                    && p.scope == source.scope
                    && p.scoped_guild_id == source.guild
                    && p.asker == source.asker
                    && p.created_at == source.created_at
                    && p.ask_signature == source.signature
            })
    }

    /// Supported active keys contribute once. A removal reverses only its
    /// recorded contribution (including a capped zero), then forgets the key.
    /// At saturation refuse new keys; never evict a contribution still pending.
    pub fn reaction(&mut self, key: ReactionKey, added: bool, now: u64) -> FeedbackAttribution {
        let positive = POSITIVE_EMOJI.contains(&key.emoji.as_str());
        if !positive && !NEGATIVE_EMOJI.contains(&key.emoji.as_str()) {
            return FeedbackAttribution::Unsupported;
        }
        if let Err(reason) = self.attribution(&key.scope, Some(&key.message), now) {
            return reason;
        }
        if self.pending.get(&key.message).is_some_and(|p| {
            self.recovery
                .erasure
                .blocks_hash(&p.scoped_guild_id, key.reactor_hash, now)
        }) {
            return FeedbackAttribution::Expired;
        }
        let p = self
            .pending
            .get_mut(&key.message)
            .expect("attributed pending turn");
        if !p.reaction_tracking {
            return FeedbackAttribution::Unsupported;
        }
        let existing = self.recovery.reactions.iter().position(|r| r.key == key);
        if added {
            if existing.is_some() {
                return FeedbackAttribution::Duplicate;
            }
            if self.recovery.reactions.len() >= MAX_RECOVERY_ENTRIES {
                return FeedbackAttribution::Unsupported;
            }
            let contribution = if positive && p.positive_reactions < MAX_POSITIVE_REACTIONS {
                p.reward += 1.0;
                p.positive_reactions += 1;
                ReactionContribution::Positive
            } else if positive {
                ReactionContribution::Capped
            } else {
                p.reward -= 1.0;
                ReactionContribution::Negative
            };
            self.recovery
                .reactions
                .push(ReactionRecord { key, contribution });
        } else {
            let Some(index) = existing else {
                return FeedbackAttribution::Duplicate;
            };
            match self.recovery.reactions.swap_remove(index).contribution {
                ReactionContribution::Positive => {
                    p.reward -= 1.0;
                    p.positive_reactions -= 1;
                }
                ReactionContribution::Negative => p.reward += 1.0,
                ReactionContribution::Capped => (),
            }
        }
        FeedbackAttribution::ExactReply
    }

    pub fn human_replied(&mut self, scope: &str, id: &str, now: u64) {
        if self.attribution(scope, Some(id), now).is_ok() {
            self.pending
                .get_mut(id)
                .expect("attributed pending turn")
                .reward += 0.5;
        }
    }
}
