use crate::{
    adapter::IngressBatch,
    budget::HierarchicalLease,
    dedupe::{DedupeCache, DedupeCommit},
    executor::{ExecutorHandle, ExecutorSubmit},
    session::{SessionDelivery, SessionRegistry},
    BotDirectory, MetricsHandle, Result, RuntimeError,
};
use futures_util::{stream::FuturesUnordered, StreamExt};
use oxidebot_core::event::kernel::DispatchEnvelope;
use oxidebot_core::{BotSlot, EventId};
use std::{
    collections::{HashSet, VecDeque},
    sync::Arc,
    time::Instant,
};
use tokio::sync::mpsc;

struct PendingDispatch {
    slot: BotSlot,
    event: Arc<DispatchEnvelope>,
    bot: crate::BotHandle,
    ingress_retention: Arc<HierarchicalLease>,
}

enum AdmissionOutcome {
    SessionConsumed,
    Executor(ExecutorSubmit),
    SessionClosed,
}

struct AdmissionCompletion {
    slot: BotSlot,
    event_id: EventId,
    outcome: AdmissionOutcome,
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn dispatch_loop(
    mut receiver: mpsc::Receiver<IngressBatch>,
    bots: BotDirectory,
    sessions: SessionRegistry,
    executor: ExecutorHandle,
    max_admissions: usize,
    dedupe_capacity: usize,
    dedupe_max_bytes: usize,
    dedupe_ttl: std::time::Duration,
    metrics: MetricsHandle,
) -> Result<()> {
    let mut sequence = 0_u64;
    let mut dedupe = DedupeCache::new(dedupe_capacity, dedupe_max_bytes, dedupe_ttl, bots.len());
    let mut inflight_ids = HashSet::<(BotSlot, EventId)>::new();
    let mut pending = (0..bots.len())
        .map(|_| VecDeque::<PendingDispatch>::new())
        .collect::<Vec<_>>();
    let mut ready_bots = VecDeque::<BotSlot>::new();
    let mut ready_set = vec![false; bots.len()];
    let mut admission_active = vec![false; bots.len()];
    let mut admissions = FuturesUnordered::new();
    let mut input_closed = false;

    loop {
        let mut admission_attempts = ready_bots.len();
        while admission_attempts > 0 && admissions.len() < max_admissions {
            admission_attempts -= 1;
            let Some(slot) = ready_bots.pop_front() else {
                break;
            };
            let bot_index = slot.0 as usize;
            let Some(ready) = ready_set.get_mut(bot_index) else {
                continue;
            };
            *ready = false;
            if admission_active.get(bot_index).copied().unwrap_or(true) {
                continue;
            }
            let Some(item) = pending.get_mut(bot_index).and_then(VecDeque::pop_front) else {
                continue;
            };
            admission_active[bot_index] = true;
            admissions.push(admit_event(executor.clone(), sessions.clone(), item));
        }

        if input_closed && admissions.is_empty() && pending.iter().all(VecDeque::is_empty) {
            break;
        }

        tokio::select! {
            ingress = receiver.recv(), if !input_closed => {
                let Some(ingress) = ingress else {
                    input_closed = true;
                    continue;
                };
                let received_at = Instant::now();
                let retention = Arc::new(ingress.lease);
                let raw = ingress.batch.raw;
                for draft in ingress.batch.events {
                    let slot = draft.index.bot;
                    let bot = bots.get(slot).cloned().ok_or(RuntimeError::Channel(
                        "event references an unknown bot slot",
                    ))?;
                    let inflight_key = (slot, draft.id.clone());
                    if dedupe.contains(slot, &draft.id, received_at)
                        || !inflight_ids.insert(inflight_key.clone())
                    {
                        metrics.duplicate_event();
                        continue;
                    }

                    let event = Arc::new(draft.finalize(sequence, received_at, raw.clone()));
                    sequence = sequence
                        .checked_add(1)
                        .ok_or(RuntimeError::Channel("event sequence exhausted"))?;
                    let bot_index = slot.0 as usize;
                    let Some(queue) = pending.get_mut(bot_index) else {
                        inflight_ids.remove(&inflight_key);
                        return Err(RuntimeError::Channel(
                            "event references an unknown bot slot",
                        ));
                    };
                    let was_empty = queue.is_empty();
                    queue.push_back(PendingDispatch {
                        slot,
                        event,
                        bot,
                        ingress_retention: Arc::clone(&retention),
                    });
                    if was_empty && !admission_active[bot_index] && !ready_set[bot_index] {
                        ready_set[bot_index] = true;
                        ready_bots.push_back(slot);
                    }
                }
            }
            completion = admissions.next(), if !admissions.is_empty() => {
                if let Some(completion) = completion {
                    let bot_index = completion.slot.0 as usize;
                    if let Some(active) = admission_active.get_mut(bot_index) {
                        *active = false;
                    }
                    inflight_ids.remove(&(completion.slot, completion.event_id.clone()));
                    match completion.outcome {
                        AdmissionOutcome::SessionConsumed => {
                            metrics.session_consumed();
                            commit_dedupe(
                                &mut dedupe,
                                completion.slot,
                                completion.event_id,
                                Instant::now(),
                                &metrics,
                            );
                        }
                        AdmissionOutcome::Executor(
                            ExecutorSubmit::Accepted | ExecutorSubmit::DroppedByPolicy,
                        ) => {
                            // DropNewest is explicitly drop-and-ack: once shed, a
                            // redelivery is still considered a duplicate.
                            commit_dedupe(
                                &mut dedupe,
                                completion.slot,
                                completion.event_id,
                                Instant::now(),
                                &metrics,
                            );
                        }
                        AdmissionOutcome::Executor(ExecutorSubmit::RejectedTooLarge) => {
                            return Err(RuntimeError::EventTooLarge(
                                completion.event_id.to_string(),
                            ));
                        }
                        AdmissionOutcome::Executor(ExecutorSubmit::Closed) => {
                            return Err(RuntimeError::Channel("executor is closed"));
                        }
                        AdmissionOutcome::SessionClosed => {
                            return Err(RuntimeError::Channel("session registry is closed"));
                        }
                    }
                    if pending
                        .get(bot_index)
                        .is_some_and(|queue| !queue.is_empty())
                        && !ready_set[bot_index]
                    {
                        ready_set[bot_index] = true;
                        ready_bots.push_back(completion.slot);
                    }
                }
            }
        }
    }
    Ok(())
}

async fn admit_event(
    executor: ExecutorHandle,
    sessions: SessionRegistry,
    pending: PendingDispatch,
) -> AdmissionCompletion {
    let event_id = pending.event.id.clone();
    let delivery = sessions
        .deliver(
            Arc::clone(&pending.event),
            Arc::clone(&pending.ingress_retention),
        )
        .await;
    let outcome = match delivery {
        Ok(SessionDelivery::Consumed) => AdmissionOutcome::SessionConsumed,
        Ok(SessionDelivery::Tap | SessionDelivery::None) => AdmissionOutcome::Executor(
            executor
                .submit(pending.event, pending.bot, pending.ingress_retention)
                .await,
        ),
        Err(_) => AdmissionOutcome::SessionClosed,
    };
    AdmissionCompletion {
        slot: pending.slot,
        event_id,
        outcome,
    }
}

fn commit_dedupe(
    dedupe: &mut DedupeCache,
    slot: BotSlot,
    id: EventId,
    now: Instant,
    metrics: &MetricsHandle,
) {
    match dedupe.commit(slot, id.clone(), now) {
        DedupeCommit::Inserted => {}
        DedupeCommit::Duplicate => metrics.duplicate_event(),
        DedupeCommit::Uncacheable => {
            metrics.dedupe_uncacheable();
            tracing::warn!(bot_slot = slot.0, event_id = %id, "event id cannot fit in dedupe byte budget");
        }
    }
}
