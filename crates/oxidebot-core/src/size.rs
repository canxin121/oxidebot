/// Conservative estimate of heap and shared bytes retained by a queued value.
///
/// The value is intentionally an upper bound rather than an allocator-exact
/// measurement. Runtime admission uses it to enforce deterministic memory
/// envelopes without inspecting platform-specific allocation internals.
pub trait RetainedSize {
    /// Returns the estimated number of retained bytes.
    fn retained_bytes(&self) -> usize;
}
