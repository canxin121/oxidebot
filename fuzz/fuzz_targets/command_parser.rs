#![no_main]

use libfuzzer_sys::fuzz_target;
use oxidebot_core::Message;
use oxidebot_runtime::command;

fuzz_target!(|data: &[u8]| {
    if let Ok(input) = std::str::from_utf8(data) {
        let _ = command("fuzz").parse_message(&Message::text(input));
    }
});
