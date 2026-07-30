use oxidebot_core::Message;
use oxidebot_runtime::{
    command, ArgumentChoice, ArgumentGroup, ArgumentSpec, CommandBranch, CommandCatalog,
    CommandParseError, CommandSchema,
};

#[test]
fn exactly_one_argument_group_has_precise_failures() {
    let command = command("connect").schema(
        CommandSchema::new()
            .argument(ArgumentSpec::new("token").long("token").required(false))
            .argument(ArgumentSpec::new("cookie").long("cookie").required(false))
            .group(
                ArgumentGroup::new("credential")
                    .arguments(["token", "cookie"])
                    .exactly_one(),
            ),
    );

    assert!(matches!(
        command.parse_message(&Message::text("/connect")),
        Err(CommandParseError::MissingArgumentGroup { ref group, .. }) if group.as_ref() == "credential"
    ));
    assert!(matches!(
        command.parse_message(&Message::text("/connect --token a --cookie b")),
        Err(CommandParseError::ArgumentGroupConflict { ref group, .. }) if group.as_ref() == "credential"
    ));
    assert!(command
        .parse_message(&Message::text("/connect --token a"))
        .is_ok());
}

#[test]
fn global_options_can_appear_before_or_after_a_subcommand() {
    let command = command("todo")
        .global_schema(CommandSchema::new().argument(ArgumentSpec::new("project").long("project")))
        .subcommand(
            CommandBranch::new("add")
                .schema(CommandSchema::new().argument(ArgumentSpec::new("title"))),
        );

    for input in [
        "/todo --project oxidebot add release",
        "/todo add --project oxidebot release",
    ] {
        let matched = command.parse_message(&Message::text(input)).unwrap();
        let arguments = matched.arguments().unwrap();
        assert_eq!(arguments.required::<String>("project").unwrap(), "oxidebot");
        assert_eq!(arguments.required::<String>("title").unwrap(), "release");
    }
}

#[test]
fn choice_aliases_validate_but_native_choices_remain_canonical() {
    let command = command("deploy").schema(
        CommandSchema::new().argument(
            ArgumentSpec::new("environment")
                .choice(ArgumentChoice::new("Production", "production").alias("prod")),
        ),
    );

    assert!(command
        .parse_message(&Message::text("/deploy prod"))
        .is_ok());
    assert!(matches!(
        command.parse_message(&Message::text("/deploy live")),
        Err(CommandParseError::InvalidChoice { .. })
    ));
    let definition = command.definition();
    assert_eq!(definition.options[0].choices[0].value, "production");
}

#[test]
fn quoted_values_option_actions_and_end_of_options_keep_their_meaning() {
    let command = command("publish").schema(
        CommandSchema::new()
            .argument(
                ArgumentSpec::new("verbose")
                    .short('v')
                    .action(oxidebot_runtime::ArgumentAction::Count),
            )
            .argument(ArgumentSpec::new("tag").long("tag").multiple(true))
            .argument(
                ArgumentSpec::new("notes")
                    .rest(true)
                    .multiple(true)
                    .required(false),
            ),
    );
    let matched = command
        .parse_message(&Message::text(
            "/publish -vv --tag=stable -- --not-an-option 'release notes'",
        ))
        .unwrap();
    let arguments = matched.arguments().unwrap();
    assert_eq!(arguments.count("verbose"), 2);
    assert_eq!(arguments.many::<String>("tag").unwrap(), ["stable"]);
    assert_eq!(
        arguments.many::<String>("notes").unwrap(),
        ["--not-an-option", "release notes"]
    );
}

#[test]
fn synchronous_validation_is_part_of_parser_semantics() {
    let command = command("batch").schema(CommandSchema::new().argument(
        ArgumentSpec::new("size").validate::<u8, _, _>(|size| {
            (*size <= 8).then_some(()).ok_or("size must not exceed 8")
        }),
    ));
    assert!(command.parse_message(&Message::text("/batch 8")).is_ok());
    assert!(matches!(
        command.parse_message(&Message::text("/batch 9")),
        Err(CommandParseError::InvalidValue { ref reason, .. }) if reason.contains("size must not exceed 8")
    ));
}

#[test]
fn help_honours_headings_and_hidden_compatibility_fields() {
    let command = command("publish").schema(
        CommandSchema::new()
            .argument(
                ArgumentSpec::new("token")
                    .long("token")
                    .heading("Authentication"),
            )
            .argument(ArgumentSpec::new("format").long("format").heading("Output"))
            .argument(ArgumentSpec::new("legacy").long("legacy").hidden()),
    );
    let help = CommandCatalog::new([command]).render_text(Some("publish"));
    assert!(help.contains("Authentication"));
    assert!(help.contains("Output"));
    assert!(!help.contains("--legacy"));
}
