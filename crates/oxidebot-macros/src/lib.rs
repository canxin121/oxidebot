use proc_macro::TokenStream;
use quote::{quote, ToTokens};
use syn::{
    parse_macro_input, spanned::Spanned, Attribute, Data, DeriveInput, Expr, Field, Fields,
    GenericArgument, LitChar, LitStr, PathArguments, Type,
};

#[proc_macro_derive(BotCommand, attributes(command))]
pub fn derive_bot_command(input: TokenStream) -> TokenStream {
    match expand_bot_command(parse_macro_input!(input as DeriveInput)) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.into_compile_error().into(),
    }
}

#[proc_macro_derive(CommandArgs, attributes(arg, command))]
pub fn derive_command_args(input: TokenStream) -> TokenStream {
    match expand_command_args(parse_macro_input!(input as DeriveInput)) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.into_compile_error().into(),
    }
}

fn expand_command_args(input: DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
    let name = input.ident;
    let Data::Struct(data) = input.data else {
        return Err(syn::Error::new(
            name.span(),
            "CommandArgs can only be derived for a struct",
        ));
    };
    let Fields::Named(fields) = data.fields else {
        return Err(syn::Error::new(
            name.span(),
            "CommandArgs requires named fields",
        ));
    };

    let mut command_options = CommandOptions::parse(&input.attrs)?;
    if command_options.description.is_none() {
        command_options.description = doc_string(&input.attrs);
    }
    let mut schema_fields = Vec::new();
    let mut initializers = Vec::new();
    let mut inferred_bounds = Vec::<syn::WherePredicate>::new();

    for field in fields.named {
        let options = FieldOptions::parse(&field)?;
        let ident = field
            .ident
            .clone()
            .ok_or_else(|| syn::Error::new(field.span(), "expected a named field"))?;
        let rust_name = ident.to_string();
        let argument_name = options.name.clone().unwrap_or_else(|| rust_name.clone());
        let field_kind = FieldKind::of(&field.ty);
        let value_ty = match &field_kind {
            FieldKind::Plain(ty) | FieldKind::Option(ty) | FieldKind::Vec(ty) => *ty,
        };
        let is_bool = matches!(&field_kind, FieldKind::Plain(ty) if is_type(ty, "bool"));
        let is_integer = matches!(&field_kind, FieldKind::Plain(ty) if is_integer_type(ty));
        let is_option = matches!(&field_kind, FieldKind::Option(_));
        let is_vec = matches!(&field_kind, FieldKind::Vec(_));
        let action_name = options
            .action
            .as_deref()
            .map(|value| value.trim().to_ascii_lowercase().replace('-', "_"));
        let action_is_count = action_name.as_deref() == Some("count");
        let action_is_append = action_name.as_deref() == Some("append");
        let action_is_set_true = matches!(action_name.as_deref(), Some("set_true" | "true"));
        let action_is_set_false = matches!(action_name.as_deref(), Some("set_false" | "false"));
        let flag = options.flag
            || action_is_count
            || action_is_set_true
            || action_is_set_false
            || (is_bool && (options.long.is_some() || options.short.is_some()));
        let multiple = options.multiple || is_vec || action_is_append;

        if options.skip {
            let has_conflicting_option = options.name.is_some()
                || options.help.is_some()
                || options.prompt.is_some()
                || options.value_name.is_some()
                || options.long.is_some()
                || options.short.is_some()
                || options.default.is_some()
                || options.required.is_some()
                || options.multiple
                || options.rest
                || options.flag
                || options.kind.is_some()
                || options.action.is_some()
                || !options.choices.is_empty()
                || options.autocomplete
                || options.min_value.is_some()
                || options.max_value.is_some()
                || options.min_length.is_some()
                || options.max_length.is_some()
                || !options.requires.is_empty()
                || !options.conflicts.is_empty();
            if has_conflicting_option {
                return Err(syn::Error::new(
                    field.span(),
                    "#[arg(skip)] cannot be combined with parsing options",
                ));
            }
            let ty = &field.ty;
            inferred_bounds.push(syn::parse_quote!(
                #ty: ::core::default::Default
            ));
            initializers.push(quote! {
                #ident: ::core::default::Default::default()
            });
            continue;
        }
        if action_is_count && !is_integer {
            return Err(syn::Error::new(
                field.ty.span(),
                "#[arg(action = \"count\")] requires a plain integer field",
            ));
        }
        if (action_is_set_true || action_is_set_false) && !is_bool {
            return Err(syn::Error::new(
                field.ty.span(),
                "set_true and set_false actions require the plain bool type",
            ));
        }
        if action_is_append && !is_vec {
            return Err(syn::Error::new(
                field.ty.span(),
                "#[arg(action = \"append\")] requires Vec<T>",
            ));
        }
        if flag && !is_bool && !action_is_count {
            return Err(syn::Error::new(
                field.ty.span(),
                "command flags must use bool unless they use the count action",
            ));
        }
        if flag && options.long.is_none() && options.short.is_none() {
            return Err(syn::Error::new(
                field.span(),
                "flag-like command arguments need #[arg(long)] or #[arg(short)]",
            ));
        }
        if flag && options.default.is_some() {
            return Err(syn::Error::new(
                field.span(),
                "command flags cannot have a default expression",
            ));
        }
        if flag && options.required == Some(true) {
            return Err(syn::Error::new(
                field.span(),
                "command flags cannot be required",
            ));
        }
        if options.rest && (options.long.is_some() || options.short.is_some()) {
            return Err(syn::Error::new(
                field.span(),
                "#[arg(rest)] cannot be combined with a long or short option",
            ));
        }
        if (options.multiple || options.rest) && !is_vec {
            return Err(syn::Error::new(
                field.ty.span(),
                "#[arg(multiple)] and #[arg(rest)] require Vec<T>",
            ));
        }
        if options.default.is_some() && (is_option || is_vec) {
            return Err(syn::Error::new(
                field.ty.span(),
                "#[arg(default = ...)] is only supported on a plain field; use Option<T> or Vec<T> without a default",
            ));
        }
        if options.default.is_some() && options.required == Some(true) {
            return Err(syn::Error::new(
                field.span(),
                "a defaulted command argument cannot also be required",
            ));
        }
        if options.required == Some(false)
            && !is_option
            && !is_vec
            && options.default.is_none()
            && !flag
        {
            return Err(syn::Error::new(
                field.ty.span(),
                "a non-required plain field needs #[arg(default = ...)] or an Option<T> type",
            ));
        }
        let required = options.required.unwrap_or_else(|| {
            !matches!(&field_kind, FieldKind::Option(_) | FieldKind::Vec(_))
                && options.default.is_none()
                && !flag
        });
        match &field_kind {
            FieldKind::Plain(ty) if !flag => inferred_bounds.push(syn::parse_quote!(
                #ty: ::oxidebot::FromCommandValue
            )),
            FieldKind::Option(ty) | FieldKind::Vec(ty) => {
                inferred_bounds.push(syn::parse_quote!(
                    #ty: ::oxidebot::FromCommandValue
                ));
            }
            FieldKind::Plain(_) => {}
        }
        if options.default_uses_default {
            let ty = &field.ty;
            inferred_bounds.push(syn::parse_quote!(
                #ty: ::core::default::Default
            ));
        }
        let long = options
            .long
            .as_ref()
            .map(|value| value.clone().unwrap_or_else(|| rust_name.replace('_', "-")));
        let short = options
            .short
            .as_ref()
            .map(|value| value.unwrap_or_else(|| rust_name.chars().next().unwrap_or('x')));
        let help = options
            .help
            .clone()
            .or_else(|| doc_string(&field.attrs))
            .unwrap_or_default();
        let prompt = options.prompt.clone();
        let value_name = options.value_name.clone();
        let rest = options.rest;
        let default_display = options
            .default
            .as_ref()
            .map(ToTokens::to_token_stream)
            .map(|tokens| tokens.to_string());
        let kind_tokens = command_value_kind_tokens(value_ty, options.kind.as_deref())?;
        let action_tokens = argument_action_tokens(options.action.as_deref(), field.span())?;

        let mut schema = quote! {
            ::oxidebot::ArgumentSpec::new(#argument_name)
                .required(#required)
                .multiple(#multiple)
                .rest(#rest)
                .flag(#flag)
                .kind(#kind_tokens)
        };
        if let Some(action) = action_tokens {
            schema = quote! { #schema.action(#action) };
        }
        if !help.is_empty() {
            schema = quote! { #schema.help(#help) };
        }
        if let Some(prompt) = prompt {
            schema = quote! { #schema.prompt(#prompt) };
        }
        if let Some(value_name) = value_name {
            schema = quote! { #schema.value_name(#value_name) };
        }
        if let Some(long) = long {
            schema = quote! { #schema.long(#long) };
        }
        if let Some(short) = short {
            schema = quote! { #schema.short(#short) };
        }
        if let Some(default) = default_display {
            schema = quote! { #schema.default_value(#default) };
        }
        for choice in &options.choices {
            schema = quote! {
                #schema.choice(::oxidebot::ArgumentChoice::new(#choice, #choice))
            };
        }
        if options.autocomplete {
            schema = quote! { #schema.autocomplete(true) };
        }
        if let Some(value) = &options.min_value {
            schema = quote! { #schema.min_value((#value) as f64) };
        }
        if let Some(value) = &options.max_value {
            schema = quote! { #schema.max_value((#value) as f64) };
        }
        if let Some(value) = &options.min_length {
            schema = quote! { #schema.min_length((#value) as u32) };
        }
        if let Some(value) = &options.max_length {
            schema = quote! { #schema.max_length((#value) as u32) };
        }
        for required_field in &options.requires {
            schema = quote! { #schema.requires(#required_field) };
        }
        for conflict in &options.conflicts {
            schema = quote! { #schema.conflicts_with(#conflict) };
        }
        schema_fields.push(quote! {
            schema = schema.argument(#schema);
        });

        let initializer = match field_kind {
            FieldKind::Option(inner) => quote! {
                arguments.optional::<#inner>(#argument_name)?
            },
            FieldKind::Vec(inner) => quote! {
                arguments.many::<#inner>(#argument_name)?
            },
            FieldKind::Plain(ty) if action_is_count => quote! {
                arguments.count(#argument_name) as #ty
            },
            FieldKind::Plain(_ty) if action_is_set_false => quote! {
                if arguments.contains(#argument_name) {
                    arguments.flag(#argument_name)
                } else {
                    true
                }
            },
            FieldKind::Plain(_ty) if flag => quote! {
                arguments.flag(#argument_name)
            },
            FieldKind::Plain(ty) => {
                if let Some(default) = &options.default {
                    quote! {
                        arguments.optional::<#ty>(#argument_name)?.unwrap_or_else(|| #default)
                    }
                } else {
                    quote! {
                        arguments.required::<#ty>(#argument_name)?
                    }
                }
            }
        };
        initializers.push(quote! { #ident: #initializer });
    }

    let command_builder = command_options.builder_tokens();
    let mut generics = input.generics;
    let predicates = &mut generics.make_where_clause().predicates;
    predicates.extend(inferred_bounds);
    predicates.push(syn::parse_quote!(Self: ::core::marker::Send + 'static));
    let (impl_generics, type_generics, where_clause) = generics.split_for_impl();

    Ok(quote! {
        impl #impl_generics ::oxidebot::CommandArgs for #name #type_generics #where_clause {
            fn schema() -> ::oxidebot::CommandSchema {
                let mut schema = ::oxidebot::CommandSchema::new();
                #(#schema_fields)*
                schema
            }

            fn from_arguments(
                arguments: &::oxidebot::ParsedArguments,
            ) -> ::core::result::Result<Self, ::oxidebot::CommandParseError> {
                ::core::result::Result::Ok(Self {
                    #(#initializers,)*
                })
            }

            fn command(
                name: impl ::core::convert::Into<::std::sync::Arc<str>>,
            ) -> ::oxidebot::Command {
                let command = ::oxidebot::Command::new(name).schema(Self::schema());
                #command_builder
            }
        }
    })
}

fn expand_bot_command(input: DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
    let enum_name = input.ident;
    let Data::Enum(data) = input.data else {
        return Err(syn::Error::new(
            enum_name.span(),
            "BotCommand can only be derived for an enum",
        ));
    };
    let mut root_options = CommandOptions::parse(&input.attrs)?;
    if root_options.description.is_none() {
        root_options.description = doc_string(&input.attrs);
    }
    let root_name = root_options
        .name
        .clone()
        .unwrap_or_else(|| ident_to_command_name(&enum_name.to_string()));
    let root_builder = root_options.builder_tokens();
    let mut branch_builders = Vec::new();
    let mut match_arms = Vec::new();
    let mut inferred_bounds = Vec::<syn::WherePredicate>::new();

    for variant in data.variants {
        let ident = variant.ident;
        let mut options = CommandOptions::parse(&variant.attrs)?;
        if options.description.is_none() {
            options.description = doc_string(&variant.attrs);
        }
        let branch_name = options
            .name
            .clone()
            .unwrap_or_else(|| ident_to_command_name(&ident.to_string()));
        let description = options.description.clone();
        let aliases = options.aliases.clone();
        let hidden = options.hidden;

        match variant.fields {
            Fields::Unit => {
                let mut builder = quote! {
                    let mut branch = ::oxidebot::CommandBranch::new(#branch_name);
                };
                if let Some(description) = description {
                    builder.extend(quote! {
                        branch = branch.description(#description);
                    });
                }
                for alias in aliases {
                    builder.extend(quote! {
                        branch = branch.alias(#alias);
                    });
                }
                if hidden {
                    builder.extend(quote! {
                        branch = branch.hidden();
                    });
                }
                builder.extend(quote! {
                    command = command.subcommand(branch);
                });
                branch_builders.push(builder);
                match_arms.push(quote! {
                    ::core::option::Option::Some(#branch_name) => {
                        ::core::result::Result::Ok(Self::#ident)
                    },
                });
            }
            Fields::Unnamed(fields) if fields.unnamed.len() == 1 => {
                let ty = &fields.unnamed.first().expect("length checked").ty;
                inferred_bounds.push(syn::parse_quote!(#ty: ::oxidebot::CommandArgs));
                let mut builder = quote! {
                    let mut branch = ::oxidebot::CommandBranch::new(#branch_name)
                        .schema(<#ty as ::oxidebot::CommandArgs>::schema());
                };
                if let Some(description) = description {
                    builder.extend(quote! {
                        branch = branch.description(#description);
                    });
                }
                for alias in aliases {
                    builder.extend(quote! {
                        branch = branch.alias(#alias);
                    });
                }
                if hidden {
                    builder.extend(quote! {
                        branch = branch.hidden();
                    });
                }
                builder.extend(quote! {
                    command = command.subcommand(branch);
                });
                branch_builders.push(builder);
                match_arms.push(quote! {
                    ::core::option::Option::Some(#branch_name) => {
                        let arguments = if let ::core::option::Option::Some(arguments) = result.arguments() {
                            arguments.clone()
                        } else {
                            result.parse_active()?
                        };
                        ::core::result::Result::Ok(Self::#ident(
                            <#ty as ::oxidebot::CommandArgs>::from_arguments(&arguments)?,
                        ))
                    },
                });
            }
            Fields::Unnamed(fields) => {
                return Err(syn::Error::new(
                    fields.span(),
                    "BotCommand tuple variants must contain exactly one CommandArgs type",
                ));
            }
            Fields::Named(fields) => {
                return Err(syn::Error::new(
                    fields.span(),
                    "BotCommand named variants should wrap a #[derive(CommandArgs)] struct",
                ));
            }
        }
    }

    let mut generics = input.generics;
    generics
        .make_where_clause()
        .predicates
        .extend(inferred_bounds);
    let (impl_generics, type_generics, where_clause) = generics.split_for_impl();

    Ok(quote! {
        impl #impl_generics ::oxidebot::FromCommandMatch for #enum_name #type_generics #where_clause {
            fn from_match(
                result: &::oxidebot::CommandMatch,
            ) -> ::core::result::Result<Self, ::oxidebot::CommandParseError> {
                match result.selected_branch_name() {
                    #(#match_arms)*
                    ::core::option::Option::Some(_) => ::core::result::Result::Err(
                        ::oxidebot::CommandParseError::InvalidValue {
                            value: result.selected_branch_name().unwrap_or_default().to_owned(),
                            expected: "known subcommand",
                            reason: "the selected command branch is not represented by this enum".to_owned(),
                        },
                    ),
                    ::core::option::Option::None => ::core::result::Result::Err(
                        ::oxidebot::CommandParseError::MissingSubcommand {
                            choices: ::std::sync::Arc::from(""),
                        },
                    ),
                }
            }
        }

        impl #impl_generics ::oxidebot::CommandTree for #enum_name #type_generics #where_clause {
            fn command() -> ::oxidebot::Command {
                let command = ::oxidebot::Command::new(#root_name);
                let mut command = #root_builder;
                #(#branch_builders)*
                command
            }
        }
    })
}

fn ident_to_command_name(value: &str) -> String {
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

enum FieldKind<'a> {
    Plain(&'a Type),
    Option(&'a Type),
    Vec(&'a Type),
}

impl<'a> FieldKind<'a> {
    fn of(ty: &'a Type) -> Self {
        if let Some(inner) = type_argument(ty, "Option") {
            Self::Option(inner)
        } else if let Some(inner) = type_argument(ty, "Vec") {
            Self::Vec(inner)
        } else {
            Self::Plain(ty)
        }
    }
}

fn type_argument<'a>(ty: &'a Type, expected: &str) -> Option<&'a Type> {
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

fn is_type(ty: &Type, expected: &str) -> bool {
    matches!(ty, Type::Path(path) if path.path.segments.last().is_some_and(|segment| segment.ident == expected))
}

fn command_value_kind_tokens(
    ty: &Type,
    explicit: Option<&str>,
) -> syn::Result<proc_macro2::TokenStream> {
    if let Some(explicit) = explicit {
        let normalized = explicit.trim().to_ascii_lowercase().replace('-', "_");
        let tokens = match normalized.as_str() {
            "string" | "text" => quote!(::oxidebot::CommandValueKind::String),
            "integer" | "int" => quote!(::oxidebot::CommandValueKind::Integer),
            "number" | "float" => quote!(::oxidebot::CommandValueKind::Number),
            "boolean" | "bool" => quote!(::oxidebot::CommandValueKind::Boolean),
            "user" => quote!(::oxidebot::CommandValueKind::User),
            "conversation" | "channel" => quote!(::oxidebot::CommandValueKind::Conversation),
            "role" => quote!(::oxidebot::CommandValueKind::Role),
            "mention" | "mentionable" => quote!(::oxidebot::CommandValueKind::Mentionable),
            "attachment" | "file" => quote!(::oxidebot::CommandValueKind::Attachment),
            "message_segment" | "segment" => quote!(::oxidebot::CommandValueKind::MessageSegment),
            value if value.starts_with("native:") => {
                let value = value.trim_start_matches("native:").to_owned();
                quote!(::oxidebot::CommandValueKind::PlatformNative(
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
        quote!(::oxidebot::CommandValueKind::Boolean)
    } else if [
        "i8", "i16", "i32", "i64", "i128", "isize", "u8", "u16", "u32", "u64", "u128", "usize",
    ]
    .iter()
    .any(|name| is_type(ty, name))
    {
        quote!(::oxidebot::CommandValueKind::Integer)
    } else if is_type(ty, "f32") || is_type(ty, "f64") {
        quote!(::oxidebot::CommandValueKind::Number)
    } else if is_type(ty, "User") {
        quote!(::oxidebot::CommandValueKind::User)
    } else if is_type(ty, "ConversationRef") || is_type(ty, "MessageTarget") {
        quote!(::oxidebot::CommandValueKind::Conversation)
    } else if is_type(ty, "File") {
        quote!(::oxidebot::CommandValueKind::Attachment)
    } else if is_type(ty, "Mention") {
        quote!(::oxidebot::CommandValueKind::Mentionable)
    } else if is_type(ty, "MessageSegment") {
        quote!(::oxidebot::CommandValueKind::MessageSegment)
    } else {
        quote!(::oxidebot::CommandValueKind::String)
    };
    Ok(tokens)
}

fn argument_action_tokens(
    action: Option<&str>,
    span: proc_macro2::Span,
) -> syn::Result<Option<proc_macro2::TokenStream>> {
    let Some(action) = action else {
        return Ok(None);
    };
    let normalized = action.trim().to_ascii_lowercase().replace('-', "_");
    let tokens = match normalized.as_str() {
        "store" => quote!(::oxidebot::ArgumentAction::Store),
        "append" => quote!(::oxidebot::ArgumentAction::Append),
        "count" => quote!(::oxidebot::ArgumentAction::Count),
        "set_true" | "true" => quote!(::oxidebot::ArgumentAction::SetTrue),
        "set_false" | "false" => quote!(::oxidebot::ArgumentAction::SetFalse),
        _ => {
            return Err(syn::Error::new(
                span,
                "unknown command action; expected store, append, count, set_true, or set_false",
            ));
        }
    };
    Ok(Some(tokens))
}

fn is_integer_type(ty: &Type) -> bool {
    [
        "i8", "i16", "i32", "i64", "i128", "isize", "u8", "u16", "u32", "u64", "u128", "usize",
    ]
    .iter()
    .any(|name| is_type(ty, name))
}

#[derive(Default)]
struct FieldOptions {
    name: Option<String>,
    help: Option<String>,
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
    skip: bool,
    kind: Option<String>,
    action: Option<String>,
    choices: Vec<String>,
    autocomplete: bool,
    min_value: Option<Expr>,
    max_value: Option<Expr>,
    min_length: Option<Expr>,
    max_length: Option<Expr>,
    requires: Vec<String>,
    conflicts: Vec<String>,
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
                } else {
                    return Err(meta.error("unknown #[command(...)] option"));
                }
                Ok(())
            })?;
        }
        Ok(output)
    }

    fn builder_tokens(&self) -> proc_macro2::TokenStream {
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
