//! Durable delivery outcome and late authorization checks. No transport I/O.
use super::*;

type DeliveryCandidate = (WorkScope, u64, u64, WorkDestination, BTreeSet<u64>);
impl WorkStore {
    pub(crate) fn delivery_candidates(&self) -> Vec<DeliveryCandidate> {
        let scopes: BTreeSet<_> = self.projects.values().map(|p| p.scope.clone()).collect();
        scopes
            .into_iter()
            .filter_map(|scope| {
                let policy = self.scope_automation.get(&scope.key())?;
                if !policy.enabled {
                    return None;
                }
                let actor = *self.scope_automation_actors.get(&scope.key())?;
                let origin = policy.destination?;
                let access = WorkAccess {
                    actor,
                    guild: match scope {
                        WorkScope::Team { guild, .. } => Some(guild),
                        _ => None,
                    },
                    channel: origin,
                    can_view: true,
                    can_manage: false,
                };
                let target = self.target(&scope, access, policy).ok()?;
                let audience = self.delivery_audience(&scope);
                Some((scope, actor, origin, target, audience))
            })
            .collect()
    }

    /// Every viewer must belong to every project in this scope. Conservative
    /// across omitted sources so later coalescing cannot widen the audience.
    pub(crate) fn delivery_audience(&self, scope: &WorkScope) -> BTreeSet<u64> {
        let mut projects = self.projects.values().filter(|p| &p.scope == scope);
        let Some(first) = projects.next() else {
            return BTreeSet::new();
        };
        projects.fold(first.members.clone(), |members, p| {
            members.intersection(&p.members).copied().collect()
        })
    }

    pub(crate) fn settle_delivery(
        &mut self,
        id: u64,
        message: Option<u64>,
    ) -> Result<(), WorkError> {
        let receipt = self.deliveries.get_mut(&id).ok_or(WorkError::Missing)?;
        if receipt.state != DeliveryState::Attempting {
            return Err(WorkError::Stale);
        }
        receipt.message_id = message.filter(|id| *id != 0);
        receipt.state = if receipt.message_id.is_some() {
            DeliveryState::Sent
        } else {
            DeliveryState::ReviewRequired
        };
        Ok(())
    }

    /// Loaded attempts might have reached Discord before a crash. Preserve all
    /// quota and dedupe coverage; recovery never schedules an automatic resend.
    pub(crate) fn mark_interrupted_deliveries(&mut self) {
        for receipt in self.deliveries.values_mut() {
            if receipt.state == DeliveryState::Attempting {
                receipt.state = DeliveryState::ReviewRequired;
                receipt.message_id = None;
            }
        }
    }
}
