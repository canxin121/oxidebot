//! Command parsing errors, localized diagnostics, and completion suggestions.

use super::*;

/// Command syntax and type conversion failures.
#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum CommandParseError {
    #[error("message did not match command `{command}`")]
    /// Input did not match this command.
    NotMatched {
        /// Canonical command name expected by the parser.
        command: Arc<str>,
    },
    #[error("missing required subcommand; expected one of: {choices}")]
    /// A command tree requires a branch name.
    MissingSubcommand {
        /// Human-readable available branch choices.
        choices: Arc<str>,
    },
    #[error("unknown subcommand `{value}`{suggestion}; expected one of: {choices}")]
    /// A supplied branch name is not part of the command grammar.
    UnknownSubcommand {
        /// Unrecognized branch text.
        value: Arc<str>,
        /// Closest matching branch suggestion, if any.
        suggestion: CompletionSuggestion,
        /// Human-readable available branch choices.
        choices: Arc<str>,
    },
    #[error("missing required argument `{name}`")]
    /// A required argument was absent.
    MissingArgument {
        /// Schema name of the missing argument.
        name: Arc<str>,
        /// Localized prompt suitable for interactive recovery.
        prompt: Arc<str>,
    },
    #[error("one of argument group `{group}` is required; expected one of: {members:?}")]
    /// A required group did not receive any member.
    MissingArgumentGroup {
        /// Group that requires one member.
        group: Arc<str>,
        /// Eligible member fields.
        members: Arc<[Arc<str>]>,
    },
    #[error("argument group `{group}` accepts only one member; received: {members:?}")]
    /// More than one mutually exclusive group member was supplied.
    ArgumentGroupConflict {
        /// Mutually exclusive group.
        group: Arc<str>,
        /// Supplied member fields.
        members: Arc<[Arc<str>]>,
    },
    #[error("missing schema field {id:?}")]
    /// A required field ID was absent.
    MissingFieldId {
        /// Stable ID of the missing schema field.
        id: CommandFieldId,
    },
    #[error("unknown option `{option}`{suggestion}")]
    /// A named text option is unknown to the active schema.
    UnknownOption {
        /// Unrecognized option spelling.
        option: Arc<str>,
        /// Closest matching option suggestion, if any.
        suggestion: CompletionSuggestion,
    },
    #[error("unknown native option `{option}`")]
    /// A native platform option does not map to the active schema.
    UnknownNativeOption {
        /// Unrecognized native option name.
        option: Arc<str>,
    },
    #[error("option `{option}` requires a value")]
    /// A value-taking option was supplied without a value.
    MissingOptionValue {
        /// Option spelling that needs a following value.
        option: Arc<str>,
    },
    #[error("option `{option}` was provided more than once")]
    /// A non-repeatable option was supplied more than once.
    DuplicateOption {
        /// Option spelling that was repeated.
        option: Arc<str>,
    },
    #[error("unexpected extra argument `{value}`")]
    /// Input contained a positional value with no accepting field.
    ExtraArgument {
        /// Unexpected input value.
        value: Arc<str>,
    },
    #[error("expected {expected}, received {actual}")]
    /// A typed command value has the wrong kind.
    UnexpectedValue {
        /// Expected portable kind.
        expected: &'static str,
        /// Actual portable kind.
        actual: &'static str,
    },
    #[error("could not parse `{value}` as {expected}: {reason}")]
    /// Text conversion into a requested Rust type failed.
    InvalidValue {
        /// Input value that could not be converted.
        value: String,
        /// Expected Rust or portable type.
        expected: &'static str,
        /// Conversion failure detail.
        reason: String,
    },
    #[error("`{argument}` must be one of: {choices}")]
    /// A value is not in an argument's declared choices.
    InvalidChoice {
        /// Argument whose choice validation failed.
        argument: Arc<str>,
        /// Human-readable allowed choices.
        choices: Arc<str>,
    },
    #[error("`{argument}` is outside the accepted range")]
    /// A value violates configured numeric bounds.
    OutOfRange {
        /// Argument violating numeric bounds.
        argument: Arc<str>,
    },
    #[error("`{argument}` has length {actual}, expected {expected}")]
    /// A value violates configured text-length bounds.
    InvalidLength {
        /// Argument violating length bounds.
        argument: Arc<str>,
        /// Observed text length.
        actual: usize,
        /// Human-readable accepted length range.
        expected: Arc<str>,
    },
    #[error("`{argument}` requires `{required}`")]
    /// A supplied argument requires another argument.
    Requires {
        /// Supplied argument with an unmet requirement.
        argument: Arc<str>,
        /// Required companion argument.
        required: Arc<str>,
    },
    #[error("`{left}` conflicts with `{right}`")]
    /// Two mutually exclusive arguments were supplied together.
    Conflict {
        /// First conflicting argument.
        left: Arc<str>,
        /// Second conflicting argument.
        right: Arc<str>,
    },
    #[error("unterminated quote in command")]
    /// Text tokenization ended while a quote was still open.
    UnterminatedQuote,
    #[error("interactive command completion was cancelled")]
    /// Interactive recovery was cancelled by the caller.
    Cancelled,
    #[error("interactive command completion exceeded its configured rounds")]
    /// Interactive recovery used all configured rounds without valid input.
    CompletionExhausted,
}

