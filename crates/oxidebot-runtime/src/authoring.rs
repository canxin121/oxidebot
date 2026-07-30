use crate::{
    BotDirectory, BotHandle, Command, CommandFieldId, CommandId, CommandMatch, CommandOutput,
    CommandParseError, CommandRegistry, CommandRenderer, CompletionItem, Context, Extract,
    ExtractError, FromCommandMatch, FromCommandValue, Guard, GuardDecision, HandlerError,
    HandlerResult,
};
use async_trait::async_trait;
use futures_util::FutureExt;
use oxidebot_core::{
    conversation::MessageTarget,
    source::message::{DeliveryPlan, DeliveryReport, FallbackPolicy, Message},
    TranslationCatalog,
};
use std::{
    collections::HashMap,
    future::Future,
    ops::Deref,
    panic::AssertUnwindSafe,
    sync::{Arc, RwLock},
};

#[path = "authoring_hooks.rs"]
mod hooks;

#[path = "authoring_runtime.rs"]
mod runtime;

#[path = "authoring_branch.rs"]
mod branch;

pub use branch::{when_branch, when_field_equals, BranchArgs, CommandBranchTag, UnitBranch};
pub use hooks::{
    CommandMiddleware, CommandOutputMiddleware, CommandRewriter, CompletionInput,
    DeliveryMiddleware, DynamicCompleter, EventLocaleResolver, LocaleResolver, MessageNormalizer,
    RewriteInput,
};
pub(crate) use runtime::{AuthoringRuntime, BoundDeliveryPipeline, ErasedDeliveryPipeline};

#[cfg(test)]
mod resource_limit_tests {
    use super::*;
    use crate::{
        bot::{CommandWorker, GlobalCommandCapacity},
        budget::PriorityQueueLimiter,
        LocalMediaResolver, RuntimeConfig, RuntimeMetrics, Shortcut,
    };
    use oxidebot_core::{
        application::CommandDefinition, BotCapabilities, BotId, CallApiTrait, CallError,
        CallResult, PlatformId, SupportLevel,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Default)]
    struct PublicationApi {
        calls: AtomicUsize,
        failures: AtomicUsize,
    }

    #[async_trait]
    impl CallApiTrait for PublicationApi {
        fn bot_capabilities(&self) -> BotCapabilities {
            let mut capabilities = BotCapabilities::default();
            capabilities.application.structured_commands = SupportLevel::Native;
            capabilities
        }

        async fn set_command_definitions(
            &self,
            _commands: Vec<CommandDefinition>,
        ) -> CallResult<()> {
            self.calls.fetch_add(1, Ordering::AcqRel);
            let mut failures = self.failures.load(Ordering::Acquire);
            let should_fail = loop {
                let Some(next) = failures.checked_sub(1) else {
                    break false;
                };
                match self.failures.compare_exchange_weak(
                    failures,
                    next,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                ) {
                    Ok(_) => break true,
                    Err(observed) => failures = observed,
                }
            };
            if should_fail {
                return Err(CallError::temporary("scripted publication failure"));
            }
            Ok(())
        }
    }

    #[test]
    fn command_registry_enforces_shortcut_and_byte_limits() {
        let registry = CommandRegistry::bounded_bytes(2, 512);
        let command = crate::command("bounded");
        registry.register(&command).expect("base command fits");

        let error = registry
            .add_shortcut(
                command.id(),
                Shortcut::literal("x".repeat(200), "y".repeat(200)),
            )
            .expect_err("shortcut exceeds registry byte budget");
        assert!(error.to_string().contains("byte capacity"));

        let oversized = crate::command("oversized").shortcut(Shortcut::literal(
            "x".repeat(crate::MAX_SHORTCUT_PATTERN_BYTES + 1),
            "/oversized",
        ));
        let error = CommandRegistry::bounded(2)
            .register(&oversized)
            .expect_err("oversized pattern is rejected");
        assert!(error.to_string().contains("pattern byte limit"));
    }

    #[tokio::test]
    async fn local_media_resolver_reads_only_limit_plus_one_bytes() {
        static NEXT_MEDIA_PATH: AtomicUsize = AtomicUsize::new(0);
        let sequence = NEXT_MEDIA_PATH.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "oxidebot-media-limit-{}-{}",
            std::process::id(),
            sequence
        ));
        std::fs::write(&path, b"123456789").expect("write temporary media");
        let result = LocalMediaResolver::new(8).read_path(path.clone()).await;
        let _ = std::fs::remove_file(path);
        let error = result.expect_err("media larger than the hard limit is rejected");
        assert!(error.to_string().contains("byte limit"));
    }

    #[tokio::test]
    async fn failed_publication_remains_pending_and_a_noop_toggle_retries_it() {
        let api = Arc::new(PublicationApi::default());
        let config = RuntimeConfig::default();
        let metrics = Arc::new(RuntimeMetrics::default());
        let descriptor = crate::BotDescriptor::new(
            PlatformId::new("publication-test").expect("static platform id"),
            BotId::new("bot").expect("static bot id"),
        );
        let services = crate::BotServices::new(api.clone());
        let global_limiter =
            PriorityQueueLimiter::new(config.global_command, config.max_command_bytes);
        let global_capacity = GlobalCommandCapacity::new(
            config.command_in_flight_global,
            config.command_in_flight_reserved_high_global,
        );
        let (handle, worker) = CommandWorker::build(
            oxidebot_core::BotSlot(0),
            descriptor,
            services,
            global_limiter,
            global_capacity,
            config.command,
            config.max_command_bytes,
            config.command_overload,
            config.command_in_flight_per_bot,
            config.command_in_flight_reserved_high_per_bot,
            config.command_high_priority_burst,
            config.command_attempt_timeout,
            config.command_total_timeout,
            0,
            config.command_retry_base,
            config.command_retry_max,
            metrics,
        );
        let worker_task = tokio::spawn(worker.run());
        let command = crate::command("publish-test");
        let registry = CommandRegistry::bounded(4);
        registry.register(&command).expect("register command");
        let directory = BotDirectory::new(vec![handle]);
        registry.attach_publication(
            directory.clone(),
            crate::CommandCatalog::new([command.clone()]),
        );
        registry
            .refresh_publication()
            .await
            .expect("initial publication succeeds");

        api.failures.store(1, Ordering::Release);
        registry
            .disable_and_publish(command.id())
            .await
            .expect_err("first dynamic publication fails");
        assert_eq!(
            registry.publication_status().pending_revision,
            Some(registry.revision())
        );

        let changed = registry
            .disable_and_publish(command.id())
            .await
            .expect("unchanged state retries pending publication");
        assert!(!changed);
        assert_eq!(registry.publication_status().pending_revision, None);
        assert_eq!(api.calls.load(Ordering::Acquire), 3);

        registry.detach_publication();
        drop(directory);
        worker_task.await.expect("command worker exits cleanly");
    }
}
