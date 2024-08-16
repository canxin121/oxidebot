use std::any::Any;

#[derive(Clone, Debug)]
pub struct AnyEvent {
    pub server: &'static str,
    pub r#type: String,
    pub data: AnyEventDataObject,
}

pub type AnyEventDataObject = Box<dyn AnyEventDataTrait>;

pub trait AnyEventDataTrait: Send + Sync {
    fn clone_box(&self) -> Box<dyn AnyEventDataTrait>;
    fn as_any(&self) -> &dyn Any;
}

impl Clone for Box<dyn AnyEventDataTrait> {
    fn clone(&self) -> Self {
        self.clone_box()
    }
}

impl std::fmt::Debug for AnyEventDataObject {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnyEventDataObject").finish()
    }
}

impl AnyEvent {
    pub fn downcast_ref<T: Any>(&self) -> Option<&T> {
        self.data.as_any().downcast_ref::<T>()
    }
}
