use async_trait::async_trait;

use crate::matcher::Matcher;

#[async_trait]
pub trait FilterTrait: Send + Sync {
    async fn filter(&self, matcher: Matcher) -> bool;
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
