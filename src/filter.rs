use async_trait::async_trait;

use crate::matcher::Matcher;

/// Filter runs before the Handler, allowing it to process the event and decide whether the event should continue to be handled by the Handler.
/// The Filter runs in order of priority, from low to high and stops when one of the Filters returns false.
/// It's a good idea to allow the user to customize the priority of the Filter.
#[async_trait]
pub trait FilterTrait: Send + Sync {
    /// Filter the event with the Matcher.
    async fn filter(&self, matcher: Matcher) -> bool;
    /// Get the priority of the Filter.
    fn get_priority(&self) -> u8;
}

pub type FilterObject = Box<dyn FilterTrait>;

pub struct FilterPool {
    filters: Vec<FilterObject>,
}

impl FilterPool {
    pub fn new() -> Self {
        FilterPool {
            filters: Vec::new(),
        }
    }

    pub fn build(filters: Vec<FilterObject>) -> Self {
        let mut filters = filters;
        filters.sort_by_key(|filter| filter.get_priority());

        FilterPool { filters }
    }

    pub fn add_filter(&mut self, filter: FilterObject) {
        self.filters.push(filter);
        self.filters.sort_by_key(|filter| filter.get_priority());
    }

    pub async fn filter(&self, matcher: Matcher) -> bool {
        for filter in &self.filters {
            if !filter.filter(matcher.clone()).await {
                return false;
            }
        }
        true
    }
}
