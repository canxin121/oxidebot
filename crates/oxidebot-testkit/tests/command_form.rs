use oxidebot::{Args, BotCommand, CommandArgs, CommandTree, HandlerResult, Module};
use oxidebot_testkit::BotTest;

#[derive(Debug, CommandArgs)]
#[command(interactive)]
struct CreateArgs {
    #[arg(
        prompt = "要创建哪一种任务？",
        choice = "普通任务",
        choice = "紧急任务"
    )]
    kind: String,
    #[arg(prompt = "要创建几个？", min = 1, max = 3)]
    count: u8,
}

async fn create(Args(args): Args<CreateArgs>) -> HandlerResult<String> {
    Ok(format!("已创建 {} 个{}", args.count, args.kind))
}

#[derive(Debug, CommandArgs)]
struct TodoAddArgs {
    #[arg(prompt = "任务内容是什么？")]
    title: String,
}

#[derive(Debug, BotCommand)]
#[command(name = "todo", interactive)]
enum TodoCommand {
    Add(TodoAddArgs),
}

async fn todo(Args(command): Args<TodoCommand>) -> String {
    match command {
        TodoCommand::Add(args) => format!("已添加任务：{}", args.title),
    }
}

#[derive(Debug, CommandArgs)]
#[command(interactive, group(name = "credential", exactly_one))]
struct AuthenticateArgs {
    #[arg(long, group = "credential")]
    token: Option<String>,
    #[arg(long, group = "credential")]
    cookie: Option<String>,
}

async fn authenticate(Args(args): Args<AuthenticateArgs>) -> String {
    match (args.token, args.cookie) {
        (Some(_), None) => "已使用 token 认证".to_owned(),
        (None, Some(_)) => "已使用 cookie 认证".to_owned(),
        _ => "认证参数无效".to_owned(),
    }
}

#[tokio::test]
async fn interactive_command_form_prompts_each_missing_field_and_retries_invalid_answers() {
    BotTest::new(Module::new().add(CreateArgs::feature("create", create)))
        .message("/create")
        .expect_reply_contains("要创建哪一种任务？")
        .message("未知类型")
        .expect_reply_contains("刚才的输入不符合要求")
        .message("普通任务")
        .expect_reply_contains("要创建几个？")
        .message("9")
        .expect_reply_contains("刚才的输入不符合要求")
        .message("2")
        .expect_reply("已创建 2 个普通任务")
        .run()
        .await
        .expect("interactive command form succeeds");
}

#[tokio::test]
async fn interactive_command_form_can_be_cancelled() {
    BotTest::new(Module::new().add(CreateArgs::feature("create", create)))
        .message("/create")
        .expect_reply_contains("发送 `取消` 可退出")
        .message("取消")
        .expect_reply_contains("已取消命令补全")
        .run()
        .await
        .expect("interactive command form cancellation succeeds");
}

#[tokio::test]
async fn interactive_command_form_can_select_a_missing_subcommand_then_fill_its_fields() {
    BotTest::new(Module::new().add(TodoCommand::feature(todo)))
        .message("/todo")
        .expect_reply_contains("请选择一个子命令")
        .message("add")
        .expect_reply_contains("任务内容是什么？")
        .message("整理发布说明")
        .expect_reply("已添加任务：整理发布说明")
        .run()
        .await
        .expect("interactive subcommand form succeeds");
}

#[tokio::test]
async fn interactive_command_form_can_collect_an_exactly_one_group() {
    BotTest::new(Module::new().add(AuthenticateArgs::feature("auth", authenticate)))
        .message("/auth")
        .expect_reply_contains("请发送下列字段之一")
        .message("--token temporary")
        .expect_reply("已使用 token 认证")
        .run()
        .await
        .expect("interactive group collection succeeds");
}

#[tokio::test]
async fn opt_in_typo_assistance_suggests_but_never_executes_a_root_command() {
    BotTest::new(
        Module::new()
            .command(oxidebot::command("deploy"), || async { "executed" })
            .typo_assist(),
    )
    .message("/deply")
    .expect_reply("未知命令；你是不是想输入 `/deploy`？")
    .run()
    .await
    .expect("typo suggestion succeeds");
}
