/// Adapter connection-state event.
#[derive(Debug, Clone, PartialEq)]
pub enum MetaEvent {
    /// The adapter established its platform connection.
    Connected,
    /// The adapter lost or closed its platform connection.
    Disconnected,
}
