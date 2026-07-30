//! Cross-scope command-registration validation.

use super::*;

pub(in crate::module) fn validate_command_conflicts<S>(
    handlers: &[HandlerDefinition<S>],
) -> Result<(), String>
where
    S: Send + Sync + 'static,
{
    let commands = handlers.iter().filter_map(|handler| {
        let Selector::Command(command) = &handler.selector else {
            return None;
        };
        Some((command, &handler.scope))
    });

    let mut globals = HashMap::<Arc<str>, Arc<str>>::new();
    let mut platforms = HashMap::<(PlatformId, Arc<str>), Arc<str>>::new();
    let mut bots = HashMap::<(BotIdentity, Arc<str>), Arc<str>>::new();
    let mut seen_commands = std::collections::HashSet::new();
    let commands = commands
        .filter(|(command, scope)| seen_commands.insert((command.id(), (*scope).clone())))
        .collect::<Vec<_>>();
    for (command, _) in &commands {
        command.validate()?;
    }

    for (command, _scope) in commands
        .iter()
        .copied()
        .filter(|(_, scope)| scope.is_global())
    {
        for key in command.registration_keys() {
            if let Some(previous) = globals.insert(key.clone(), Arc::from(command.name())) {
                return Err(format!(
                    "command `{}` conflicts with `{previous}` in the global Bot scope",
                    command.name(),
                ));
            }
        }
    }

    for (command, scope) in commands
        .iter()
        .copied()
        .filter(|(_, scope)| scope.bot.is_none() && scope.platform.is_some())
    {
        let platform = scope
            .platform
            .as_ref()
            .expect("platform-scoped command has a platform");
        for key in command.registration_keys() {
            if let Some(previous) = globals.get(&key) {
                return Err(format!(
                    "command `{}` for platform {platform} conflicts with global command `{previous}`",
                    command.name(),
                ));
            }
            let map_key = (platform.clone(), key);
            if let Some(previous) = platforms.insert(map_key, Arc::from(command.name())) {
                return Err(format!(
                    "command `{}` conflicts with `{previous}` for platform {platform}",
                    command.name(),
                ));
            }
        }
    }

    for (command, scope) in commands
        .iter()
        .copied()
        .filter(|(_, scope)| scope.bot.is_some())
    {
        let bot = scope.bot.as_ref().expect("bot-scoped command has a bot");
        for key in command.registration_keys() {
            if let Some(previous) = globals.get(&key) {
                return Err(format!(
                    "command `{}` for bot {}:{} conflicts with global command `{previous}`",
                    command.name(),
                    bot.platform,
                    bot.bot,
                ));
            }
            if let Some(previous) = platforms.get(&(bot.platform.clone(), key.clone())) {
                return Err(format!(
                    "command `{}` for bot {}:{} conflicts with platform command `{previous}`",
                    command.name(),
                    bot.platform,
                    bot.bot,
                ));
            }
            let map_key = (bot.clone(), key);
            if let Some(previous) = bots.insert(map_key, Arc::from(command.name())) {
                return Err(format!(
                    "command `{}` conflicts with `{previous}` for bot {}:{}",
                    command.name(),
                    bot.platform,
                    bot.bot,
                ));
            }
        }
    }

    Ok(())
}
