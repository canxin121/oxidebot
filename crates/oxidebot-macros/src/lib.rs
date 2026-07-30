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

fn expand_completer_function(mut function: ItemFn) -> syn::Result<proc_macro2::TokenStream> {
    let oxidebot = oxidebot_crate();
    if function.sig.asyncness.is_none() {
        return Err(syn::Error::new(
            function.sig.span(),
            "#[oxidebot::completer] requires an async function",
        ));
    }
    if !function.sig.generics.params.is_empty() {
        return Err(syn::Error::new(
            function.sig.generics.span(),
            "#[oxidebot::completer] does not support generic functions",
        ));
    }
    if function.sig.inputs.len() != 2 {
        return Err(syn::Error::new(
            function.sig.inputs.span(),
            "a completer must accept Context<State> and CompletionInput",
        ));
    }

    let mut inputs = function.sig.inputs.iter();
    let Some(FnArg::Typed(context)) = inputs.next() else {
        return Err(syn::Error::new(
            function.sig.inputs.span(),
            "methods are not supported",
        ));
    };
    let Some(state) = type_argument(context.ty.as_ref(), "Context").cloned() else {
        return Err(syn::Error::new(
            context.ty.span(),
            "the first completer parameter must be Context<State>",
        ));
    };
    let Some(FnArg::Typed(input)) = inputs.next() else {
        return Err(syn::Error::new(
            function.sig.inputs.span(),
            "methods are not supported",
        ));
    };
    if !is_type(input.ty.as_ref(), "CompletionInput") {
        return Err(syn::Error::new(
            input.ty.span(),
            "the second completer parameter must be CompletionInput",
        ));
    }

    let provider_ident = function.sig.ident.clone();
    let implementation_ident = format_ident!("__oxidebot_{}_implementation", provider_ident);
    let visibility = function.vis.clone();
    let outer_attributes = function.attrs.clone();
    let implementation_attributes = outer_attributes
        .iter()
        .filter(|attribute| {
            attribute.path().is_ident("cfg") || attribute.path().is_ident("cfg_attr")
        })
        .cloned()
        .collect::<Vec<_>>();
    let feature_attributes = outer_attributes
        .iter()
        .filter(|attribute| {
            attribute.path().is_ident("doc")
                || attribute.path().is_ident("cfg")
                || attribute.path().is_ident("cfg_attr")
                || attribute.path().is_ident("deprecated")
        })
        .cloned()
        .collect::<Vec<_>>();
    function.attrs = outer_attributes.clone();
    function.vis = syn::Visibility::Inherited;
    function.sig.ident = implementation_ident.clone();

    Ok(quote! {
        #[doc(hidden)]
        #function

        #(#feature_attributes)*
        #[allow(non_camel_case_types)]
        #[derive(Clone, Copy, Debug, Default)]
        #visibility struct #provider_ident;

        #(#implementation_attributes)*
        #[#oxidebot::runtime::__private::async_trait]
        impl #oxidebot::runtime::DynamicCompleter<#state> for #provider_ident {
            async fn complete(
                &self,
                context: &#oxidebot::runtime::Context<#state>,
                input: #oxidebot::runtime::CompletionInput,
            ) -> #oxidebot::runtime::HandlerResult<::std::vec::Vec<#oxidebot::runtime::CompletionItem>> {
                #implementation_ident(context.clone(), input).await
            }
        }
    })
}

struct BranchAttribute {
    path: Path,
    unit: bool,
}

