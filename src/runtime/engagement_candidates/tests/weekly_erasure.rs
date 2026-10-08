use super::*;
use chrono::{Datelike, Timelike};
use std::{future::Future, task::Poll};

fn seed_weekly(h: &Harness, at: u64) {
    let day = crate::calendar::utc(at).unwrap();
    let mut stores = AppState::lock(&h.state.stores);
    let e = &mut stores.work.engagement;
    e.member_policies.clear();
    e.observations.clear();
    e.eligibility.clear();
    e.responses.clear();
    for (member, guild) in [(2, 7), (3, 7), (4, 9)] {
        let scope = EngagementScope::Guild { guild, channel: 3 };
        let source = SourceRef {
            scope: scope.clone(),
            author: member,
            message: member + 10,
            revision: 1,
            at,
        };
        e.member_policies.insert(
            member,
            crate::engagement::MemberPolicy {
                revision: 1,
                daily_limit: Some(1),
                timezone: Some("UTC".into()),
                quiet_start: 0,
                quiet_end: 0,
                weekly_subscription: Some(crate::engagement::WeeklySubscription {
                    weekday: day.weekday().num_days_from_monday() as u8,
                    hour: day.hour() as u8,
                    scope: scope.clone(),
                    destination: DestinationPreference::Origin,
                }),
                ..Default::default()
            },
        );
        e.observations
            .entry(scope)
            .or_default()
            .insert(member, source.clone());
        e.eligibility
            .entry(member)
            .or_default()
            .insert(source.clone());
        e.responses.insert(source.message, source.message + 100);
    }
}

#[tokio::test]
async fn erasure_weekly_real_planner_queued_behind_erase_cannot_restore_assessment() {
    for reset in [false, true] {
        let h = Harness::new("unclear");
        let at = crate::runtime::now();
        seed_weekly(&h, at);
        let transport = Transport {
            state: h.state.clone(),
            invalidate: false,
        };
        let owner = h.state.persistence_preparation.lock().await;
        let state = h.state.clone();
        let erase = tokio::spawn(async move {
            if reset {
                state.reset_learning_scope("discord:7".into(), at).await
            } else {
                state.erase_personal_learning("discord:7".into(), 2).await
            }
        });
        // On this single-thread executor, run the retained erase task to its
        // preparation wait before polling the production planner's next waiter.
        tokio::task::yield_now().await;
        tokio::task::yield_now().await;
        let mut planner = Box::pin(h.state.clone().plan_weekly(&transport, at));
        std::future::poll_fn(|cx| match planner.as_mut().poll(cx) {
            Poll::Pending => Poll::Ready(()),
            Poll::Ready(result) => panic!("planner did not wait for owner: {result:?}"),
        })
        .await;
        drop(owner);
        erase.await.unwrap().unwrap();
        let result = planner.await;
        let disk = crate::persist::Stores::load(&h.dir).unwrap();
        let e = &disk.work.engagement;
        assert!(e.member_policies[&2].weekly_subscription.is_none());
        assert!(
            !e.weekly_assessments.contains_key(&2),
            "queued planner resurrected erased assessment"
        );
        assert!(!e.candidates.values().any(|c| c.member == Some(2)));
        assert_eq!(result, Err(WorkError::Stale));
        assert_eq!(e.weekly_assessments.contains_key(&3), !reset);
        assert!(e.weekly_assessments.contains_key(&4));
        assert!(
            h.state
                .engagement_events_healthy
                .load(std::sync::atomic::Ordering::SeqCst)
        );
        h.state
            .clone()
            .deliver_engagement(
                &transport,
                tokio_util::sync::CancellationToken::new(),
                || at,
            )
            .await
            .unwrap();
        assert!(
            h.state
                .engagement_events_healthy
                .load(std::sync::atomic::Ordering::SeqCst)
        );
        h.finish().await;
    }
}

#[tokio::test]
async fn erasure_weekly_old_original_plan_stays_denied_after_300_seconds() {
    for reset in [false, true] {
        let h = Harness::new("unclear");
        let original = crate::runtime::now() - 1000;
        seed_weekly(&h, original);
        h.state
            .erase_learning_now("discord:7", (!reset).then_some(2), original + 1)
            .unwrap();
        // Model a genuinely renewed identical subscription/source set. The old
        // owner still cannot relabel its captured admission as this completion.
        seed_weekly(&h, original);
        let transport = Transport {
            state: h.state.clone(),
            invalidate: false,
        };
        assert!(crate::runtime::now() > original + 301);
        assert_eq!(
            h.state.clone().plan_weekly(&transport, original).await,
            Err(WorkError::Stale)
        );
        let disk = crate::persist::Stores::load(&h.dir).unwrap();
        let e = &disk.work.engagement;
        assert!(!e.weekly_assessments.contains_key(&2));
        assert!(!e.candidates.values().any(|c| c.member == Some(2)));
        assert_eq!(e.weekly_assessments.contains_key(&3), !reset);
        assert!(e.weekly_assessments.contains_key(&4));
        assert!(
            h.state
                .engagement_events_healthy
                .load(std::sync::atomic::Ordering::SeqCst)
        );
        h.finish().await;
    }
}

#[tokio::test]
async fn erasure_weekly_captured_subscription_and_native_revision_are_rechecked() {
    for subscription_changed in [false, true] {
        let h = Harness::new("unclear");
        let at = crate::runtime::now();
        seed_weekly(&h, at);
        let transport = Transport {
            state: h.state.clone(),
            invalidate: false,
        };
        let owner = h.state.persistence_preparation.lock().await;
        let state = h.state.clone();
        let change = tokio::spawn(async move {
            state
                .commit_engagement(move |e| {
                    if subscription_changed {
                        e.member_policies
                            .get_mut(&2)
                            .unwrap()
                            .weekly_subscription
                            .as_mut()
                            .unwrap()
                            .destination = DestinationPreference::Private;
                    } else {
                        let scope = EngagementScope::Guild {
                            guild: 7,
                            channel: 3,
                        };
                        e.observations
                            .get_mut(&scope)
                            .unwrap()
                            .get_mut(&2)
                            .unwrap()
                            .revision += 1;
                    }
                    Ok(())
                })
                .await
        });
        tokio::task::yield_now().await;
        tokio::task::yield_now().await;
        let mut planner = Box::pin(h.state.clone().plan_weekly(&transport, at));
        std::future::poll_fn(|cx| match planner.as_mut().poll(cx) {
            Poll::Pending => Poll::Ready(()),
            Poll::Ready(result) => panic!("planner did not wait: {result:?}"),
        })
        .await;
        drop(owner);
        change.await.unwrap().unwrap();
        assert_eq!(planner.await, Err(WorkError::Stale));
        let disk = crate::persist::Stores::load(&h.dir).unwrap();
        assert!(!disk.work.engagement.weekly_assessments.contains_key(&2));
        assert!(disk.work.engagement.weekly_assessments.contains_key(&3));
        assert!(disk.work.engagement.weekly_assessments.contains_key(&4));
        h.finish().await;
    }
}
