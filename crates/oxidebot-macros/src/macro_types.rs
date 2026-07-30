//! Shared Rust-type inspection and token-generation support for derives.
//!
//! Proc-macro entrypoints remain in the crate root, while this module owns
//! pure naming, type-shape, command-value, and argument-action utilities.

use super::*;

pub(crate) fn ident_to_module_name(value: &str) -> String {
    let mut output = String::new();
    for (index, character) in value.chars().enumerate() {
        if character.is_uppercase() {
            if index > 0 {
                output.push('_');
            }
            output.extend(character.to_lowercase());
        } else {
            output.push(character);
        }
    }
    output
}

pub(crate) fn ident_to_command_name(value: &str) -> String {
    let mut output = String::new();
    for (index, character) in value.chars().enumerate() {
        if character.is_uppercase() {
            if index > 0 {
                output.push('-');
            }
            for lower in character.to_lowercase() {
                output.push(lower);
            }
        } else if character == '_' {
            output.push('-');
        } else {
            output.push(character);
        }
    }
    output
}

pub(crate) enum FieldKind<'a> {
    Plain(&'a Type),
    Option(&'a Type),
    Vec(&'a Type),
}

impl<'a> FieldKind<'a> {
    pub(crate) fn of(ty: &'a Type) -> Self {
        if let Some(inner) = type_argument(ty, "Option") {
            Self::Option(inner)
        } else if let Some(inner) = type_argument(ty, "Vec") {
            Self::Vec(inner)
        } else {
            Self::Plain(ty)
        }
    }
}

pub(crate) fn type_argument<'a>(ty: &'a Type, expected: &str) -> Option<&'a Type> {
    let Type::Path(path) = ty else {
        return None;
    };
    let segment = path.path.segments.last()?;
    if segment.ident != expected {
        return None;
    }
    let PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return None;
    };
    arguments.args.iter().find_map(|argument| match argument {
        GenericArgument::Type(ty) => Some(ty),
        _ => None,
    })
}

pub(crate) fn is_type(ty: &Type, expected: &str) -> bool {
    matches!(ty, Type::Path(path) if path.path.segments.last().is_some_and(|segment| segment.ident == expected))
}

pub(crate) fn command_value_kind_tokens(
    ty: &Type,
    explicit: Option<&str>,
) -> syn::Result<proc_macro2::TokenStream> {
    let oxidebot = oxidebot_crate();
    if let Some(explicit) = explicit {
        let normalized = explicit.trim().to_ascii_lowercase().replace('-', "_");
        let tokens = match normalized.as_str() {
            "string" | "text" => quote!(#oxidebot::runtime::CommandValueKind::String),
            "integer" | "int" => quote!(#oxidebot::runtime::CommandValueKind::Integer),
            "number" | "float" => quote!(#oxidebot::runtime::CommandValueKind::Number),
            "boolean" | "bool" => quote!(#oxidebot::runtime::CommandValueKind::Boolean),
            "user" => quote!(#oxidebot::runtime::CommandValueKind::User),
            "conversation" | "channel" => {
                quote!(#oxidebot::runtime::CommandValueKind::Conversation)
            }
            "role" => quote!(#oxidebot::runtime::CommandValueKind::Role),
            "mention" | "mentionable" => quote!(#oxidebot::runtime::CommandValueKind::Mentionable),
            "attachment" | "file" => quote!(#oxidebot::runtime::CommandValueKind::Attachment),
            "message_segment" | "segment" => {
                quote!(#oxidebot::runtime::CommandValueKind::MessageSegment)
            }
            value if value.starts_with("native:") => {
                let value = value.trim_start_matches("native:").to_owned();
                quote!(#oxidebot::runtime::CommandValueKind::PlatformNative(
                    ::std::sync::Arc::from(#value)
                ))
            }
            _ => {
                return Err(syn::Error::new(
                    ty.span(),
                    "unknown command value kind; expected string, integer, number, boolean, user, conversation, role, mentionable, attachment, message_segment, or native:<kind>",
                ));
            }
        };
        return Ok(tokens);
    }

    let tokens = if is_type(ty, "bool") {
        quote!(#oxidebot::runtime::CommandValueKind::Boolean)
    } else if [
        "i8", "i16", "i32", "i64", "i128", "isize", "u8", "u16", "u32", "u64", "u128", "usize",
    ]
    .iter()
    .any(|name| is_type(ty, name))
    {
        quote!(#oxidebot::runtime::CommandValueKind::Integer)
    } else if is_type(ty, "f32") || is_type(ty, "f64") {
        quote!(#oxidebot::runtime::CommandValueKind::Number)
    } else if is_type(ty, "User") {
        quote!(#oxidebot::runtime::CommandValueKind::User)
    } else if is_type(ty, "ConversationRef") || is_type(ty, "MessageTarget") {
        quote!(#oxidebot::runtime::CommandValueKind::Conversation)
    } else if is_type(ty, "File") {
        quote!(#oxidebot::runtime::CommandValueKind::Attachment)
    } else if is_type(ty, "Mention") {
        quote!(#oxidebot::runtime::CommandValueKind::Mentionable)
    } else if is_type(ty, "MessageSegment") {
        quote!(#oxidebot::runtime::CommandValueKind::MessageSegment)
    } else {
        quote!(#oxidebot::runtime::CommandValueKind::String)
    };
    Ok(tokens)
}

pub(crate) fn argument_action_tokens(
    action: Option<&str>,
    span: proc_macro2::Span,
) -> syn::Result<Option<proc_macro2::TokenStream>> {
    let oxidebot = oxidebot_crate();
    let Some(action) = action else {
        return Ok(None);
    };
    let normalized = action.trim().to_ascii_lowercase().replace('-', "_");
    let tokens = match normalized.as_str() {
        "store" => quote!(#oxidebot::runtime::ArgumentAction::Store),
        "append" => quote!(#oxidebot::runtime::ArgumentAction::Append),
        "count" => quote!(#oxidebot::runtime::ArgumentAction::Count),
        "set_true" | "true" => quote!(#oxidebot::runtime::ArgumentAction::SetTrue),
        "set_false" | "false" => quote!(#oxidebot::runtime::ArgumentAction::SetFalse),
        _ => {
            return Err(syn::Error::new(
                span,
                "unknown command action; expected store, append, count, set_true, or set_false",
            ));
        }
    };
    Ok(Some(tokens))
}

pub(crate) fn is_integer_type(ty: &Type) -> bool {
    [
        "i8", "i16", "i32", "i64", "i128", "isize", "u8", "u16", "u32", "u64", "u128", "usize",
    ]
    .iter()
    .any(|name| is_type(ty, name))
}