impl Parse for BranchAttribute {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        let path = input.parse()?;
        let unit = if input.peek(Token![,]) {
            input.parse::<Token![,]>()?;
            let mode: Ident = input.parse()?;
            if mode != "unit" {
                return Err(syn::Error::new(mode.span(), "expected `unit`"));
            }
            true
        } else {
            false
        };
        if !input.is_empty() {
            return Err(input.error("unexpected branch attribute input"));
        }
        Ok(Self { path, unit })
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

fn expand_branch_function(
    attribute: BranchAttribute,
    mut function: ItemFn,
) -> syn::Result<proc_macro2::TokenStream> {
    let oxidebot = oxidebot_crate();
    if function.sig.asyncness.is_none() {
        return Err(syn::Error::new(
            function.sig.span(),
            "#[oxidebot::branch] requires an async function",
        ));
    }
    if !function.sig.generics.params.is_empty() {
        return Err(syn::Error::new(
            function.sig.generics.span(),
            "#[oxidebot::branch] does not support generic functions",
        ));
    }

    let BranchAttribute { path, unit } = attribute;
    let feature_ident = function.sig.ident.clone();
    let implementation_ident = format_ident!("__oxidebot_{}_implementation", feature_ident);
    let handler_ident = format_ident!("__oxidebot_{}_handler", feature_ident);
    let visibility = function.vis.clone();
    let return_type = function.sig.output.clone();
    let outer_attributes = function.attrs.clone();
    let implementation_attributes = outer_attributes
        .iter()
        .filter(|attribute| {
            attribute.path().is_ident("cfg") || attribute.path().is_ident("cfg_attr")
        })
        .cloned()
        .collect::<Vec<_>>();
    let feature_attributes = outer_attributes
        .iter()
        .filter(|attribute| {
            attribute.path().is_ident("doc")
                || attribute.path().is_ident("cfg")
                || attribute.path().is_ident("cfg_attr")
                || attribute.path().is_ident("deprecated")
        })
        .cloned()
        .collect::<Vec<_>>();
    function.attrs = outer_attributes.clone();
    function.vis = syn::Visibility::Inherited;
    function.sig.ident = implementation_ident.clone();

    let mut inputs = function.sig.inputs.iter().cloned().collect::<Vec<_>>();
    let branch_argument = if unit {
        None
    } else {
        if inputs.is_empty() {
            return Err(syn::Error::new(
                function.sig.inputs.span(),
                "a non-unit branch handler needs its branch arguments as the first parameter",
            ));
        }
        Some(inputs.remove(0))
    };

    let mut wrapper_inputs = Vec::new();
    let mut wrapper_types = vec![quote! { #oxidebot::runtime::BranchArgs<#path> }];
    let mut call_arguments = Vec::new();
    if let Some(argument) = branch_argument {
        let span = argument.span();
        let FnArg::Typed(_) = argument else {
            return Err(syn::Error::new(span, "methods are not supported"));
        };
        call_arguments.push(quote! { __oxidebot_branch_value });
    }

    for (index, argument) in inputs.into_iter().enumerate() {
        let span = argument.span();
        let FnArg::Typed(typed) = argument else {
            return Err(syn::Error::new(span, "methods are not supported"));
        };
        let ident = format_ident!("__oxidebot_extractor_{index}");
        let ty = typed.ty.clone();
        let attributes = typed.attrs.clone();
        wrapper_inputs.push(quote! { #(#attributes)* #ident: #ty });
        wrapper_types.push(quote! { #ty });
        call_arguments.push(quote! { #ident });
    }

    let branch_input = if unit {
        quote! { _: #oxidebot::runtime::BranchArgs<#path>, }
    } else {
        quote! {
            #oxidebot::runtime::BranchArgs(__oxidebot_branch_value): #oxidebot::runtime::BranchArgs<#path>,
        }
    };
    let extractor_bounds = wrapper_types
        .iter()
        .map(|ty| {
            quote! { #ty: #oxidebot::runtime::Extract<S> + ::core::marker::Send + 'static }
        })
        .collect::<Vec<_>>();
    let extractor_bounds_for_install = extractor_bounds.clone();
    let extractor_bounds_for_generated = extractor_bounds.clone();

    Ok(quote! {
        #[doc(hidden)]
        #function

        #(#implementation_attributes)*
        #[doc(hidden)]
        async fn #handler_ident(
            #branch_input
            #(#wrapper_inputs),*
        ) #return_type {
            #implementation_ident(#(#call_arguments),*).await
        }

        #(#feature_attributes)*
        #[allow(non_camel_case_types)]
        #[derive(Clone, Copy, Debug, Default)]
        #visibility struct #feature_ident;

        #(#implementation_attributes)*
        impl #feature_ident {
            #[must_use]
            #visibility fn feature<S>(self) -> #oxidebot::runtime::Feature<S>
            where
                S: ::core::marker::Send + ::core::marker::Sync + 'static,
                #(#extractor_bounds,)*
            {
                #oxidebot::runtime::Feature::command_branch(#path, #handler_ident)
            }
        }

        #(#implementation_attributes)*
        impl<S> #oxidebot::runtime::IntoFeature<S> for #feature_ident
        where
            S: ::core::marker::Send + ::core::marker::Sync + 'static,
            #(#extractor_bounds_for_install,)*
        {
            fn install(self, module: #oxidebot::runtime::Module<S>) -> #oxidebot::runtime::Module<S> {
                <#oxidebot::runtime::Feature<S> as #oxidebot::runtime::IntoFeature<S>>::install(
                    self.feature(),
                    module,
                )
            }
        }

        #(#implementation_attributes)*
        impl<S> #oxidebot::runtime::GeneratedFeature<S> for #feature_ident
        where
            S: ::core::marker::Send + ::core::marker::Sync + 'static,
            #(#extractor_bounds_for_generated,)*
        {
            fn into_feature(self) -> #oxidebot::runtime::Feature<S> {
                self.feature()
            }
        }
    })
}

/// Turns an async function into a typed OxideBot command feature.
///
/// The optional string literal supplies the command name; otherwise the
/// function name is converted from snake case to kebab case.
#[proc_macro_attribute]
pub fn command(attribute: TokenStream, input: TokenStream) -> TokenStream {
    match expand_command_function(attribute, parse_macro_input!(input as ItemFn)) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.into_compile_error().into(),
    }
}

fn expand_command_function(
    attribute: TokenStream,
    mut function: ItemFn,
) -> syn::Result<proc_macro2::TokenStream> {
    let oxidebot = oxidebot_crate();
    if function.sig.asyncness.is_none() {
        return Err(syn::Error::new(
            function.sig.span(),
            "#[oxidebot::command] requires an async function",
        ));
    }
    if !function.sig.generics.params.is_empty() {
        return Err(syn::Error::new(
            function.sig.generics.span(),
            "#[oxidebot::command] does not support generic functions; use CommandArgs for reusable generic code",
        ));
    }

    let command_name = if attribute.is_empty() {
        function.sig.ident.to_string().replace('_', "-")
    } else {
        syn::parse::<LitStr>(attribute)?.value()
    };
    let feature_ident = function.sig.ident.clone();
    let implementation_ident = syn::Ident::new(
        &format!("__oxidebot_{}_implementation", feature_ident),
        feature_ident.span(),
    );
    let handler_ident = syn::Ident::new(
        &format!("__oxidebot_{}_handler", feature_ident),
        feature_ident.span(),
    );
    let args_ident = syn::Ident::new(
        &format!(
            "__OxideBot{}Args",
            to_pascal_case(&feature_ident.to_string())
        ),
        feature_ident.span(),
    );

    let visibility = function.vis.clone();
    let return_type = function.sig.output.clone();
    let outer_attributes = function.attrs.clone();
    let implementation_attributes = outer_attributes
        .iter()
        .filter(|attribute| {
            attribute.path().is_ident("cfg") || attribute.path().is_ident("cfg_attr")
        })
        .cloned()
        .collect::<Vec<_>>();
    let feature_attributes = outer_attributes
        .iter()
        .filter(|attribute| {
            attribute.path().is_ident("doc")
                || attribute.path().is_ident("cfg")
                || attribute.path().is_ident("cfg_attr")
                || attribute.path().is_ident("deprecated")
        })
        .cloned()
        .collect::<Vec<_>>();
    function.attrs = outer_attributes.clone();
    function.vis = syn::Visibility::Inherited;
    function.sig.ident = implementation_ident.clone();

    let mut command_fields = Vec::new();
    let mut completer_bindings = Vec::new();
    let mut completer_providers = Vec::<Path>::new();
    let generated_field_module =
        format_ident!("{}_fields", ident_to_module_name(&args_ident.to_string()),);
    let mut wrapper_inputs = Vec::new();
    let mut wrapper_types = Vec::new();
    let mut call_arguments = Vec::new();

    for (index, argument) in function.sig.inputs.iter_mut().enumerate() {
        let FnArg::Typed(typed) = argument else {
            return Err(syn::Error::new(
                argument.span(),
                "methods are not supported",
            ));
        };
        let ty = typed.ty.clone();
        let (arg_attributes, other_attributes): (Vec<_>, Vec<_>) = typed
            .attrs
            .drain(..)
            .partition(|attribute| attribute.path().is_ident("arg"));
        typed.attrs = other_attributes.clone();
        if arg_attributes.is_empty() {
            let ident = format_ident!("__oxidebot_extractor_{index}");
            wrapper_inputs.push(quote! { #(#other_attributes)* #ident: #ty });
            wrapper_types.push(quote! { #ty });
            call_arguments.push(quote! { #ident });
        } else {
            let Pat::Ident(pattern) = typed.pat.as_ref() else {
                return Err(syn::Error::new(
                    typed.pat.span(),
                    "command arguments declared with #[arg(...)] must use simple identifier patterns",
                ));
            };
            let ident = pattern.ident.clone();
            if let Some(provider) = arg_path_option(&arg_attributes, "complete")? {
                completer_providers.push(provider.clone());
                let marker = format_ident!(
                    "{}",
                    to_pascal_case(ident.to_string().trim_start_matches("r#")),
                );
                completer_bindings.push(quote! {
                    feature = feature.complete(
                        #generated_field_module::#marker,
                        #provider,
                    );
                });
            }
            command_fields.push(quote! {
                #(#arg_attributes)*
                #ident: #ty
            });
            call_arguments.push(quote! { __oxidebot_args.#ident });
        }
    }

    let args_declaration = if command_fields.is_empty() {
        quote! {}
    } else {
        quote! {
            #(#implementation_attributes)*
            #[doc(hidden)]
            #[allow(non_camel_case_types)]
            #[derive(#oxidebot::CommandArgs)]
            #visibility struct #args_ident {
                #(#command_fields,)*
            }
        }
    };
    let args_input = if command_fields.is_empty() {
        quote! {}
    } else {
        wrapper_types.insert(0, quote! { #oxidebot::runtime::Args<#args_ident> });
        quote! { #oxidebot::runtime::Args(__oxidebot_args): #oxidebot::runtime::Args<#args_ident>, }
    };
    let mut command_builder = if command_fields.is_empty() {
        quote! { #oxidebot::runtime::command(#command_name) }
    } else {
        quote! { #oxidebot::runtime::command(#command_name).args::<#args_ident>() }
    };
    if let Some(description) = doc_string(&outer_attributes) {
        command_builder = quote! { #command_builder.description(#description) };
    }
    let extractor_bounds = wrapper_types
        .iter()
        .map(|ty| {
            quote! { #ty: #oxidebot::runtime::Extract<S> + ::core::marker::Send + 'static }
        })
        .collect::<Vec<_>>();
    let extractor_bounds_for_install = extractor_bounds.clone();
    let extractor_bounds_for_generated = extractor_bounds.clone();
    let completer_bounds = completer_providers
        .iter()
        .map(|provider| quote! { #provider: #oxidebot::runtime::DynamicCompleter<S> })
        .collect::<Vec<_>>();
    let completer_bounds_for_install = completer_bounds.clone();
    let completer_bounds_for_generated = completer_bounds.clone();

    Ok(quote! {
        #args_declaration

        #[doc(hidden)]
        #function

        #(#implementation_attributes)*
        #[doc(hidden)]
        async fn #handler_ident(
            #args_input
            #(#wrapper_inputs),*
        ) #return_type {
            #implementation_ident(#(#call_arguments),*).await
        }

        #(#feature_attributes)*
        #[allow(non_camel_case_types)]
        #[derive(Clone, Copy, Debug, Default)]
        #visibility struct #feature_ident;

        #(#implementation_attributes)*
        impl #feature_ident {
            #[must_use]
            #visibility fn command() -> #oxidebot::runtime::Command {
                #command_builder
            }

            #[must_use]
            #visibility fn feature<S>(self) -> #oxidebot::runtime::Feature<S>
            where
                S: ::core::marker::Send + ::core::marker::Sync + 'static,
                #(#extractor_bounds,)*
                #(#completer_bounds,)*
            {
                let mut feature = #oxidebot::runtime::Feature::command(Self::command(), #handler_ident);
                #(#completer_bindings)*
                feature
            }
        }

        #(#implementation_attributes)*
        impl<S> #oxidebot::runtime::IntoFeature<S> for #feature_ident
        where
            S: ::core::marker::Send + ::core::marker::Sync + 'static,
            #(#extractor_bounds_for_install,)*
            #(#completer_bounds_for_install,)*
        {
            fn install(self, module: #oxidebot::runtime::Module<S>) -> #oxidebot::runtime::Module<S> {
                <#oxidebot::runtime::Feature<S> as #oxidebot::runtime::IntoFeature<S>>::install(
                    self.feature(),
                    module,
                )
            }
        }

        #(#implementation_attributes)*
        impl<S> #oxidebot::runtime::GeneratedFeature<S> for #feature_ident
        where
            S: ::core::marker::Send + ::core::marker::Sync + 'static,
            #(#extractor_bounds_for_generated,)*
            #(#completer_bounds_for_generated,)*
        {
            fn into_feature(self) -> #oxidebot::runtime::Feature<S> {
                self.feature()
            }
        }

    })
}

fn arg_path_option(attributes: &[Attribute], name: &str) -> syn::Result<Option<Path>> {
    for attribute in attributes {
        let entries = attribute
            .parse_args_with(syn::punctuated::Punctuated::<Meta, Token![,]>::parse_terminated)?;
        for entry in entries {
            let Meta::NameValue(value) = entry else {
                continue;
            };
            if !value.path.is_ident(name) {
                continue;
            }
            let Expr::Path(path) = value.value else {
                return Err(syn::Error::new(
                    value.value.span(),
                    format!("#[arg({name} = ...)] expects a function path"),
                ));
            };
            return Ok(Some(path.path));
        }
    }
    Ok(None)
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
    match expand_bot_command(parse_macro_input!(input as DeriveInput)) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.into_compile_error().into(),
    }
}

/// Derives typed command argument parsing for a struct.
#[proc_macro_derive(CommandArgs, attributes(arg, command))]
pub fn derive_command_args(input: TokenStream) -> TokenStream {
    match expand_command_args(parse_macro_input!(input as DeriveInput)) {
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

fn expand_command_enum(input: DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
    let oxidebot = oxidebot_crate();
    if !input.generics.params.is_empty() {
        return Err(syn::Error::new(
            input.generics.span(),
            "CommandEnum does not support generic enums",
        ));
    }
    let name = input.ident;
    let Data::Enum(data) = input.data else {
        return Err(syn::Error::new(
            name.span(),
            "CommandEnum can only be derived for an enum",
        ));
    };
    let mut choice_builders = Vec::new();
    let mut matches = Vec::new();
    let mut accepted = Vec::new();
    for variant in data.variants {
        if !matches!(variant.fields, Fields::Unit) {
            return Err(syn::Error::new(
                variant.fields.span(),
                "CommandEnum variants must be unit variants",
            ));
        }
        let variant_name = variant.ident;
        let mut value = ident_to_command_name(&variant_name.to_string());
        let mut display = value.clone();
        let mut aliases = Vec::new();
        for attribute in variant
            .attrs
            .iter()
            .filter(|attribute| attribute.path().is_ident("choice"))
        {
            attribute.parse_nested_meta(|meta| {
                if meta.path.is_ident("value") {
                    value = meta.value()?.parse::<LitStr>()?.value();
                } else if meta.path.is_ident("name") {
                    display = meta.value()?.parse::<LitStr>()?.value();
                } else if meta.path.is_ident("alias") {
                    aliases.push(meta.value()?.parse::<LitStr>()?.value());
                } else {
                    return Err(meta.error("unknown #[choice(...)] option"));
                }
                Ok(())
            })?;
        }
        let mut builder = quote! {
            #oxidebot::runtime::ArgumentChoice::new(#display, #value)
        };
        for alias in &aliases {
            builder = quote! { #builder.alias(#alias) };
        }
        choice_builders.push(builder);
        let mut patterns = vec![quote! { #value }];
        patterns.extend(aliases.iter().map(|alias| quote! { #alias }));
        matches.push(quote! { #(#patterns)|* => ::core::result::Result::Ok(Self::#variant_name), });
        accepted.push(value);
    }
    Ok(quote! {
        impl #oxidebot::runtime::CommandEnum for #name {
            fn choices() -> ::std::vec::Vec<#oxidebot::runtime::ArgumentChoice> {
                ::std::vec![#(#choice_builders),*]
            }
        }

        impl #oxidebot::runtime::FromCommandValue for #name {
            fn from_command_value(
                value: #oxidebot::runtime::CommandValue,
            ) -> ::core::result::Result<Self, #oxidebot::runtime::CommandParseError> {
                let value = value.as_text().map(|value| value.to_owned()).ok_or(
                    #oxidebot::runtime::CommandParseError::UnexpectedValue {
                        expected: "text choice",
                        actual: "non-text command value",
                    },
                )?;
                match value.as_str() {
                    #(#matches)*
                    _ => ::core::result::Result::Err(
                        #oxidebot::runtime::CommandParseError::InvalidChoice {
                            argument: ::std::sync::Arc::from("value"),
                            choices: ::std::sync::Arc::from([#(#accepted),*].join(", ")),
                        },
                    ),
                }
            }
        }
    })
}

/// Derives efficient `FromState` projections for an application state type.
#[proc_macro_derive(BotState, attributes(state))]
pub fn derive_bot_state(input: TokenStream) -> TokenStream {
    match expand_bot_state(parse_macro_input!(input as DeriveInput)) {
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
    match expand_dialogue_form(parse_macro_input!(input as DeriveInput)) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.into_compile_error().into(),
    }
}

#[derive(Default)]
struct DialogueFieldOptions {
    prompt: Option<String>,
    error: Option<String>,
    confirm: bool,
    attempts: Option<usize>,
    choices: Vec<(String, String)>,
    validate: Option<Path>,
}

impl DialogueFieldOptions {
    fn parse(field: &Field) -> syn::Result<Self> {
        let mut output = Self::default();
        for attribute in field
            .attrs
            .iter()
            .filter(|attribute| attribute.path().is_ident("dialogue"))
        {
            attribute.parse_nested_meta(|meta| {
                if meta.path.is_ident("prompt") {
                    output.prompt = Some(meta.value()?.parse::<LitStr>()?.value());
                } else if meta.path.is_ident("error") {
                    output.error = Some(meta.value()?.parse::<LitStr>()?.value());
                } else if meta.path.is_ident("confirm") {
                    output.confirm = if meta.input.peek(Token![=]) {
                        meta.value()?.parse::<syn::LitBool>()?.value()
                    } else {
                        true
                    };
                } else if meta.path.is_ident("attempts") {
                    output.attempts = Some(meta.value()?.parse::<LitInt>()?.base10_parse()?);
                } else if meta.path.is_ident("retry") {
                    let retries: usize = meta.value()?.parse::<LitInt>()?.base10_parse()?;
                    output.attempts = Some(retries.saturating_add(1));
                } else if meta.path.is_ident("choice") {
                    let value = meta.value()?.parse::<LitStr>()?.value();
                    let Some((label, stored)) = value.split_once('=') else {
                        return Err(meta.error("dialogue choice must use `Label=value`"));
                    };
                    let label = label.trim();
                    let stored = stored.trim();
                    if label.is_empty() || stored.is_empty() {
                        return Err(meta.error("dialogue choice label and value must be non-empty"));
                    }
                    output.choices.push((label.to_owned(), stored.to_owned()));
                } else if meta.path.is_ident("validate") {
                    output.validate = Some(meta.value()?.parse::<Path>()?);
                } else {
                    return Err(meta.error("unknown #[dialogue(...)] option"));
                }
                Ok(())
            })?;
        }
        Ok(output)
    }
}

fn expand_dialogue_form(input: DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
    let oxidebot = oxidebot_crate();
    if !input.generics.params.is_empty() {
        return Err(syn::Error::new(
            input.generics.span(),
            "DialogueForm does not support generic structs",
        ));
    }
    let name = input.ident;
    let Data::Struct(data) = input.data else {
        return Err(syn::Error::new(
            name.span(),
            "DialogueForm can only be derived for a struct",
        ));
    };
    let Fields::Named(fields) = data.fields else {
        return Err(syn::Error::new(
            name.span(),
            "DialogueForm requires named fields",
        ));
    };

    let mut initializers = Vec::new();
    for field in fields.named {
        let ident = field
            .ident
            .clone()
            .ok_or_else(|| syn::Error::new(field.span(), "expected a named field"))?;
        let ty = field.ty.clone();
        let options = DialogueFieldOptions::parse(&field)?;
        let prompt = options
            .prompt
            .or_else(|| doc_string(&field.attrs))
            .unwrap_or_else(|| format!("请输入 {}：", ident.to_string().trim_start_matches("r#")));
        let attempts = options.attempts.unwrap_or(3).max(1);

        let expression = if options.confirm {
            if !is_type(&ty, "bool") {
                return Err(syn::Error::new(
                    ty.span(),
                    "#[dialogue(confirm)] requires a bool field",
                ));
            }
            if !options.choices.is_empty() || options.validate.is_some() {
                return Err(syn::Error::new(
                    field.span(),
                    "confirmation fields cannot also declare choices or a validator",
                ));
            }
            let words = options.error.map_or_else(
                || quote! { #oxidebot::runtime::ConfirmationWords::default() },
                |error| {
                    quote! {
                        #oxidebot::runtime::ConfirmationWords::default().retry_message(#error)
                    }
                },
            );
            quote! {
                dialogue
                    .confirm_with(#prompt, #words, #attempts)
                    .await?
            }
        } else if !options.choices.is_empty() {
            let labels = options.choices.iter().map(|(label, _)| label);
            let values = options.choices.iter().map(|(_, value)| value);
            let error = options
                .error
                .unwrap_or_else(|| "未知选项，请输入编号、选项名称或使用按钮。".to_owned());
            let selected = quote! {
                dialogue
                    .choose_with_error(
                        #prompt,
                        [
                            #(
                                (
                                    #labels,
                                    <#ty as ::core::str::FromStr>::from_str(#values)
                                        .map_err(|_| #oxidebot::runtime::HandlerError::internal(
                                            format!("invalid static dialogue choice for {}", stringify!(#ident)),
                                        ))?,
                                )
                            ),*
                        ],
                        #attempts,
                        #error,
                    )
                    .await?
            };
            if let Some(validate) = options.validate {
                quote! {{
                    let value: #ty = #selected;
                    #validate(&value).map_err(|error| #oxidebot::runtime::HandlerError::user(error.to_string()))?;
                    value
                }}
            } else {
                selected
            }
        } else {
            let error = options
                .error
                .map(|value| quote! { question = question.error(#value); });
            let validate = options
                .validate
                .map(|path| quote! { question = question.try_validate(|value| #path(value)); });
            quote! {{
                let mut question = dialogue.question::<#ty>(#prompt).attempts(#attempts);
                #error
                #validate
                question.await?
            }}
        };
        initializers.push(quote! { #ident: #expression });
    }

    Ok(quote! {
        impl #oxidebot::runtime::DialogueForm for #name {
            fn collect(dialogue: #oxidebot::runtime::Dialogue) -> #oxidebot::runtime::DialogueFormFuture<Self> {
                ::std::boxed::Box::pin(async move {
                    ::core::result::Result::Ok(Self {
                        #(#initializers,)*
                    })
                })
            }
        }
    })
}

fn expand_bot_state(input: DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
    let oxidebot = oxidebot_crate();
    if !input.generics.params.is_empty() {
        return Err(syn::Error::new(
            input.generics.span(),
            "BotState does not support generic root states",
        ));
    }
    let name = input.ident;
    let Data::Struct(data) = input.data else {
        return Err(syn::Error::new(
            name.span(),
            "BotState can only be derived for a struct",
        ));
    };
    let Fields::Named(fields) = data.fields else {
        return Err(syn::Error::new(
            name.span(),
            "BotState requires named fields",
        ));
    };
    let mut implementations = Vec::new();
    let mut selected_types = ::std::collections::HashSet::new();
    for field in fields.named {
        let selected = field
            .attrs
            .iter()
            .any(|attribute| attribute.path().is_ident("state"));
        if !selected {
            continue;
        }
        let field_span = field.span();
        let ident = field
            .ident
            .ok_or_else(|| syn::Error::new(field_span, "expected a named field"))?;
        let field_ty = field.ty;
        let (selected_ty, body, selected_bound) = if let Some(inner) =
            type_argument(&field_ty, "Arc")
        {
            (
                inner.clone(),
                quote! { ::std::sync::Arc::clone(&root.#ident) },
                quote! { #inner: ::core::marker::Send + ::core::marker::Sync + 'static },
            )
        } else {
            (
                field_ty.clone(),
                quote! { ::std::sync::Arc::new(root.#ident.clone()) },
                quote! { #field_ty: ::core::clone::Clone + ::core::marker::Send + ::core::marker::Sync + 'static },
            )
        };
        if is_type(&selected_ty, &name.to_string()) {
            return Err(syn::Error::new(
                selected_ty.span(),
                "BotState cannot expose the root state as a #[state] field because State<Root> is already available",
            ));
        }
        let key = selected_ty.to_token_stream().to_string();
        if !selected_types.insert(key) {
            return Err(syn::Error::new(
                selected_ty.span(),
                "BotState cannot expose two #[state] fields with the same selected type",
            ));
        }
        implementations.push(quote! {
            impl #oxidebot::runtime::FromState<#name> for #selected_ty
            where
                #name: ::core::marker::Send + ::core::marker::Sync + 'static,
                #selected_bound,
            {
                fn from_state(
                    root: &::std::sync::Arc<#name>,
                ) -> ::std::sync::Arc<Self> {
                    #body
                }
            }
        });
    }
    if implementations.is_empty() {
        return Err(syn::Error::new(
            name.span(),
            "BotState needs at least one field annotated with #[state]",
        ));
    }
    Ok(quote! { #(#implementations)* })
}

fn expand_command_args(input: DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
    let oxidebot = oxidebot_crate();
    let visibility = input.vis.clone();
    let name = input.ident;
    let field_module = format_ident!("{}_fields", ident_to_module_name(&name.to_string()));
    let field_visibility: syn::Visibility = if matches!(visibility, syn::Visibility::Inherited) {
        syn::parse_quote!(pub(super))
    } else {
        syn::parse_quote!(pub)
    };
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
    let mut grouped_fields = Vec::<(String, String)>::new();
    let mut initializers = Vec::new();
    let mut field_markers = Vec::new();
    let mut completer_bindings = Vec::new();
    let mut completer_providers = Vec::<Path>::new();
    let mut inferred_bounds = Vec::<syn::WherePredicate>::new();

    for field in fields.named {
        let options = FieldOptions::parse(&field)?;
        let ident = field
            .ident
            .clone()
            .ok_or_else(|| syn::Error::new(field.span(), "expected a named field"))?;
        let rust_name = ident.to_string().trim_start_matches("r#").to_owned();
        let argument_name = options.name.clone().unwrap_or_else(|| rust_name.clone());
        for group in &options.groups {
            grouped_fields.push((group.clone(), argument_name.clone()));
        }
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
        let value_enum = options.value_enum;
        let flag = options.flag
            || action_is_count
            || action_is_set_true
            || action_is_set_false
            || (is_bool && (options.long.is_some() || options.short.is_some()));
        let multiple = options.multiple || is_vec || action_is_append;

        if options.skip {
            let has_conflicting_option = options.name.is_some()
                || options.help.is_some()
                || options.heading.is_some()
                || options.prompt.is_some()
                || options.value_name.is_some()
                || options.long.is_some()
                || options.short.is_some()
                || options.default.is_some()
                || options.required.is_some()
                || options.multiple
                || options.rest
                || options.flag
                || options.hidden
                || options.kind.is_some()
                || options.action.is_some()
                || !options.choices.is_empty()
                || options.autocomplete
                || options.complete.is_some()
                || options.min_value.is_some()
                || options.max_value.is_some()
                || options.min_length.is_some()
                || options.max_length.is_some()
                || !options.requires.is_empty()
                || !options.conflicts.is_empty()
                || !options.groups.is_empty()
                || options.validate.is_some();
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
        let marker_ident = format_ident!("{}", to_pascal_case(rust_name.trim_start_matches("r#")));
        field_markers.push(quote! {
            #[derive(Clone, Copy, Debug, Default)]
            #field_visibility struct #marker_ident;
            impl #oxidebot::runtime::CommandFieldTag for #marker_ident {
                const NAME: &'static str = #argument_name;
            }
        });
        if let Some(provider) = options.complete.clone() {
            completer_providers.push(provider.clone());
            completer_bindings.push(quote! {
                feature = feature.complete(
                    #field_module::#marker_ident,
                    #provider,
                );
            });
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
        if value_enum && !options.choices.is_empty() {
            return Err(syn::Error::new(
                field.span(),
                "#[arg(value_enum)] cannot be combined with explicit #[arg(choice = ...)] values",
            ));
        }
        if options.validate.is_some() && (is_option || is_vec || flag) {
            return Err(syn::Error::new(
                field.span(),
                "#[arg(validate = ...)] currently supports one non-flag field; validate Option<T> or Vec<T> in the handler",
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
                #ty: #oxidebot::runtime::FromCommandValue
            )),
            FieldKind::Option(ty) | FieldKind::Vec(ty) => {
                inferred_bounds.push(syn::parse_quote!(
                    #ty: #oxidebot::runtime::FromCommandValue
                ));
            }
            FieldKind::Plain(_) => {}
        }
        if value_enum {
            inferred_bounds.push(syn::parse_quote!(
                #value_ty: #oxidebot::runtime::CommandEnum
            ));
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
            #oxidebot::runtime::ArgumentSpec::new(#argument_name)
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
        if let Some(heading) = &options.heading {
            schema = quote! { #schema.heading(#heading) };
        }
        if options.hidden {
            schema = quote! { #schema.hidden() };
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
                #schema.choice(#oxidebot::runtime::ArgumentChoice::new(#choice, #choice))
            };
        }
        if value_enum {
            schema = quote! { #schema.with_choices(<#value_ty as #oxidebot::runtime::CommandEnum>::choices()) };
        }
        if let Some(validator) = &options.validate {
            schema = quote! { #schema.validate::<#value_ty, _, _>(#validator) };
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

    let mut schema_groups = Vec::new();
    let mut configured_group_names = ::std::collections::HashSet::new();
    for group in &command_options.groups {
        if group.name.trim().is_empty() || !configured_group_names.insert(group.name.clone()) {
            return Err(syn::Error::new(
                name.span(),
                "command argument groups need unique, non-empty names",
            ));
        }
        let members = grouped_fields
            .iter()
            .filter(|(name, _)| name == &group.name)
            .map(|(_, field)| field)
            .collect::<Vec<_>>();
        if members.is_empty() {
            return Err(syn::Error::new(
                name.span(),
                format!(
                    "command group `{}` does not contain any #[arg(group = ...)] fields",
                    group.name
                ),
            ));
        }
        let group_name = &group.name;
        let required = group.required;
        let multiple = group.multiple;
        schema_groups.push(quote! {
            schema = schema.group(
                #oxidebot::runtime::ArgumentGroup::new(#group_name)
                    .arguments([#(#members),*])
                    .required(#required)
                    .multiple(#multiple),
            );
        });
    }
    for (group, _) in &grouped_fields {
        if !configured_group_names.contains(group) {
            return Err(syn::Error::new(
                name.span(),
                format!("field group `{group}` needs #[command(group(name = \"{group}\", ...))]",),
            ));
        }
    }

    let command_builder = command_options.builder_tokens();
    let mut generics = input.generics;
    let predicates = &mut generics.make_where_clause().predicates;
    predicates.extend(inferred_bounds);
    predicates.push(syn::parse_quote!(Self: ::core::marker::Send + 'static));
    let (impl_generics, type_generics, where_clause) = generics.split_for_impl();
    let completer_bounds = completer_providers
        .iter()
        .map(|provider| quote! { #provider: #oxidebot::runtime::DynamicCompleter<S> })
        .collect::<Vec<_>>();

    Ok(quote! {
        #visibility mod #field_module {
            #(#field_markers)*
        }

        impl #impl_generics #oxidebot::runtime::CommandArgs for #name #type_generics #where_clause {
            fn schema() -> #oxidebot::runtime::CommandSchema {
                let mut schema = #oxidebot::runtime::CommandSchema::new();
                #(#schema_fields)*
                #(#schema_groups)*
                schema
            }

            fn from_arguments(
                arguments: &#oxidebot::runtime::ParsedArguments,
            ) -> ::core::result::Result<Self, #oxidebot::runtime::CommandParseError> {
                ::core::result::Result::Ok(Self {
                    #(#initializers,)*
                })
            }

            fn command(
                name: impl ::core::convert::Into<::std::sync::Arc<str>>,
            ) -> #oxidebot::runtime::Command {
                let command = #oxidebot::runtime::Command::new(name).schema(Self::schema());
                #command_builder
            }
        }

        impl #impl_generics #name #type_generics #where_clause {
            /// Defines this command and binds all field-local completers.
            #[must_use]
            #visibility fn feature<S, H, T>(
                name: impl ::core::convert::Into<::std::sync::Arc<str>>,
                handler: H,
            ) -> #oxidebot::runtime::Feature<S>
            where
                S: ::core::marker::Send + ::core::marker::Sync + 'static,
                H: #oxidebot::runtime::IntoHandler<T, S>,
                #(#completer_bounds,)*
            {
                let mut feature = #oxidebot::runtime::Feature::command(
                    <Self as #oxidebot::runtime::CommandArgs>::command(name),
                    handler,
                );
                #(#completer_bindings)*
                feature
            }
        }
    })
}

fn expand_bot_command(input: DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
    let oxidebot = oxidebot_crate();
    if !input.generics.params.is_empty() {
        return Err(syn::Error::new(
            input.generics.span(),
            "BotCommand does not support generic enums; wrap generic values in a concrete CommandArgs type",
        ));
    }
    let visibility = input.vis.clone();
    let branch_visibility: syn::Visibility = if matches!(visibility, syn::Visibility::Inherited) {
        syn::parse_quote!(pub(super))
    } else {
        syn::parse_quote!(pub)
    };
    let enum_name = input.ident;
    let branch_module = format_ident!("{}_branches", ident_to_module_name(&enum_name.to_string()));
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
    let global_args = root_options.global_args.clone();
    let root_builder = root_options.builder_tokens();
    let global_builder = global_args.as_ref().map(|global_args| {
        quote! {
            command = command.global_args::<#global_args>();
        }
    });
    let mut branch_builders = Vec::new();
    let mut match_arms = Vec::new();
    let mut nested_match_arms = Vec::new();
    let mut inferred_bounds = Vec::<syn::WherePredicate>::new();
    if let Some(global_args) = &global_args {
        inferred_bounds.push(syn::parse_quote!(#global_args: #oxidebot::runtime::CommandArgs));
    }
    let mut branch_markers = Vec::new();

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
                branch_markers.push(quote! {
                    #[derive(Clone, Copy, Debug, Default)]
                    #branch_visibility struct #ident;
                    impl #oxidebot::runtime::CommandBranchTag for #ident {
                        type Command = super::#enum_name;
                        type Arguments = #oxidebot::runtime::UnitBranch;
                        const PATH: &'static [&'static str] = &[#branch_name];
                    }
                });
                let mut builder = quote! {
                    let mut branch = #oxidebot::runtime::CommandBranch::new(#branch_name);
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
                let branch_ty = match ty {
                    Type::Path(path)
                        if path.qself.is_none()
                            && path.path.leading_colon.is_none()
                            && path.path.segments.len() == 1 =>
                    {
                        quote!(super::#ty)
                    }
                    _ => quote!(#ty),
                };
                if options.subcommand {
                    inferred_bounds.push(syn::parse_quote!(#ty: #oxidebot::runtime::CommandTree));
                    branch_markers.push(quote! {
                        #[derive(Clone, Copy, Debug, Default)]
                        #branch_visibility struct #ident;
                        impl #oxidebot::runtime::CommandBranchTag for #ident {
                            type Command = super::#enum_name;
                            type Arguments = #branch_ty;
                            const PATH: &'static [&'static str] = &[#branch_name];
                            const MATCH_DESCENDANTS: bool = true;
                            const STRIP_PREFIX: usize = 1;
                        }
                    });
                    let mut builder = quote! {
                        let nested = <#ty as #oxidebot::runtime::CommandTree>::command();
                        let mut branch = #oxidebot::runtime::CommandBranch::new(#branch_name);
                        if let ::core::option::Option::Some(schema) = nested.schema_ref() {
                            branch = branch.schema(schema.clone());
                        }
                        for child in nested.branches().iter().cloned() {
                            branch = branch.subcommand(child);
                        }
                    };
                    if let Some(description) = description {
                        builder.extend(quote! { branch = branch.description(#description); });
                    }
                    for alias in aliases {
                        builder.extend(quote! { branch = branch.alias(#alias); });
                    }
                    if hidden {
                        builder.extend(quote! { branch = branch.hidden(); });
                    }
                    builder.extend(quote! { command = command.subcommand(branch); });
                    branch_builders.push(builder);
                    nested_match_arms.push(quote! {
                        if __oxidebot_selected_branch
                            .is_some_and(|value| value.eq_ignore_ascii_case(#branch_name))
                        {
                            let nested = result.descend(1);
                            return ::core::result::Result::Ok(Self::#ident(
                                <#ty as #oxidebot::runtime::FromCommandMatch>::from_match(&nested)?,
                            ));
                        }
                    });
                } else {
                    inferred_bounds.push(syn::parse_quote!(#ty: #oxidebot::runtime::CommandArgs));
                    branch_markers.push(quote! {
                        #[derive(Clone, Copy, Debug, Default)]
                        #branch_visibility struct #ident;
                        impl #oxidebot::runtime::CommandBranchTag for #ident {
                            type Command = super::#enum_name;
                            type Arguments = #branch_ty;
                            const PATH: &'static [&'static str] = &[#branch_name];
                        }
                    });
                    let mut builder = quote! {
                        let mut branch = #oxidebot::runtime::CommandBranch::new(#branch_name)
                            .schema(<#ty as #oxidebot::runtime::CommandArgs>::schema());
                    };
                    if let Some(description) = description {
                        builder.extend(quote! { branch = branch.description(#description); });
                    }
                    for alias in aliases {
                        builder.extend(quote! { branch = branch.alias(#alias); });
                    }
                    if hidden {
                        builder.extend(quote! { branch = branch.hidden(); });
                    }
                    builder.extend(quote! { command = command.subcommand(branch); });
                    branch_builders.push(builder);
                    match_arms.push(quote! {
                        ::core::option::Option::Some(#branch_name) => {
                            let arguments = if let ::core::option::Option::Some(arguments) = result.arguments() {
                                arguments.clone()
                            } else {
                                result.parse_active()?
                            };
                            ::core::result::Result::Ok(Self::#ident(
                                <#ty as #oxidebot::runtime::CommandArgs>::from_arguments(&arguments)?,
                            ))
                        },
                    });
                }
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
        #visibility mod #branch_module {
            #(#branch_markers)*
        }

        impl #impl_generics #oxidebot::runtime::FromCommandMatch for #enum_name #type_generics #where_clause {
            fn from_match(
                result: &#oxidebot::runtime::CommandMatch,
            ) -> ::core::result::Result<Self, #oxidebot::runtime::CommandParseError> {
                let __oxidebot_command = <Self as #oxidebot::runtime::CommandTree>::command();
                let __oxidebot_selected_branch = result.branch_names().iter().find_map(|actual| {
                    __oxidebot_command.branches().iter().find_map(|branch| {
                        (branch.name().eq_ignore_ascii_case(actual)
                            || branch
                                .aliases_list()
                                .iter()
                                .any(|alias| alias.eq_ignore_ascii_case(actual)))
                        .then_some(branch.name())
                    })
                });
                #(#nested_match_arms)*
                match __oxidebot_selected_branch {
                    #(#match_arms)*
                    ::core::option::Option::Some(_) => ::core::result::Result::Err(
                        #oxidebot::runtime::CommandParseError::InvalidValue {
                            value: __oxidebot_selected_branch.unwrap_or_default().to_owned(),
                            expected: "known subcommand",
                            reason: "the selected command branch is not represented by this enum".to_owned(),
                        },
                    ),
                    ::core::option::Option::None => ::core::result::Result::Err(
                        #oxidebot::runtime::CommandParseError::MissingSubcommand {
                            choices: ::std::sync::Arc::from(""),
                        },
                    ),
                }
            }
        }

        impl #impl_generics #oxidebot::runtime::CommandTree for #enum_name #type_generics #where_clause {
            fn command() -> #oxidebot::runtime::Command {
                let command = #oxidebot::runtime::Command::new(#root_name);
                let mut command = #root_builder;
                #global_builder
                #(#branch_builders)*
                command
            }
        }
    })
}

fn ident_to_module_name(value: &str) -> String {
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

fn argument_action_tokens(
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
