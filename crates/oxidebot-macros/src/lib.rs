use proc_macro::TokenStream;
use quote::{quote, ToTokens};
use syn::{
    parse_macro_input, spanned::Spanned, Attribute, Data, DeriveInput, Expr, Field, Fields,
    GenericArgument, LitChar, LitStr, PathArguments, Type,
};

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
        let kind = FieldKind::of(&field.ty);
        let is_bool = matches!(&kind, FieldKind::Plain(ty) if is_type(ty, "bool"));
        let is_option = matches!(&kind, FieldKind::Option(_));
        let is_vec = matches!(&kind, FieldKind::Vec(_));
        let flag = options.flag || (is_bool && (options.long.is_some() || options.short.is_some()));
        let multiple = options.multiple || is_vec;

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
                || options.flag;
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
        if flag && !is_bool {
            return Err(syn::Error::new(
                field.ty.span(),
                "command flags must use the plain bool type",
            ));
        }
        if flag && options.long.is_none() && options.short.is_none() {
            return Err(syn::Error::new(
                field.span(),
                "#[arg(flag)] also needs #[arg(long)] or #[arg(short)]",
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
            !matches!(&kind, FieldKind::Option(_) | FieldKind::Vec(_))
                && options.default.is_none()
                && !flag
        });
        match &kind {
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

        let mut schema = quote! {
            ::oxidebot::ArgumentSpec::new(#argument_name)
                .required(#required)
                .multiple(#multiple)
                .rest(#rest)
                .flag(#flag)
        };
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
        schema_fields.push(quote! {
            schema = schema.argument(#schema);
        });

        let initializer = match kind {
            FieldKind::Option(inner) => quote! {
                arguments.optional::<#inner>(#argument_name)?
            },
            FieldKind::Vec(inner) => quote! {
                arguments.many::<#inner>(#argument_name)?
            },
            FieldKind::Plain(_ty) if flag => quote! {
                arguments.flag(#argument_name)
            },
            FieldKind::Plain(ty) => {
                if let Some(default) = options.default {
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
                if meta.path.is_ident("description") {
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
