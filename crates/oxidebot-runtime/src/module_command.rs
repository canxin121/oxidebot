//! Internal command endpoints, completion, and interactive form flow.
//!
//! This module owns the command-specific endpoint machinery installed by
//! `Module`. It keeps native suggestions, textual completion, command matching,
//! and progressive dialogue completion together so their context/session
//! invariants remain explicit.

use super::*;

pub(super) struct HelpEndpoint<S>
where
    S: Send + Sync + 'static,
{
    pub(super) catalog: Arc<CommandCatalog>,
    pub(super) _state: PhantomData<fn() -> S>,
}

impl<S> Endpoint<S> for HelpEndpoint<S>
where
    S: Send + Sync + 'static,
{
    fn call(&self, context: Context<S>) -> BoxFuture<'static, HandlerResult<Outcome>> {
        let catalog = self
            .catalog
            .for_identity(context.bot_identity())
            .enabled(&context.authoring().registry);
        let query = context
            .command()
            .map(|command| command.text_values().collect::<Vec<_>>().join(" "))
            .filter(|query| !query.is_empty());
        Box::pin(async move {
            let output = catalog.output(query.as_deref());
            let message = context.authoring().render(&context, output).await?;
            Ok(Outcome::new().reply(message))
        })
    }
}

pub(super) struct SuggestionEndpoint<S>
where
    S: Send + Sync + 'static,
{
    pub(super) catalog: Arc<CommandCatalog>,
    pub(super) _state: PhantomData<fn() -> S>,
}

impl<S> Endpoint<S> for SuggestionEndpoint<S>
where
    S: Send + Sync + 'static,
{
    fn call(&self, context: Context<S>) -> BoxFuture<'static, HandlerResult<Outcome>> {
        let catalog = self
            .catalog
            .for_identity(context.bot_identity())
            .enabled(&context.authoring().registry);
        Box::pin(async move {
            let Event::Lifecycle(oxidebot_core::event::LifecycleEvent::SuggestionRequested(
                request,
            )) = context.event()
            else {
                return Ok(Outcome::continue_());
            };
            let request = request.clone();
            let mut suggestions = catalog.native_suggestions(&request);
            let command = request
                .command_id
                .as_deref()
                .and_then(|id| catalog.find_by_id(id))
                .or_else(|| {
                    request
                        .command_name
                        .as_deref()
                        .and_then(|name| catalog.find(name))
                });
            if let (Some(command), Some(field)) = (command, request.field_id.as_deref()) {
                let field = field
                    .trim_start_matches("oxidebot:")
                    .parse::<u32>()
                    .ok()
                    .map(crate::CommandFieldId);
                if let Some(field) = field {
                    let input = CompletionInput {
                        command: command.clone(),
                        field,
                        partial: Arc::from(request.query.as_str()),
                        replace: SourceSpan::default(),
                        locale: request.locale.as_deref().map(Arc::from),
                        limit: request.limit.unwrap_or(25).min(100) as usize,
                    };
                    let dynamic = context.authoring().complete(&context, input).await?;
                    let offset = suggestions.len();
                    suggestions.extend(
                        dynamic.into_iter().enumerate().map(|(index, item)| {
                            item.into_suggestion(offset.saturating_add(index))
                        }),
                    );
                    suggestions.truncate(request.limit.unwrap_or(25).min(100) as usize);
                }
            }
            context
                .bot()?
                .answer_suggestion_request(request, suggestions, None)
                .await
                .map_err(|error| HandlerError::Api(error.to_string()))?;
            Ok(Outcome::stop())
        })
    }
}

pub(super) fn explicit_completion_input(event: &Event) -> Option<(String, usize)> {
    let Event::Message(message) = event else {
        return None;
    };
    let raw = message.message.get_raw_text();
    let trimmed = raw.trim_end();
    let input = trimmed.strip_suffix('?')?;
    if !input.chars().last().is_some_and(char::is_whitespace) {
        return None;
    }
    let input = input.trim_end().to_owned();
    let cursor = input.len();
    Some((input, cursor))
}

