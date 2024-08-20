# Oxidebot

**Oxidebot** is a lightweight yet powerful chatbot framework based on Rust and the Tokio runtime. It aims to provide developers with a flexible and extensible environment for bot development through modular design.

## Available Bots
- [onebot_v11_oxidebot](https://github.com/canxin121/onebot_v11_oxidebot)
- [telegram_bot_oxidebot](https://github.com/canxin121/telegram_bot_oxidebot)

## Example Usage
- [oxidebot_example](https://github.com/canxin121/oxidebot_example)

## Core Concepts

### Bot
`Bot` is the core component of the framework, responsible for providing `Event`s and offering basic API methods for developers to call. It serves as the bridge between the framework and external platforms (such as QQ, Telegram, etc.).

### Event
`Event` is the object that the framework processes, representing the various events received by the bot. Event types include:

- **MessageEvent**: Message events
- **NoticeEvent**: Notification events
- **RequestEvent**: Request events
- **MetaEvent**: Meta events
- **AnyEvent**: Generalized events

### Matcher
`Matcher` is an abstraction over `Bot` and `Event`, simplifying event handling and API calls. It provides convenient methods to extract key information from events (such as users, messages, groups) and easily call related APIs.

### Handler
`Handler` is the core component for event processing, divided into two types:

- **EventHandler**: Handles incoming `Event`s and is triggered only when an event occurs.
- **ActiveHandler**: Suitable for proactive processing scenarios, it can run continuously, execute scheduled tasks, or perform other background operations.

A `Handler` can include either an `EventHandler` or an `ActiveHandler`, or both.

### Filter
`Filter` is a global event filter used to process and intercept events before they reach the `Handler`. The `Filter` has a higher priority than the `Handler`.

### OxideBotManager
`OxideBotManager` is the manager of the framework, the entry point for starting and running the bot. Developers should call its `run_block` method at the end of the `main` function to launch the entire framework along with all registered `Bot`s, `Filter`s, and `Handler`s.

## Auxiliary Tools

### Interaction
`Interaction` provides an interaction mode similar to `dialoguer`, supporting more intuitive event handling and dialogue flow management. It helps developers implement continuous dialogue logic and enhance the user interaction experience.

## License
MIT OR Apache-2.0