impl CommandParseError {
    /// Returns the localized prompt for a missing required argument.
    #[must_use]
    pub fn missing_prompt(&self) -> Option<&str> {
        match self {
            Self::MissingArgument { prompt, .. } => Some(prompt),
            _ => None,
        }
    }

    /// Returns the missing required argument name, when applicable.
    #[must_use]
    pub fn missing_name(&self) -> Option<&str> {
        match self {
            Self::MissingArgument { name, .. } => Some(name),
            _ => None,
        }
    }

    /// Returns a localized human-readable error message.
    #[must_use]
    pub fn localized_message(&self, locale: Option<&str>) -> String {
        let chinese = locale.is_none_or(|locale| locale.starts_with("zh"));
        if !chinese {
            return self.to_string();
        }
        match self {
            Self::NotMatched { command } => format!("消息没有匹配命令 `{command}`"),
            Self::MissingSubcommand { choices } => {
                format!("缺少子命令；可用子命令：{choices}")
            }
            Self::UnknownSubcommand {
                value,
                suggestion,
                choices,
            } => {
                if let Some(suggested) = suggestion.value() {
                    format!(
                        "未知子命令 `{value}`；你是不是想输入 `{suggested}`？可用子命令：{choices}"
                    )
                } else {
                    format!("未知子命令 `{value}`；可用子命令：{choices}")
                }
            }
            Self::MissingArgument { name, .. } => format!("缺少必填参数 `{name}`"),
            Self::MissingArgumentGroup { group, members } => format!(
                "参数组 `{group}` 至少需要提供以下字段之一：{}",
                members
                    .iter()
                    .map(AsRef::as_ref)
                    .collect::<Vec<_>>()
                    .join("、")
            ),
            Self::ArgumentGroupConflict { group, members } => format!(
                "参数组 `{group}` 只能提供一个字段，但同时提供了：{}",
                members
                    .iter()
                    .map(AsRef::as_ref)
                    .collect::<Vec<_>>()
                    .join("、")
            ),
            Self::MissingFieldId { id } => format!("命令字段 {id:?} 不存在"),
            Self::UnknownOption { option, suggestion } => {
                if let Some(value) = suggestion.value() {
                    format!("未知选项 `{option}`；你是不是想输入 `{value}`？")
                } else {
                    format!("未知选项 `{option}`")
                }
            }
            Self::UnknownNativeOption { option } => format!("平台命令包含未知选项 `{option}`"),
            Self::MissingOptionValue { option } => format!("选项 `{option}` 需要一个值"),
            Self::DuplicateOption { option } => format!("选项 `{option}` 不能重复出现"),
            Self::ExtraArgument { value } => format!("存在多余参数 `{value}`"),
            Self::UnexpectedValue { expected, actual } => {
                format!("需要 {expected}，但收到 {actual}")
            }
            Self::InvalidValue {
                value,
                expected,
                reason,
            } => format!("无法把 `{value}` 解析为 {expected}：{reason}"),
            Self::InvalidChoice { argument, choices } => {
                format!("参数 `{argument}` 必须是以下值之一：{choices}")
            }
            Self::OutOfRange { argument } => format!("参数 `{argument}` 超出允许范围"),
            Self::InvalidLength {
                argument,
                actual,
                expected,
            } => format!("参数 `{argument}` 长度为 {actual}，要求 {expected}"),
            Self::Requires { argument, required } => {
                format!("参数 `{argument}` 需要同时提供 `{required}`")
            }
            Self::Conflict { left, right } => {
                format!("参数 `{left}` 与 `{right}` 不能同时使用")
            }
            Self::UnterminatedQuote => "命令中存在未闭合的引号".to_owned(),
            Self::Cancelled => "已取消命令补全".to_owned(),
            Self::CompletionExhausted => "命令补全已达到最大轮数".to_owned(),
        }
    }
}

/// Display helper that avoids storing a formatted suggestion in every error.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CompletionSuggestion(Option<Arc<str>>);

impl CompletionSuggestion {
    pub(in crate::command) fn new(value: Option<Arc<str>>) -> Self {
        Self(value)
    }

    /// Returns the suggested replacement text, if an edit-distance match exists.
    #[must_use]
    pub fn value(&self) -> Option<&str> {
        self.0.as_deref()
    }
}

impl fmt::Display for CompletionSuggestion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(value) = &self.0 {
            write!(formatter, "; did you mean `{value}`?")
        } else {
            Ok(())
        }
    }
}
