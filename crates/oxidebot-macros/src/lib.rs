//! Procedural macros for OxideBot commands, command values, state projection,
//! and dialogue forms.

use proc_macro::TokenStream;
use proc_macro_crate::{crate_name, FoundCrate};
use quote::{format_ident, quote, ToTokens};
use syn::{
    parse::{Parse, ParseStream},
    parse_macro_input,
    spanned::Spanned,
    Attribute, Data, DeriveInput, Expr, Field, Fields, FnArg, GenericArgument, Ident, ItemFn,
    LitChar, LitInt, LitStr, Meta, Pat, Path, PathArguments, Token, Type,
};

mod macro_args;
mod macro_branch;
mod macro_command;
mod macro_completer;
mod macro_dialogue;
mod macro_enum;
mod macro_tree;
mod macro_types;

use macro_branch::{expand_branch_function, BranchAttribute};
use macro_completer::expand_completer_function;
use macro_enum::expand_command_enum;
use macro_types::*;

fn oxidebot_crate() -> proc_macro2::TokenStream {
    match crate_name("oxidebot") {
        Ok(FoundCrate::Itself) => quote!(crate),
        Ok(FoundCrate::Name(name)) => {
            let ident = format_ident!("{}", name.replace('-', "_"));
            quote!(::#ident)
        }
        Err(_) => quote!(::oxidebot),
    }
}

/// Turns one state-aware completion function into a statically typed provider.
///
/// The function must be `async fn(Context<State>, CompletionInput) ->
/// HandlerResult<Vec<CompletionItem>>`. The generated unit value can be used by
/// `#[arg(complete = provider)]` without a global registry or dynamic state map.
#[proc_macro_attribute]
pub fn completer(attribute: TokenStream, input: TokenStream) -> TokenStream {
    if !attribute.is_empty() {
        return syn::Error::new(
            proc_macro2::Span::call_site(),
            "#[oxidebot::completer] does not accept arguments",
        )
        .into_compile_error()
        .into();
    }
    match expand_completer_function(parse_macro_input!(input as ItemFn)) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.into_compile_error().into(),
    }
}

/// Binds one command-tree branch to an ordinary async function.
///
/// The first function parameter is the branch argument value. Use
/// `#[oxidebot::branch(path, unit)]` for a unit branch with no argument value.
#[proc_macro_attribute]
pub fn branch(attribute: TokenStream, input: TokenStream) -> TokenStream {
    match expand_branch_function(
        parse_macro_input!(attribute as BranchAttribute),
        parse_macro_input!(input as ItemFn),
    ) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.into_compile_error().into(),
    }
}

/// Turns an async function into a typed OxideBot command feature.
///
/// The optional string literal supplies the command name; otherwise the
/// function name is converted from snake case to kebab case.
#[proc_macro_attribute]
pub fn command(attribute: TokenStream, input: TokenStream) -> TokenStream {
    match macro_command::expand_command_function(attribute, parse_macro_input!(input as ItemFn)) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.into_compile_error().into(),
    }
}

fn to_pascal_case(value: &str) -> String {
    value
        .split('_')
        .filter(|component| !component.is_empty())
        .map(|component| {
            let mut chars = component.chars();
            chars
                .next()
                .map(|first| first.to_uppercase().collect::<String>() + chars.as_str())
                .unwrap_or_default()
        })
        .collect()
}

/// Derives a complete command tree from an enum or struct command schema.
#[proc_macro_derive(BotCommand, attributes(command))]
pub fn derive_bot_command(input: TokenStream) -> TokenStream {
    match macro_tree::expand_bot_command(parse_macro_input!(input as DeriveInput)) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.into_compile_error().into(),
    }
}

/// Derives typed command argument parsing for a struct.
#[proc_macro_derive(CommandArgs, attributes(arg, command))]
pub fn derive_command_args(input: TokenStream) -> TokenStream {
    match macro_args::expand_command_args(parse_macro_input!(input as DeriveInput)) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.into_compile_error().into(),
    }
}

/// Derives a finite, strongly typed command choice enum.
#[proc_macro_derive(CommandEnum, attributes(choice))]
pub fn derive_command_enum(input: TokenStream) -> TokenStream {
    match expand_command_enum(parse_macro_input!(input as DeriveInput)) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.into_compile_error().into(),
    }
}

/// Derives efficient `FromState` projections for an application state type.
#[proc_macro_derive(BotState, attributes(state))]
pub fn derive_bot_state(input: TokenStream) -> TokenStream {
    match macro_dialogue::expand_bot_state(parse_macro_input!(input as DeriveInput)) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.into_compile_error().into(),
    }
}

/// Derives a bounded, typed sequence of dialogue questions for a named struct.
///
/// Fields accept `#[dialogue(prompt = "...", attempts = 3, error = "...")]`,
/// `#[dialogue(confirm)]`, repeated `choice = "Label=value"`, and
/// `validate = path`.
#[proc_macro_derive(DialogueForm, attributes(dialogue))]
pub fn derive_dialogue_form(input: TokenStream) -> TokenStream {
    match macro_dialogue::expand_dialogue_form(parse_macro_input!(input as DeriveInput)) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.into_compile_error().into(),
    }
}

