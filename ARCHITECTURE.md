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
  -> per-bot deduplication
  -> exact session fast path
  -> sharded virtual-actor executor
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

## Resource ownership

- `AdapterContext` owns no background task. It submits admitted frames into the
  runtime ingress channel.
- `IngressBatch` retains its byte/item lease until every derived event has left
  the session/executor paths.
- `ExecutorHandle` holds global and per-bot queue limiters. Each shard owns only
  virtual queues and a bounded set of running futures.
- `SessionRegistry` owns exact scope interest and one timer queue per shard.
- `BotHandle` owns a bounded command client. Each bot has one scheduler worker,
  but all workers also share global queue and in-flight limits.
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
propagate pressure upstream.

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
