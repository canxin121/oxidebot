use std::any::Any;

/// A lossless platform-native event that has no portable OxideBot model.
#[derive(Clone, Debug)]
pub struct NativeEvent {
    pub platform: &'static str,
    pub kind: String,
    pub data: NativeEventData,
}

pub type NativeEventData = Box<dyn NativeEventPayload>;

pub trait NativeEventPayload: Any + Send + Sync {
    fn clone_box(&self) -> Box<dyn NativeEventPayload>;
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
    pub fn downcast_ref<T: Any>(&self) -> Option<&T> {
        self.data.as_any().downcast_ref::<T>()
    }
}
