use oxidebot::{commands::prelude::{CompletionInput, CompletionItem, CompletionKind}, message::prelude::TemplateValue, prelude::*};
use oxidebot_adapter_console::ConsoleAdapter;

#[derive(Clone, Default)]
struct GreetingService;

impl GreetingService {
    fn status(&self) -> &'static str {
        "ready"
    }
}

#[derive(BotState)]
struct AppState {
    #[state]
    greetings: GreetingService,
}

#[oxidebot::command("ping")]
/// Checks whether the bot is alive.
async fn ping(State(service): State<GreetingService>) -> String {
    format!("pong ({})", service.status())
}

#[oxidebot::completer]
async fn complete_words(
    _context: Context<AppState>,
    input: CompletionInput,
) -> HandlerResult<Vec<CompletionItem>> {
    let values = ["hello", "oxidebot", "world"];
    Ok(values
        .into_iter()
        .filter(|value| value.starts_with(input.partial.as_ref()))
        .take(input.limit)
        .map(|value| {
            CompletionItem::new(value, CompletionKind::Choice, input.replace)
                .description(format!("echo {value}"))
        })
        .collect())
}

#[oxidebot::command("echo")]
async fn echo(
    #[arg(
        rest,
        required = true,
        prompt = "What should I echo?",
        complete = complete_words
    )]
    content: Vec<String>,
) -> String {
    content.join(" ")
}

#[oxidebot::command("hello")]
async fn hello(Sender(user): Sender, i18n: I18n) -> HandlerResult<Message> {
    i18n
        .message("hello")
        .arg("user", TemplateValue::mention(user.id))
        .await
}


fn non_empty(value: &String) -> Result<(), &'static str> {
    (!value.trim().is_empty())
        .then_some(())
        .ok_or("Project name cannot be empty.")
}

#[derive(DialogueForm)]
struct SetupForm {
    /// What is the project name?
    #[dialogue(
        attempts = 3,
        error = "Please enter a non-empty project name.",
        validate = non_empty
    )]
    name: String,

    #[dialogue(
        prompt = "Choose an environment:",
        choice = "Development=dev",
        choice = "Staging=staging",
        choice = "Production=production"
    )]
    environment: String,

    #[dialogue(prompt = "Create this project?", confirm)]
    confirmed: bool,
}

#[oxidebot::command("setup")]
async fn setup(dialogue: Dialogue, i18n: I18n) -> HandlerResult<Message> {
    let form = dialogue.named("console-project-setup").form::<SetupForm>().await?;
    i18n
        .message("setup.complete")
        .arg("name", form.name)
        .arg("environment", form.environment)
        .arg("confirmed", form.confirmed.to_string())
        .await
}

#[tokio::main]
async fn main() -> oxidebot::Result<()> {
    let locale_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("locales");

    OxideBot::with_state(AppState {
        greetings: GreetingService,
    })
    .localization_from_dir(locale_dir, "en-US", 256)
    .expect("console example translations are valid")
    .adapter(ConsoleAdapter::development())
    .add(ping)
    .add(echo)
    .add(hello)
    .add(setup)
    .include(Module::new().help())
    .run()
    .await
}
