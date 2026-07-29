# OxideBot Telegram adapter

`oxidebot-adapter-telegram` is a production transport for Telegram Bot API
long polling. It normalizes incoming text messages into OxideBot events and
delivers planned portable text messages with Telegram's `sendMessage` method.

```no_run
use oxidebot::prelude::*;
use oxidebot_adapter_telegram::TelegramAdapter;

# fn main() -> Result<(), Box<dyn std::error::Error>> {
let adapter = TelegramAdapter::new(
    std::env::var("TELEGRAM_BOT_TOKEN")?,
    std::env::var("TELEGRAM_BOT_ID")?,
)?;
let app = OxideBot::new().adapter(adapter);
# let _ = app;
# Ok(())
# }
```

Use a Telegram test bot before production traffic. The adapter accepts a custom
Bot API base URL through `TelegramConfig::api_base`, which makes recorded or
local Bot API fixture testing possible.