pub(super) async fn command_completion_outcome<S>(
    context: &Context<S>,
    input: &str,
    cursor: usize,
) -> HandlerResult<Outcome>
where
    S: Send + Sync + 'static,
{
    let Some(result) = context.command() else {
        return Ok(Outcome::continue_());
    };
    let locale = context.authoring().locale(context).await;
    let mut items = result.command().suggest(input, cursor, locale.as_deref());
    let dynamic_fields = items
        .iter()
        .filter_map(|item| item.field_id.map(|field| (field, item.replace)))
        .collect::<Vec<_>>();
    for (field, replace) in dynamic_fields {
        let partial = input
            .get(replace.start.min(input.len())..replace.end.min(input.len()))
            .unwrap_or_default();
        let mut dynamic = context
            .authoring()
            .complete(
                context,
                CompletionInput {
                    command: result.command().clone(),
                    field,
                    partial: Arc::from(partial),
                    replace,
                    locale: locale.clone(),
                    limit: 25,
                },
            )
            .await?;
        items.append(&mut dynamic);
    }
    let mut seen = std::collections::HashSet::new();
    items.retain(|item| seen.insert((item.value.clone(), item.replace.start, item.replace.end)));
    items.truncate(25);
    let output = if items.is_empty() {
        CommandOutput::Message(oxidebot_core::Message::text(
            if locale
                .as_deref()
                .is_some_and(|value| value.to_ascii_lowercase().starts_with("zh"))
            {
                "当前输入位置没有可用补全"
            } else {
                "No completions are available at the current input position"
            },
        ))
    } else {
        CommandOutput::Suggestions {
            items: items.into(),
        }
    };
    let message = context.authoring().render(context, output).await?;
    Ok(Outcome::stop().reply(message))
}

// The ready context is consumed immediately by the dispatcher, so boxing it
// would add an allocation to every successfully prepared command.
#[allow(clippy::large_enum_variant)]
pub(super) enum CommandPreparation<S>
where
    S: Send + Sync + 'static,
{
    Ready(Context<S>),
    Respond(Outcome),
}

