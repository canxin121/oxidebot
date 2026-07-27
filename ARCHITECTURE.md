# OxideBot Runtime Architecture

## Hot path

```text
platform frame
  -> lightweight EventIndex
  -> compiled InterestPlan
  -> item/byte ingress admission
  -> full canonical decode once
  -> structural validation
  -> uninterested sibling pruning
  -> dedupe precheck + in-flight duplicate guard
  -> per-bot fair admission
  -> exact session fast path
  -> sharded virtual-actor executor
  -> dedupe commit after consume/admit/intentional shed
  -> compiled route candidates
  -> handler Outcome
  -> bounded per-bot command scheduler
  -> platform service
```

## Runtime invariants

1. A canonical event has one authoritative body and one routing index.
2. Index and body addresses must agree before the event becomes trusted.
3. A bot handle cannot send, edit, or delete data owned by another bot slot.
4. Native request and response data must belong to the bot platform.
5. Queue admission is bounded by retained bytes and item count.
6. Global and per-bot executor and command budgets are acquired in a fixed
   order.
7. No event or handler creates a detached Tokio task.
8. One execution key has at most one running handler pipeline.
9. Exact command, interaction, and native routes do not scan unrelated routes.
10. A normal message performs no session-worker round trip when no exact waiter
    is active.
11. Handler and service panic isolation requires an unwind panic strategy.
12. Shutdown uses one process-wide deadline rather than a fresh grace period for
    each subsystem.
13. `Block` never converts an item that cannot fit an executor envelope into a
    successful submission.
14. Dedupe state is committed only after session consumption, executor admission,
    or an explicit drop-and-ack overload policy.
15. A per-bot semaphore release wakes executor shards that are waiting without
    requiring unrelated input to arrive.
16. Static route interest and candidate tables respect global, platform, and
    exact-bot scopes.

## Resource ownership

- `AdapterContext` owns no background task. It submits admitted frames into the
  runtime ingress channel.
- `IngressBatch` retains both global and per-bot byte/item leases until every derived event has left
  the session/executor paths.
- `ExecutorHandle` holds global and per-bot queue limiters. Each shard owns only
  virtual queues, a bounded set of running futures, and a bounded set of real
  semaphore waiters. A waiter does not consume a local running slot.
- `SessionRegistry` owns exact scope interest and one timer queue per shard.
- `BotHandle` is one clone-cheap `Arc` around immutable identity and a bounded
  command client. Each bot has one scheduler worker, while all workers share
  global queue and service-call limits with reserved high-priority capacity.
- `Application` supervises every long-lived task and controls structured drain.

## Event partitioning

`MessageExecutionPartition::Conversation` is the conservative default. The
execution key includes the optional thread/topic component, so unrelated
threads do not serialize each other.

`ConversationActor` adds the actor to the key for message-created events. This
is useful for group-dialogue workloads, but handlers that mutate shared
conversation state must provide their own synchronization or keep the default
partition.

## Adapter validation boundary

Adapters operate outside the trusted kernel boundary. Before ingress, OxideBot
checks:

- bot and platform ownership;
- semantic ID bounds;
- route-key bounds;
- index/body kind agreement;
- message, actor, and conversation agreement;
- normalized command agreement;
- interaction value limits;
- rich-text UTF-8 ranges;
- media and native-data ownership;
- decoded retained bytes not exceeding the pre-decode estimate.

The same ownership checks are repeated for outgoing commands before they enter
the command scheduler, and adapter-returned receipts/native responses are
validated before reaching application code.

## Overload behavior

`Block` is the reliable mode. Admission waits without allocating unbounded
queues, allowing TCP, WebSocket, long polling, or webhook infrastructure to
propagate pressure upstream. An item that exceeds a configured per-item limit is
rejected; it is never reported as accepted and then silently discarded.

`DropNewest` intentionally sheds the newest executor or command item and updates
metrics. It should be used only when permanent loss is acceptable.

## Low-power behavior

- No global event broadcast.
- No polling loop in the kernel.
- No task per event, handler, or session.
- Exact session interest avoids inactive session wakeups.
- Current-thread Tokio runtimes are supported by default features.
- Timers are centralized through Tokio's timer facilities and shard-local
  `DelayQueue`s.


## Fairness and retry ownership

The dispatcher keeps independent pending queues and at most one admission future
per bot. Ready bots are scheduled round-robin, so a saturated bot cannot block
another bot before the executor.

Executor shards wait on shared per-bot concurrency semaphores through actual
permit futures. Permit waiting is separate from local running capacity, which
prevents both cross-shard lost wake-ups and a saturated bot occupying every local
execution slot.

Command queue admission and active service calls reserve capacity for interaction
responses. A platform service permit covers only one network attempt; it is
released before retry backoff or a bot-wide rate-limit cooldown. Command total
deadlines start before queue admission and therefore include queueing, service
capacity waits, attempts, and backoff.

## Retained-memory charging

`RetainedSize` is a conservative queue charge. `RetainedBytes` prevents a small
`Bytes` slice from hiding a much larger backing allocation: arbitrary views are
compacted by default, while zero-copy adapters must declare the backing charge.
Shared frame raw data is charged once by the ingress lease, and executor leases
charge only each event's incremental state.
