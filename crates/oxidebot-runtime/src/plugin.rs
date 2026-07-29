use crate::{Module, Service};
use oxidebot_core::BotCapabilities;
use std::sync::Arc;

/// Stable descriptive metadata for a reusable application feature.
///
/// Metadata deliberately has no runtime behavior. It gives applications and
/// tooling a single place to describe a plugin without introducing a second
/// dependency-injection or lifecycle system.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PluginMetadata {
    name: Arc<str>,
    version: Option<Arc<str>>,
    description: Option<Arc<str>>,
}

impl PluginMetadata {
    /// Creates metadata with the human-readable plugin name.
    #[must_use]
    pub fn new(name: impl Into<Arc<str>>) -> Self {
        Self {
            name: name.into(),
            version: None,
            description: None,
        }
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub fn version(&self) -> Option<&str> {
        self.version.as_deref()
    }

    #[must_use]
    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }
}

/// A reusable application plugin with handlers and supervised services.
///
/// A `PluginBundle` is intentionally a thin composition layer over the
/// existing flat [`Module`] and [`Service`] APIs. It provides a natural home
/// for a feature's configuration and background tasks without adding hidden
/// routing, mutable global registries, or a second application runtime.
///
/// ```no_run
/// # use oxidebot_runtime::{Module, PluginBundle};
/// let moderation = PluginBundle::new("moderation")
///     .version("1.0")
///     .description("Moderation commands and policy checks")
///     .include(Module::new());
/// # let _ = moderation;
/// ```
pub struct PluginBundle<S = ()>
where
    S: Send + Sync + 'static,
{
    metadata: PluginMetadata,
    module: Module<S>,
    services: Vec<Arc<dyn Service<S>>>,
    requirements: Vec<PluginRequirement>,
}

pub(crate) struct PluginParts<S>
where
    S: Send + Sync + 'static,
{
    pub(crate) metadata: PluginMetadata,
    pub(crate) module: Module<S>,
    pub(crate) services: Vec<Arc<dyn Service<S>>>,
    pub(crate) requirements: Vec<PluginRequirement>,
}

/// A portable capability prerequisite declared by a plugin.
#[derive(Clone)]
pub struct PluginRequirement {
    pub(crate) description: Arc<str>,
    pub(crate) predicate: Arc<dyn Fn(&BotCapabilities) -> bool + Send + Sync>,
}

impl std::fmt::Debug for PluginRequirement {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PluginRequirement")
            .field("description", &self.description)
            .finish_non_exhaustive()
    }
}

impl<S> PluginBundle<S>
where
    S: Send + Sync + 'static,
{
    /// Starts an empty named plugin.
    #[must_use]
    pub fn new(name: impl Into<Arc<str>>) -> Self {
        Self {
            metadata: PluginMetadata::new(name),
            module: Module::new(),
            services: Vec::new(),
            requirements: Vec::new(),
        }
    }

    /// Adds optional version metadata used by diagnostics and packaging tools.
    #[must_use]
    pub fn version(mut self, version: impl Into<Arc<str>>) -> Self {
        self.metadata.version = Some(version.into());
        self
    }

    /// Adds a short plugin description.
    #[must_use]
    pub fn description(mut self, description: impl Into<Arc<str>>) -> Self {
        self.metadata.description = Some(description.into());
        self
    }

    /// Includes handlers and feature-local hooks in registration order.
    #[must_use]
    pub fn include(mut self, module: Module<S>) -> Self {
        self.module = self.module.include(module);
        self
    }

    /// Adds one generated command, feature, or module to this plugin.
    #[must_use]
    #[allow(
        clippy::should_implement_trait,
        reason = "`add` installs a feature into this fluent plugin builder; it is not arithmetic"
    )]
    pub fn add<F>(mut self, feature: F) -> Self
    where
        F: crate::IntoFeature<S>,
    {
        self.module = self.module.add(feature);
        self
    }

    /// Adds a supervised background service owned by this plugin.
    #[must_use]
    pub fn service<T>(mut self, service: T) -> Self
    where
        T: Service<S>,
    {
        self.services.push(Arc::new(service));
        self
    }

    /// Requires every registered adapter to satisfy a portable capability
    /// predicate before the application can build.
    ///
    /// The predicate receives the one canonical `BotCapabilities` model, so a
    /// plugin never probes adapter-specific globals at runtime.
    #[must_use]
    pub fn require_capability<F>(mut self, description: impl Into<Arc<str>>, predicate: F) -> Self
    where
        F: Fn(&BotCapabilities) -> bool + Send + Sync + 'static,
    {
        self.requirements.push(PluginRequirement {
            description: description.into(),
            predicate: Arc::new(predicate),
        });
        self
    }

    /// Returns the plugin's immutable metadata.
    #[must_use]
    pub fn metadata(&self) -> &PluginMetadata {
        &self.metadata
    }

    pub(crate) fn into_parts(self) -> PluginParts<S> {
        PluginParts {
            metadata: self.metadata,
            module: self.module,
            services: self.services,
            requirements: self.requirements,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundle_keeps_metadata_with_its_flat_module() {
        let bundle = PluginBundle::<()>::new("reminders")
            .version("1.2.3")
            .description("sends scheduled reminders")
            .include(Module::new());

        assert_eq!(bundle.metadata().name(), "reminders");
        assert_eq!(bundle.metadata().version(), Some("1.2.3"));
        assert_eq!(
            bundle.metadata().description(),
            Some("sends scheduled reminders")
        );
    }
}
