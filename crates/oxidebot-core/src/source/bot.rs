use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, PartialEq, Default, Serialize, Deserialize)]
pub struct BotInfo {
    pub id: Option<String>,
    pub nickname: Option<String>,
}
