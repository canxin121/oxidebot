use super::MAX_RUNTIME_BOTS;

use crate::{
    adapter::AdapterMode,
    bot::{CommandWorker, GlobalCommandCapacity},
    budget::PriorityQueueLimiter,
    Adapter, BotDescriptor, BotDirectory, BotServices, BuildError, CommandCatalog, CommandRegistry,
    MetricsHandle, Result, RuntimeConfig, RuntimeError, ServiceError,
};
use oxidebot_core::{
    application::{CommandDefinition, CommandOption},
    BotCapabilities, BotIdentity, BotSlot,
};
use std::{
    collections::HashSet,
    panic::{catch_unwind, AssertUnwindSafe},
    sync::Arc,
};

pub(super) struct PreparedAdapter {
    pub(super) identity: BotIdentity,
    pub(super) descriptor: BotDescriptor,
    pub(super) services: BotServices,
    pub(super) mode: AdapterMode,
    pub(super) adapter: Box<dyn Adapter>,
}

pub(super) fn prepare_adapters(
    adapters: Vec<Box<dyn Adapter>>,
) -> std::result::Result<Vec<PreparedAdapter>, BuildError> {
    if adapters.is_empty() {
        return Err(BuildError::NoAdapters);
    }
    if adapters.len() > MAX_RUNTIME_BOTS {
        return Err(BuildError::TooManyBots);
    }
    let mut identities = HashSet::<BotIdentity>::with_capacity(adapters.len());
    let mut prepared = Vec::with_capacity(adapters.len());
    for adapter in adapters {
        let (descriptor, mode, services) = catch_unwind(AssertUnwindSafe(|| {
            (adapter.descriptor(), adapter.mode(), adapter.services())
        }))
        .map_err(|_| BuildError::InvalidBot("adapter metadata panicked".into()))?;
        descriptor
            .validate()
            .map_err(|error| BuildError::InvalidBot(error.to_string()))?;
        if descriptor
            .display_name
            .as_ref()
            .is_some_and(|value| value.len() > 4 * 1024)
        {
            return Err(BuildError::InvalidBot(
                "display name exceeds 4096 bytes".into(),
            ));
        }
        let identity = descriptor.identity();
        if !identities.insert(identity.clone()) {
            return Err(BuildError::DuplicateBot(format!(
                "{}:{}",
                identity.platform, identity.bot
            )));
        }
        prepared.push(PreparedAdapter {
            identity,
            descriptor,
            services,
            mode,
            adapter,
        });
    }
    Ok(prepared)
}

pub(super) struct RegisteredAdapter {
    pub(super) slot: BotSlot,
    pub(super) identity: BotIdentity,
    pub(super) platform: oxidebot_core::PlatformId,
    pub(super) mode: AdapterMode,
    pub(super) adapter: Box<dyn Adapter>,
}

pub(super) fn register_bots(
    adapters: Vec<PreparedAdapter>,
    config: &RuntimeConfig,
    metrics: MetricsHandle,
) -> std::result::Result<(Vec<RegisteredAdapter>, Vec<CommandWorker>, BotDirectory), BuildError> {
    let mut registered = Vec::with_capacity(adapters.len());
    let mut workers = Vec::with_capacity(adapters.len());
    let mut handles = Vec::with_capacity(adapters.len());
    let global_command_limiter =
        PriorityQueueLimiter::new(config.global_command, config.max_command_bytes);
    let global_command_capacity = GlobalCommandCapacity::new(
        config.command_in_flight_global,
        config.command_in_flight_reserved_high_global,
    );

    for (index, prepared) in adapters.into_iter().enumerate() {
        let slot = BotSlot(u32::try_from(index).map_err(|_| BuildError::TooManyBots)?);
        let PreparedAdapter {
            identity,
            descriptor,
            services,
            mode,
            adapter,
        } = prepared;
        let platform = descriptor.platform.clone();
        let (handle, worker) = CommandWorker::build(
            slot,
            descriptor,
            services,
            global_command_limiter.clone(),
            global_command_capacity.clone(),
            config.command,
            config.max_command_bytes,
            config.command_overload,
            config.command_in_flight_per_bot,
            config.command_in_flight_reserved_high_per_bot,
            config.command_high_priority_burst,
            config.command_attempt_timeout,
            config.command_total_timeout,
            config.command_max_retries,
            config.command_retry_base,
            config.command_retry_max,
            Arc::clone(&metrics),
        );
        handles.push(handle);
        workers.push(worker);
        registered.push(RegisteredAdapter {
            slot,
            identity,
            platform,
            mode,
            adapter,
        });
    }

    Ok((registered, workers, BotDirectory::new(handles)))
}

fn prepare_command_definitions(
    mut definitions: Vec<CommandDefinition>,
    capabilities: &BotCapabilities,
) -> std::result::Result<Vec<CommandDefinition>, String> {
    if capabilities
        .limits
        .max_commands
        .is_some_and(|limit| definitions.len() > limit)
    {
        return Err(format!(
            "{} commands exceed the platform limit of {}",
            definitions.len(),
            capabilities.limits.max_commands.unwrap_or_default(),
        ));
    }
    let localized = capabilities
        .application
        .command_localizations
        .is_supported();
    let autocomplete = capabilities.application.autocomplete.is_supported();
    for definition in &mut definitions {
        if !localized {
            definition.name.translations.clear();
            definition.description.translations.clear();
        }
        adapt_native_options(&mut definition.options, localized, autocomplete);
    }
    Ok(definitions)
}

fn adapt_native_options(options: &mut [CommandOption], localized: bool, autocomplete: bool) {
    for option in options {
        if !localized {
            option.name.translations.clear();
            option.description.translations.clear();
            for choice in &mut option.choices {
                choice.name.translations.clear();
            }
        }
        if !autocomplete {
            option.autocomplete = false;
        }
        adapt_native_options(&mut option.options, localized, autocomplete);
    }
}

pub(crate) async fn publish_command_definitions(
    bots: &BotDirectory,
    catalog: &CommandCatalog,
    registry: &CommandRegistry,
) -> Result<()> {
    if catalog.commands().is_empty() {
        return Ok(());
    }
    for bot in bots.iter() {
        let scoped_catalog = catalog.for_identity(bot.identity());
        if scoped_catalog.commands().is_empty() {
            continue;
        }
        let definitions = scoped_catalog.enabled(registry).definitions();
        let capabilities = bot.bot_capabilities().map_err(|error| {
            RuntimeError::Service(ServiceError::new(format!(
                "could not read capabilities for {}:{}: {error}",
                bot.identity().platform,
                bot.identity().bot,
            )))
        })?;
        if !capabilities.application.structured_commands.is_supported() {
            continue;
        }
        let definitions =
            prepare_command_definitions(definitions, &capabilities).map_err(|error| {
                RuntimeError::Service(ServiceError::new(format!(
                    "could not prepare command definitions for {}:{}: {error}",
                    bot.identity().platform,
                    bot.identity().bot,
                )))
            })?;
        bot.set_command_definitions(definitions)
            .await
            .map_err(|error| {
                RuntimeError::Service(ServiceError::new(format!(
                    "could not publish command definitions for {}:{}: {error}",
                    bot.identity().platform,
                    bot.identity().bot,
                )))
            })?;
    }
    Ok(())
}
