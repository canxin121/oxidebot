use std::any::Any;

/// A lossless platform-native event that has no portable OxideBot model.
#[derive(Clone, Debug)]
pub struct NativeEvent {
    /// Adapter platform that emitted the event.
    pub platform: &'static str,
    /// Adapter-defined native event kind.
    pub kind: String,
    /// Opaque payload retained by the adapter.
    pub data: NativeEventData,
}

/// Type-erased payload attached to a [`NativeEvent`].
pub type NativeEventData = Box<dyn NativeEventPayload>;

/// Cloneable, downcastable payload contract for adapter-native events.
pub trait NativeEventPayload: Any + Send + Sync {
    /// Clones this payload behind the trait object.
    fn clone_box(&self) -> Box<dyn NativeEventPayload>;
    /// Returns the payload as [`Any`] for typed downcasting.
    fn as_any(&self) -> &dyn Any;
}

impl Clone for Box<dyn NativeEventPayload> {
    fn clone(&self) -> Self {
        self.clone_box()
    }
}

impl std::fmt::Debug for NativeEventData {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("NativeEventData").finish()
    }
}

impl NativeEvent {
    /// Attempts to view the opaque payload as `T`.
    pub fn downcast_ref<T: Any>(&self) -> Option<&T> {
        self.data.as_any().downcast_ref::<T>()
    }
}