pub(super) async fn prepare_command<S>(
    context: Context<S>,
) -> Result<CommandPreparation<S>, HandlerError>
where
    S: Send + Sync + 'static,
{
    let Some(mut result) = context.command().cloned() else {
        return Ok(CommandPreparation::Ready(context));
    };

    match result.parse_active() {
        Ok(arguments) => {
            return finalize_command(context, result, arguments).await;
        }
        Err(error) if form_target(&result, &error).is_none() => {
            return Ok(CommandPreparation::Respond(
                command_match_error_outcome(&context, &result, error).await?,
            ));
        }
        Err(error)
            if result.completion_ref().is_none()
                || matches!(result.source(), crate::CommandSource::Native(_)) =>
        {
            return Ok(CommandPreparation::Respond(
                command_match_error_outcome(&context, &result, error).await?,
            ));
        }
        Err(_) => {}
    }

    let completion = result
        .completion_ref()
        .cloned()
        .expect("missing arguments reach completion only when it is configured");
    let dialogue = Dialogue::extract(&context)?
        .timeout(completion.timeout)
        .namespace(format!(
            "command:{:016x}:{}",
            result.command().id().0,
            result
                .branch_path()
                .last()
                .map_or(0, |branch| u64::from(branch.0))
        ))?;

    let mut prior_error = None;
    let mut active_target = None;
    let mut attempts_for_target = 0usize;
    for _ in 0..completion.max_rounds {
        let error = match result.parse_active() {
            Ok(arguments) => return finalize_command(context, result, arguments).await,
            Err(error) => error,
        };
        let Some(target) = form_target(&result, &error) else {
            return Ok(CommandPreparation::Respond(
                command_match_error_outcome(&context, &result, error).await?,
            ));
        };
        if active_target.as_ref() != Some(&target) {
            active_target = Some(target.clone());
            attempts_for_target = 0;
            prior_error = None;
        }
        let mut prompt_message = render_form_prompt(&result, &target, prior_error.as_ref());
        if let FormTarget::Field(name) = &target {
            if let Some(argument) = result
                .schema()
                .find(name)
                .filter(|argument| argument.is_autocomplete())
            {
                let locale = context.authoring().locale(&context).await;
                let dynamic = context
                    .authoring()
                    .complete(
                        &context,
                        CompletionInput {
                            command: result.command().clone(),
                            field: argument.id(),
                            partial: Arc::from(""),
                            replace: SourceSpan::default(),
                            locale,
                            limit: 12,
                        },
                    )
                    .await?;
                if !dynamic.is_empty() {
                    let suggestions = context
                        .authoring()
                        .render(
                            &context,
                            CommandOutput::Suggestions {
                                items: dynamic.into(),
                            },
                        )
                        .await?;
                    prompt_message = prompt_message.then("\n");
                    prompt_message.extend(suggestions.segments);
                }
            }
        }
        let response = dialogue.ask_message(prompt_message).await?;
        let Event::Message(message) = response.event() else {
            return Err(HandlerError::internal(
                "command completion received a non-message event",
            ));
        };
        let raw_text = message.message.get_raw_text();
        if completion.is_cancelled(&raw_text) {
            return Ok(CommandPreparation::Respond(
                command_match_error_outcome(&context, &result, CommandParseError::Cancelled)
                    .await?,
            ));
        }
        let values = match tokenize_segments(&message.message.segments) {
            Ok(values) => values,
            Err(error) => {
                return Ok(CommandPreparation::Respond(
                    command_match_error_outcome(&context, &result, error).await?,
                ));
            }
        };
        let Some(answered) = target.apply(&result, values) else {
            return Ok(CommandPreparation::Respond(
                command_match_error_outcome(&context, &result, error).await?,
            ));
        };
        match answered.parse_active() {
            Ok(_) => {
                result = answered;
                active_target = None;
                prior_error = None;
            }
            Err(next_error) => {
                let next_target = form_target(&answered, &next_error);
                if next_target.as_ref().is_some_and(|next| next != &target) {
                    result = answered;
                    active_target = None;
                    prior_error = None;
                    continue;
                }
                if completion.retry_invalid && form_retryable_error(&next_error) {
                    attempts_for_target += 1;
                    if attempts_for_target < completion.max_attempts_per_field {
                        prior_error = Some(next_error);
                        continue;
                    }
                }
                return Ok(CommandPreparation::Respond(
                    command_match_error_outcome(&context, &answered, next_error).await?,
                ));
            }
        }
    }

    match result.parse_active() {
        Ok(arguments) => finalize_command(context, result, arguments).await,
        Err(error) if error.missing_prompt().is_some() => Ok(CommandPreparation::Respond(
            command_match_error_outcome(&context, &result, CommandParseError::CompletionExhausted)
                .await?,
        )),
        Err(error) => Ok(CommandPreparation::Respond(
            command_match_error_outcome(&context, &result, error).await?,
        )),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum FormTarget {
    Field(Arc<str>),
    Group(Arc<str>),
    Subcommand,
}

impl FormTarget {
    fn apply(
        &self,
        result: &CommandMatch,
        values: Vec<crate::CommandValue>,
    ) -> Option<CommandMatch> {
        match self {
            Self::Field(name) => Some(result.with_answer(name, values)),
            Self::Group(_) => Some(result.with_appended(values)),
            Self::Subcommand => result.with_subcommand_answer(values),
        }
    }
}

fn form_target(result: &CommandMatch, error: &CommandParseError) -> Option<FormTarget> {
    match error {
        CommandParseError::MissingArgument { name, .. } => {
            Some(FormTarget::Field(Arc::clone(name)))
        }
        CommandParseError::Requires { required, .. } => result
            .schema()
            .find(required)
            .map(|_| FormTarget::Field(Arc::clone(required))),
        CommandParseError::MissingArgumentGroup { group, .. } => {
            Some(FormTarget::Group(Arc::clone(group)))
        }
        CommandParseError::MissingSubcommand { .. } => Some(FormTarget::Subcommand),
        _ => None,
    }
}

fn form_retryable_error(error: &CommandParseError) -> bool {
    matches!(
        error,
        CommandParseError::MissingArgument { .. }
            | CommandParseError::MissingArgumentGroup { .. }
            | CommandParseError::InvalidValue { .. }
            | CommandParseError::InvalidChoice { .. }
            | CommandParseError::OutOfRange { .. }
            | CommandParseError::InvalidLength { .. }
            | CommandParseError::UnexpectedValue { .. }
            | CommandParseError::MissingOptionValue { .. }
            | CommandParseError::DuplicateOption { .. }
            | CommandParseError::ExtraArgument { .. }
            | CommandParseError::UnknownOption { .. }
            | CommandParseError::Requires { .. }
            | CommandParseError::Conflict { .. }
    )
}

fn render_form_prompt(
    result: &CommandMatch,
    target: &FormTarget,
    prior_error: Option<&CommandParseError>,
) -> oxidebot_core::Message {
    let locale = result.locale();
    let mut text = String::new();
    if let Some(error) = prior_error {
        text.push_str(if locale.is_some_and(|locale| !locale.starts_with("zh")) {
            "That value is not valid: "
        } else {
            "刚才的输入不符合要求："
        });
        text.push_str(&error.localized_message(locale));
        text.push_str("\n\n");
    }
    if locale.is_some_and(|locale| !locale.starts_with("zh")) {
        text.push_str("Completing ");
        text.push_str(&result.command().display_name());
        text.push('\n');
    } else {
        text.push_str("正在完善 ");
        text.push_str(&result.command().display_name());
        text.push('\n');
    }
    match target {
        FormTarget::Field(name) => {
            let argument = result
                .schema()
                .find(name)
                .expect("missing command field must exist in the active schema");
            text.push_str(&argument.prompt_text_for(locale));
            if !argument.choices().is_empty() {
                text.push_str(if locale.is_some_and(|locale| !locale.starts_with("zh")) {
                    "\nChoices: "
                } else {
                    "\n可选："
                });
                text.push_str(
                    &argument
                        .choices()
                        .iter()
                        .map(|choice| choice.name.resolve(locale))
                        .collect::<Vec<_>>()
                        .join("、"),
                );
            }
        }
        FormTarget::Group(group) => {
            let members = result
                .schema()
                .groups()
                .iter()
                .find(|candidate| candidate.name() == group.as_ref())
                .map(|candidate| {
                    candidate
                        .arguments_ref()
                        .iter()
                        .map(AsRef::as_ref)
                        .collect::<Vec<_>>()
                        .join("、")
                })
                .unwrap_or_default();
            text.push_str(if locale.is_some_and(|locale| !locale.starts_with("zh")) {
                "Send one of these option values: "
            } else {
                "请发送下列字段之一的完整选项和值："
            });
            text.push_str(&members);
        }
        FormTarget::Subcommand => {
            text.push_str(if locale.is_some_and(|locale| !locale.starts_with("zh")) {
                "Choose a subcommand."
            } else {
                "请选择一个子命令。"
            });
        }
    }
    text.push_str(if locale.is_some_and(|locale| !locale.starts_with("zh")) {
        "\nSend `cancel` to stop."
    } else {
        "\n直接发送这一项即可；发送 `取消` 可退出。"
    });
    oxidebot_core::Message::text(text)
}

pub(super) enum CommandEventError {
    Parse(CommandParseError),
    Handler(HandlerError),
}

impl From<CommandParseError> for CommandEventError {
    fn from(value: CommandParseError) -> Self {
        Self::Parse(value)
    }
}

impl From<HandlerError> for CommandEventError {
    fn from(value: HandlerError) -> Self {
        Self::Handler(value)
    }
}

pub(super) async fn match_command_event<S>(
    command: &Command,
    event: &Event,
    context: &Context<S>,
    command_input: Option<&tokio::sync::OnceCell<RewriteInput>>,
) -> Result<Option<CommandMatch>, CommandEventError>
where
    S: Send + Sync + 'static,
{
    match event {
        Event::Message(message_event) => {
            let shortcuts = context
                .authoring()
                .registry
                .runtime_shortcuts_for(command.id());
            if let Some(command_input) = command_input {
                let input = command_input
                    .get_or_try_init(|| async {
                        let mut message = explicit_completion_input(event)
                            .map(|(input, _)| oxidebot_core::Message::text(input))
                            .unwrap_or_else(|| message_event.message.clone());
                        for normalizer in context.authoring().normalizers.iter() {
                            message = normalizer.normalize(context, message).await?;
                        }
                        let mut input = RewriteInput {
                            message,
                            locale: event_locale(event).map(Arc::from),
                        };
                        for rewriter in context.authoring().rewriters.iter() {
                            input = rewriter.rewrite(context, input).await?;
                        }
                        Ok::<RewriteInput, HandlerError>(input)
                    })
                    .await?;
                return command
                    .match_message_with_shortcuts(&input.message, &shortcuts)
                    .map(|matched| matched.map(|matched| matched.with_locale(input.locale.clone())))
                    .map_err(Into::into);
            }
            let input = RewriteInput {
                message: explicit_completion_input(event)
                    .map(|(input, _)| oxidebot_core::Message::text(input))
                    .unwrap_or_else(|| message_event.message.clone()),
                locale: event_locale(event).map(Arc::from),
            };
            command
                .match_message_with_shortcuts(&input.message, &shortcuts)
                .map(|matched| matched.map(|matched| matched.with_locale(input.locale)))
                .map_err(Into::into)
        }
        Event::Interaction(interaction)
            if interaction.kind == InteractionKind::Command && interaction.command.is_some() =>
        {
            command
                .match_invocation(
                    interaction
                        .command
                        .as_ref()
                        .expect("guarded interaction command exists"),
                )
                .map_err(Into::into)
        }
        _ => Ok(None),
    }
}

fn event_locale(event: &Event) -> Option<&str> {
    match event {
        Event::Interaction(interaction) => interaction
            .command
            .as_ref()
            .and_then(|command| command.locale.as_deref())
            .or(interaction.locale.as_deref()),
        _ => None,
    }
}

async fn finalize_command<S>(
    mut context: Context<S>,
    result: CommandMatch,
    arguments: crate::ParsedArguments,
) -> Result<CommandPreparation<S>, HandlerError>
where
    S: Send + Sync + 'static,
{
    let parsed = result.with_parsed(arguments);
    let parsed = context
        .authoring()
        .apply_command_middleware(&context, parsed)
        .await?;
    context.command = Some(parsed);
    Ok(CommandPreparation::Ready(context))
}

async fn command_match_error_outcome<S>(
    context: &Context<S>,
    result: &CommandMatch,
    error: CommandParseError,
) -> Result<Outcome, HandlerError>
where
    S: Send + Sync + 'static,
{
    command_parse_outcome(context, result.command(), result.branch_names(), error).await
}

pub(super) async fn command_parse_outcome<S>(
    context: &Context<S>,
    command: &Command,
    branch_names: &[Arc<str>],
    error: CommandParseError,
) -> Result<Outcome, HandlerError>
where
    S: Send + Sync + 'static,
{
    let output = CommandOutput::ParseError {
        command: command.clone(),
        branch_names: branch_names.to_vec().into(),
        error,
    };
    let message = context.authoring().render(context, output).await?;
    Ok(Outcome::stop().reply(message))
}