#[derive(Default)]
struct FieldOptions {
    name: Option<String>,
    help: Option<String>,
    heading: Option<String>,
    prompt: Option<String>,
    value_name: Option<String>,
    long: Option<Option<String>>,
    short: Option<Option<char>>,
    default: Option<Expr>,
    default_uses_default: bool,
    required: Option<bool>,
    multiple: bool,
    rest: bool,
    flag: bool,
    hidden: bool,
    skip: bool,
    kind: Option<String>,
    action: Option<String>,
    choices: Vec<String>,
    autocomplete: bool,
    value_enum: bool,
    complete: Option<Path>,
    min_value: Option<Expr>,
    max_value: Option<Expr>,
    min_length: Option<Expr>,
    max_length: Option<Expr>,
    requires: Vec<String>,
    conflicts: Vec<String>,
    groups: Vec<String>,
    validate: Option<Path>,
}

impl FieldOptions {
    fn parse(field: &Field) -> syn::Result<Self> {
        let mut output = Self::default();
        for attribute in field
            .attrs
            .iter()
            .filter(|attribute| attribute.path().is_ident("arg"))
        {
            attribute.parse_nested_meta(|meta| {
                if meta.path.is_ident("name") {
                    output.name = Some(meta.value()?.parse::<LitStr>()?.value());
                } else if meta.path.is_ident("help") {
                    output.help = Some(meta.value()?.parse::<LitStr>()?.value());
                } else if meta.path.is_ident("heading") {
                    output.heading = Some(meta.value()?.parse::<LitStr>()?.value());
                } else if meta.path.is_ident("prompt") {
                    output.prompt = Some(meta.value()?.parse::<LitStr>()?.value());
                } else if meta.path.is_ident("value_name") {
                    output.value_name = Some(meta.value()?.parse::<LitStr>()?.value());
                } else if meta.path.is_ident("long") {
                    output.long = Some(if meta.input.peek(syn::Token![=]) {
                        Some(meta.value()?.parse::<LitStr>()?.value())
                    } else {
                        None
                    });
                } else if meta.path.is_ident("short") {
                    output.short = Some(if meta.input.peek(syn::Token![=]) {
                        Some(meta.value()?.parse::<LitChar>()?.value())
                    } else {
                        None
                    });
                } else if meta.path.is_ident("default") {
                    if meta.input.peek(syn::Token![=]) {
                        output.default = Some(meta.value()?.parse::<Expr>()?);
                    } else {
                        output.default_uses_default = true;
                        output.default =
                            Some(syn::parse_quote!(::core::default::Default::default()));
                    }
                } else if meta.path.is_ident("required") {
                    output.required = Some(meta.value()?.parse::<syn::LitBool>()?.value());
                } else if meta.path.is_ident("multiple") {
                    output.multiple = true;
                } else if meta.path.is_ident("rest") {
                    output.rest = true;
                } else if meta.path.is_ident("flag") {
                    output.flag = true;
                } else if meta.path.is_ident("hidden") {
                    output.hidden = true;
                } else if meta.path.is_ident("skip") {
                    output.skip = true;
                } else if meta.path.is_ident("kind") {
                    output.kind = Some(meta.value()?.parse::<LitStr>()?.value());
                } else if meta.path.is_ident("action") {
                    output.action = Some(meta.value()?.parse::<LitStr>()?.value());
                } else if meta.path.is_ident("choice") {
                    output
                        .choices
                        .push(meta.value()?.parse::<LitStr>()?.value());
                } else if meta.path.is_ident("autocomplete") {
                    output.autocomplete = if meta.input.peek(syn::Token![=]) {
                        meta.value()?.parse::<syn::LitBool>()?.value()
                    } else {
                        true
                    };
                } else if meta.path.is_ident("value_enum") {
                    output.value_enum = if meta.input.peek(syn::Token![=]) {
                        meta.value()?.parse::<syn::LitBool>()?.value
                    } else {
                        true
                    };
                } else if meta.path.is_ident("complete") {
                    output.complete = Some(meta.value()?.parse::<Path>()?);
                    output.autocomplete = true;
                } else if meta.path.is_ident("min") || meta.path.is_ident("min_value") {
                    output.min_value = Some(meta.value()?.parse::<Expr>()?);
                } else if meta.path.is_ident("max") || meta.path.is_ident("max_value") {
                    output.max_value = Some(meta.value()?.parse::<Expr>()?);
                } else if meta.path.is_ident("min_length") {
                    output.min_length = Some(meta.value()?.parse::<Expr>()?);
                } else if meta.path.is_ident("max_length") {
                    output.max_length = Some(meta.value()?.parse::<Expr>()?);
                } else if meta.path.is_ident("requires") {
                    output
                        .requires
                        .push(meta.value()?.parse::<LitStr>()?.value());
                } else if meta.path.is_ident("conflicts_with") {
                    output
                        .conflicts
                        .push(meta.value()?.parse::<LitStr>()?.value());
                } else if meta.path.is_ident("group") {
                    output.groups.push(meta.value()?.parse::<LitStr>()?.value());
                } else if meta.path.is_ident("validate") {
                    output.validate = Some(meta.value()?.parse::<Path>()?);
                } else {
                    return Err(meta.error("unknown #[arg(...)] option"));
                }
                Ok(())
            })?;
        }
        Ok(output)
    }
}

