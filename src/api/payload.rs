pub enum GroupMuteType {
    Mute,
    Unmute,
}

pub enum SendMessageTarget {
    Group(String),
    Private(String),
}

pub enum GroupAdminChangeType {
    Set,
    Unset,
}

pub enum RequestResponse {
    Approve,
    Reject,
}
