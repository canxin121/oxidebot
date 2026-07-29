# OxideBot Telegram adapter

`oxidebot-adapter-telegram` is a production transport for the implemented
Telegram Bot API text-message subset. It uses long polling, normalizes incoming
text messages into OxideBot events, and delivers planned portable text messages
with Telegram's `sendMessage` method.

Its declared `BotCapabilities` intentionally match that scope: direct and
group conversations plus plain-text delivery are native, and rich text is
emulated. Message edits, deletes, reactions, callback interactions, media, and
webhooks are not implemented yet and are therefore never advertised as
available.

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