#[derive(Default)]
struct CommandOptions {
    name: Option<String>,
    description: Option<String>,
    category: Option<String>,
    aliases: Vec<String>,
    prefixes: Vec<String>,
    case_insensitive: bool,
    hidden: bool,
    subcommand: bool,
    groups: Vec<GroupOptions>,
    global_args: Option<Type>,
    interactive: bool,
}

struct GroupOptions {
    name: String,
    required: bool,
    multiple: bool,
}

impl Default for GroupOptions {
    fn default() -> Self {
        Self {
            name: String::new(),
            required: false,
            multiple: true,
        }
    }
}

impl CommandOptions {
    fn parse(attributes: &[Attribute]) -> syn::Result<Self> {
        let mut output = Self::default();
        for attribute in attributes
            .iter()
            .filter(|attribute| attribute.path().is_ident("command"))
        {
            attribute.parse_nested_meta(|meta| {
                if meta.path.is_ident("name") {
                    output.name = Some(meta.value()?.parse::<LitStr>()?.value());
                } else if meta.path.is_ident("description") {
                    output.description = Some(meta.value()?.parse::<LitStr>()?.value());
                } else if meta.path.is_ident("category") {
                    output.category = Some(meta.value()?.parse::<LitStr>()?.value());
                } else if meta.path.is_ident("alias") {
                    output
                        .aliases
                        .push(meta.value()?.parse::<LitStr>()?.value());
                } else if meta.path.is_ident("prefix") {
                    output
                        .prefixes
                        .push(meta.value()?.parse::<LitStr>()?.value());
                } else if meta.path.is_ident("no_prefix") {
                    output.prefixes.push(String::new());
                } else if meta.path.is_ident("case_insensitive") {
                    output.case_insensitive = true;
                } else if meta.path.is_ident("hidden") {
                    output.hidden = true;
                } else if meta.path.is_ident("subcommand") {
                    output.subcommand = true;
                } else if meta.path.is_ident("global_args") {
                    output.global_args = Some(meta.value()?.parse::<Type>()?);
                } else if meta.path.is_ident("interactive") {
                    output.interactive = if meta.input.peek(syn::Token![=]) {
                        meta.value()?.parse::<syn::LitBool>()?.value
                    } else {
                        true
                    };
                } else if meta.path.is_ident("group") {
                    let mut group = GroupOptions::default();
                    meta.parse_nested_meta(|group_meta| {
                        if group_meta.path.is_ident("name") {
                            group.name = group_meta.value()?.parse::<LitStr>()?.value();
                        } else if group_meta.path.is_ident("required") {
                            group.required = if group_meta.input.peek(syn::Token![=]) {
                                group_meta.value()?.parse::<syn::LitBool>()?.value
                            } else {
                                true
                            };
                        } else if group_meta.path.is_ident("multiple") {
                            group.multiple = group_meta.value()?.parse::<syn::LitBool>()?.value;
                        } else if group_meta.path.is_ident("exactly_one") {
                            group.required = true;
                            group.multiple = false;
                        } else {
                            return Err(group_meta.error("unknown command group option"));
                        }
                        Ok(())
                    })?;
                    output.groups.push(group);
                } else {
                    return Err(meta.error("unknown #[command(...)] option"));
                }
                Ok(())
            })?;
        }
        Ok(output)
    }

    fn builder_tokens(&self) -> proc_macro2::TokenStream {
        let oxidebot = oxidebot_crate();
        let mut expression = quote! { command };
        if let Some(description) = &self.description {
            expression = quote! { #expression.description(#description) };
        }
        if let Some(category) = &self.category {
            expression = quote! { #expression.category(#category) };
        }
        for alias in &self.aliases {
            expression = quote! { #expression.alias(#alias) };
        }
        if !self.prefixes.is_empty() {
            let prefixes = &self.prefixes;
            expression = quote! { #expression.prefixes([#(#prefixes),*]) };
        }
        if self.case_insensitive {
            expression = quote! { #expression.case_insensitive() };
        }
        if self.hidden {
            expression = quote! { #expression.hidden() };
        }
        if self.interactive {
            expression = quote! {
                #expression.completion(#oxidebot::runtime::CompletionConfig::new())
            };
        }
        expression
    }
}

fn doc_string(attributes: &[Attribute]) -> Option<String> {
    let lines = attributes.iter().filter_map(|attribute| {
        if !attribute.path().is_ident("doc") {
            return None;
        }
        let syn::Meta::NameValue(meta) = &attribute.meta else {
            return None;
        };
        let Expr::Lit(value) = &meta.value else {
            return None;
        };
        let syn::Lit::Str(value) = &value.lit else {
            return None;
        };
        Some(value.value().trim().to_owned())
    });
    let text = lines
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    (!text.is_empty()).then_some(text)
}
