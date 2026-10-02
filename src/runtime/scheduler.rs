//! Runtime work dispatched by the single retained scheduler.
use super::*;

pub(crate) trait CommunityMaintenance: Send + Sync {
    fn maintain(
        &self,
        state: Arc<AppState>,
        cancel: tokio_util::sync::CancellationToken,
        now: u64,
    ) -> impl std::future::Future<Output = Result<(), &'static str>> + Send;
}

impl AppState {
    /// Rolling channel summaries — the spec's "rolling 2k-token summary
    /// compressed via ABI". For every channel whose count is
    /// [`crate::memory::SUMMARY_EVERY_MESSAGES`] past its last summary, and
    /// whose guild has opted in (`/admin act on`) or is a DM, ask the backend
    /// for a summary of the recent lines and store it as the channel's
    /// context. One generation at a time, through the usual slot, so it never
    /// starves a live reply. Returns how many channels were summarised.
    pub async fn refresh_summaries(&self) -> usize {
        if !self.providers.generation_available() {
            return 0;
        }
        let due: Vec<String> = Self::lock(&self.stores).memory.channels_due_for_summary();
        let mut done = 0;
        for scoped_channel in due {
            // Only where Abbey has been invited to pay attention.
            let Some(guild) = guild_of_channel(&Self::lock(&self.stores), &scoped_channel) else {
                continue;
            };
            let invited = guild.contains(":dm:") || {
                let mut stores = Self::lock(&self.stores);
                Self::lock(&self.guilds)
                    .config(&guild, &mut *stores)
                    .unsolicited
            };
            if !invited {
                continue;
            }
            let (transcript, count) = {
                let mut stores = Self::lock(&self.stores);
                let ctx = stores.memory.channel_mut(&scoped_channel);
                (
                    ctx.render_recent(crate::memory::RECENT_CAP),
                    ctx.recent.len(),
                )
            };
            if transcript.trim().is_empty() {
                continue;
            }
            let (system, user) =
                crate::engine::summarize_prompt(crate::persona::Persona::Abbey, &transcript, count);
            match self
                .chat(&system, &[crate::llm::ChatTurn::user(user)])
                .await
            {
                Ok((summary, _)) => {
                    let summary = crate::ask::tidy_reply(crate::persona::Persona::Abbey, &summary);
                    let mut stores = Self::lock(&self.stores);
                    let ctx = stores.memory.channel_mut(&scoped_channel);
                    ctx.summary = summary;
                    ctx.summarized_at_count = ctx.message_count;
                    done += 1;
                    tracing::info!(channel = %scoped_channel, "rolling summary refreshed");
                }
                Err(e) => {
                    tracing::warn!(channel = %scoped_channel, error = %e, "rolling summary failed");
                    break;
                }
            }
        }
        done
    }

    /// One owned scheduler with skipped missed ticks and no immediate startup work.
    pub(crate) async fn run_scheduler<T, E, M>(
        self: Arc<Self>,
        cancel: tokio_util::sync::CancellationToken,
        work_transport: Arc<T>,
        engagement_transport: Arc<E>,
        maintenance: Arc<M>,
    ) -> crate::service::TaskExit
    where
        T: super::work_delivery::WorkDeliveryTransport + 'static,
        E: super::engagement_delivery::EngagementTransport + 'static,
        M: CommunityMaintenance + 'static,
    {
        if let Some(status) = self.managed_status() {
            status.scheduler_running();
            let _ = status.refresh();
        }
        if let Some(events) = self.operational_events() {
            let _ = events.record(
                crate::observability::EventComponent::Scheduler,
                crate::observability::EventCode::TaskStarted,
                crate::observability::EventOutcome::Started,
                None,
            );
        }
        use crate::service::scheduler::{Schedule, Tick};
        let mut schedule = Schedule::new();
        loop {
            tokio::select! {
                biased;
                () = cancel.cancelled() => return crate::service::TaskExit::Cancelled,
                tick = schedule.next() => match tick {
                    Tick::Work => {
                        if let Some(registry) = self.service.get() {
                            let state = self.clone();
                            let transport = engagement_transport.clone();
                            let _ = registry.spawn_operation(crate::service::OperationKind::EngagementDelivery, move |cancel| async move {
                                if state.deliver_engagement(transport.as_ref(), cancel, super::now).await.is_err() {
                                    tracing::warn!("engagement settlement persistence failed; attempts require review");
                                }
                                crate::service::TaskExit::Returned
                            });
                        }
                        if let Some(registry) = self.service.get() {
                            let state = self.clone();
                            let transport = work_transport.clone();
                            let _ = registry.spawn_operation(crate::service::OperationKind::WorkDelivery, move |cancel| async move {
                                if state.clone().deliver_work(transport.as_ref(), cancel.clone(), super::now).await.is_err() {
                                    tracing::warn!("work delivery persistence failed; reserved attempts require review");
                                }
                                crate::service::TaskExit::Returned
                            });
                        }
                        if let Some(registry) = self.service.get() {
                            let state = self.clone();
                            let maintenance = maintenance.clone();
                            let _ = registry.spawn_operation(crate::service::OperationKind::CommunityMaintenance, move |cancel| async move {
                                let Ok(_single_flight) = state.community_maintenance_running.try_lock() else {
                                    return crate::service::TaskExit::Returned;
                                };
                                if let Err(reason) = maintenance.maintain(state.clone(), cancel, super::now()).await {
                                    tracing::warn!(reason, "community maintenance stopped");
                                }
                                crate::service::TaskExit::Returned
                            });
                        }
                    }
                    Tick::Learn => self.learn_all(),
                    Tick::Flush => self.flush_social(),
                    Tick::Settle => self.settle_rewards(),
                    Tick::Persist => {
                    if let Some(registry) = self.service.get() {
                        let state = self.clone();
                        let _ = registry.spawn_result(crate::service::OperationKind::PersistencePreparation, async move {
                            if let Ok(report) = state.request_persistence().await { crate::persist::log_report("scheduled", &report); }
                        });
                    }
                }
                    Tick::Summary => {
                    if let Some(registry) = self.service.get() {
                        let state = self.clone();
                        let _ = registry.spawn_operation(crate::service::OperationKind::Summary, move |cancel| async move {
                            tokio::select! {
                                () = cancel.cancelled() => crate::service::TaskExit::Cancelled,
                                _ = state.refresh_summaries() => crate::service::TaskExit::Returned,
                            }
                        });
                    }
                }
                }
            }
        }
    }
}
