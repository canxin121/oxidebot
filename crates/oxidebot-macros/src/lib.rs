use proc_macro::TokenStream;
use quote::{format_ident, quote, ToTokens};
use syn::{
    parse_macro_input, spanned::Spanned, Attribute, Data, DeriveInput, Expr, Field, Fields, FnArg,
    GenericArgument, ItemFn, LitChar, LitStr, Pat, Path, PathArguments, Type,
};

#[proc_macro_attribute]
pub fn command(attribute: TokenStream, input: TokenStream) -> TokenStream {
    match expand_command_function(attribute, parse_macro_input!(input as ItemFn)) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.into_compile_error().into(),
    }
}

/// Binds an ordinary async function to one statically generated command-tree branch.
///
/// The first function argument is the branch's typed argument value; all
/// remaining arguments use OxideBot's normal extractor system. The generated
/// unit value can be installed directly with `Module::add(handler_name)`.
#[proc_macro_attribute]
pub fn branch(attribute: TokenStream, input: TokenStream) -> TokenStream {
    match expand_branch_function(attribute, parse_macro_input!(input as ItemFn)) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.into_compile_error().into(),
    }
}

fn expand_branch_function(
    attribute: TokenStream,
    mut function: ItemFn,
) -> syn::Result<proc_macro2::TokenStream> {
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
    let marker: Path = syn::parse(attribute)?;
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
    function.attrs = implementation_attributes.clone();
    function.vis = syn::Visibility::Inherited;
    function.sig.ident = implementation_ident.clone();

    let mut inputs = function.sig.inputs.iter();
    let Some(FnArg::Typed(first)) = inputs.next() else {
        return Err(syn::Error::new(
            function.sig.span(),
            "#[oxidebot::branch] requires the branch arguments as its first parameter",
        ));
    };
    let Pat::Ident(first_pattern) = first.pat.as_ref() else {
        return Err(syn::Error::new(
            first.pat.span(),
            "the branch argument must use a simple identifier pattern",
        ));
    };
    let first_ident = first_pattern.ident.clone();
    let first_ty = first.ty.clone();
    let mut wrapper_inputs = Vec::new();
    let mut wrapper_types = Vec::new();
    let mut call_arguments = vec![quote! { __oxidebot_branch_args }];
    for argument in inputs {
        let FnArg::Typed(typed) = argument else {
            return Err(syn::Error::new(argument.span(), "methods are not supported"));
        };
        let Pat::Ident(pattern) = typed.pat.as_ref() else {
            return Err(syn::Error::new(
                typed.pat.span(),
                "branch handler parameters must use simple identifier patterns",
            ));
        };
        let ident = pattern.ident.clone();
        let ty = typed.ty.clone();
        wrapper_inputs.push(quote! { #typed });
        wrapper_types.push(quote! { #ty });
        call_arguments.push(quote! { #ident });
    }
    let extractor_bounds = wrapper_types.iter().map(|ty| {
        quote! { #ty: ::oxidebot::Extract<S> + ::core::marker::Send + 'static }
    });
    let extractor_bounds_install = wrapper_types.iter().map(|ty| {
        quote! { #ty: ::oxidebot::Extract<S> + ::core::marker::Send + 'static }
    });

    // Rename the first argument in the hidden implementation so its original
    // type and documentation remain intact while the wrapper extracts through
    // the branch marker.
    if let Some(FnArg::Typed(first_mut)) = function.sig.inputs.first_mut() {
        first_mut.pat = Box::new(syn::parse_quote!(#first_ident));
    }

    Ok(quote! {
        #[doc(hidden)]
        #function

        #(#implementation_attributes)*
        #[doc(hidden)]
        async fn #handler_ident(
            ::oxidebot::BranchArgs(__oxidebot_branch_args): ::oxidebot::BranchArgs<#marker>,
            #(#wrapper_inputs),*
        ) #return_type {
            #implementation_ident(#(#call_arguments),*).await
        }

        #(#outer_attributes)*
        #[allow(non_camel_case_types)]
        #[derive(Clone, Copy, Debug, Default)]
        #visibility struct #feature_ident;

        #(#implementation_attributes)*
        impl #feature_ident {
            #[must_use]
            #visibility fn feature<S>(self) -> ::oxidebot::Feature<S>
            where
                S: ::core::marker::Send + ::core::marker::Sync + 'static,
                #marker: ::oxidebot::CommandBranchTag<Arguments = #first_ty>,
                #(#extractor_bounds,)*
            {
                ::oxidebot::Feature::command_branch(#marker, #handler_ident)
            }
        }

        #(#implementation_attributes)*
        impl<S> ::oxidebot::IntoFeature<S> for #feature_ident
        where
            S: ::core::marker::Send + ::core::marker::Sync + 'static,
            #marker: ::oxidebot::CommandBranchTag<Arguments = #first_ty>,
            #(#extractor_bounds_install,)*
        {
            fn install(self, module: ::oxidebot::Module<S>) -> ::oxidebot::Module<S> {
                <::oxidebot::Feature<S> as ::oxidebot::IntoFeature<S>>::install(
                    self.feature(),
                    module,
                )
            }
        }
    })
}

fn expand_command_function(
    attribute: TokenStream,
    mut function: ItemFn,
) -> syn::Result<proc_macro2::TokenStream> {
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
    let spec_ident = syn::Ident::new(
        &format!("{}_command", feature_ident),
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
    function.attrs = implementation_attributes.clone();
    function.vis = syn::Visibility::Inherited;
    function.sig.ident = implementation_ident.clone();

    let mut command_fields = Vec::new();
    let mut wrapper_inputs = Vec::new();
    let mut wrapper_types = Vec::new();
    let mut call_arguments = Vec::new();

    for argument in function.sig.inputs.iter_mut() {
        let FnArg::Typed(typed) = argument else {
            return Err(syn::Error::new(
                argument.span(),
                "methods are not supported",
            ));
        };
        let Pat::Ident(pattern) = typed.pat.as_ref() else {
            return Err(syn::Error::new(
                typed.pat.span(),
                "command function parameters must use simple identifier patterns",
            ));
        };
        let ident = pattern.ident.clone();
        let ty = typed.ty.clone();
        let (arg_attributes, other_attributes): (Vec<_>, Vec<_>) = typed
            .attrs
            .drain(..)
            .partition(|attribute| attribute.path().is_ident("arg"));
        typed.attrs = other_attributes.clone();
        if arg_attributes.is_empty() {
            let mut wrapper = typed.clone();
            wrapper.attrs = other_attributes;
            wrapper_inputs.push(quote! { #wrapper });
            wrapper_types.push(quote! { #ty });
            call_arguments.push(quote! { #ident });
        } else {
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
            #[derive(::oxidebot::CommandArgs)]
            #visibility struct #args_ident {
                #(#command_fields,)*
            }
        }
    };
    let args_input = if command_fields.is_empty() {
        quote! {}
    } else {
        wrapper_types.insert(0, quote! { ::oxidebot::Args<#args_ident> });
        quote! { ::oxidebot::Args(__oxidebot_args): ::oxidebot::Args<#args_ident>, }
    };
    let command_builder = if command_fields.is_empty() {
        quote! { ::oxidebot::command(#command_name) }
    } else {
        quote! { ::oxidebot::command(#command_name).args::<#args_ident>() }
    };
    let extractor_bounds = wrapper_types.iter().map(|ty| {
        quote! { #ty: ::oxidebot::Extract<S> + ::core::marker::Send + 'static }
    });
    let extractor_bounds_for_install = wrapper_types.iter().map(|ty| {
        quote! { #ty: ::oxidebot::Extract<S> + ::core::marker::Send + 'static }
    });

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

        #(#outer_attributes)*
        #[allow(non_camel_case_types)]
        #[derive(Clone, Copy, Debug, Default)]
        #visibility struct #feature_ident;

        #(#implementation_attributes)*
        impl #feature_ident {
            #[must_use]
            #visibility fn command() -> ::oxidebot::Command {
                #command_builder
            }

            #[must_use]
            #visibility fn feature<S>(self) -> ::oxidebot::Feature<S>
            where
                S: ::core::marker::Send + ::core::marker::Sync + 'static,
                #(#extractor_bounds,)*
            {
                ::oxidebot::Feature::command(Self::command(), #handler_ident)
            }
        }

        #(#implementation_attributes)*
        impl<S> ::oxidebot::IntoFeature<S> for #feature_ident
        where
            S: ::core::marker::Send + ::core::marker::Sync + 'static,
            #(#extractor_bounds_for_install,)*
        {
            fn install(self, module: ::oxidebot::Module<S>) -> ::oxidebot::Module<S> {
                <::oxidebot::Feature<S> as ::oxidebot::IntoFeature<S>>::install(
                    self.feature(),
                    module,
                )
            }
        }

        #(#implementation_attributes)*
        impl<S> ::oxidebot::GeneratedFeature<S> for #feature_ident
        where
            S: ::core::marker::Send + ::core::marker::Sync + 'static,
            #(#extractor_bounds_for_install,)*
        {
            fn into_feature(self) -> ::oxidebot::Feature<S> {
                self.feature()
            }
        }

        #(#implementation_attributes)*
        #[deprecated(note = "use Module::add(the_command_feature) or the_command_feature.feature()")]
        #[must_use]
        #visibility fn #spec_ident() -> ::oxidebot::Command {
            #feature_ident::command()
        }
    })
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

#[proc_macro_derive(BotState, attributes(state))]
pub fn derive_bot_state(input: TokenStream) -> TokenStream {
    match expand_bot_state(parse_macro_input!(input as DeriveInput)) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.into_compile_error().into(),
    }
}

fn expand_bot_state(input: DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
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
        let selected = field.attrs.iter().any(|attribute| attribute.path().is_ident("state"));
        if !selected {
            continue;
        }
        let ident = field.ident.ok_or_else(|| syn::Error::new(field.span(), "expected a named field"))?;
        let field_ty = field.ty;
        let (selected_ty, body, selected_bound) = if let Some(inner) = type_argument(&field_ty, "Arc") {
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
        let key = selected_ty.to_token_stream().to_string();
        if !selected_types.insert(key) {
            return Err(syn::Error::new(
                selected_ty.span(),
                "BotState cannot expose two #[state] fields with the same selected type",
            ));
        }
        implementations.push(quote! {
            impl ::oxidebot::FromState<#name> for #selected_ty
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
    let visibility = input.vis.clone();
    let name = input.ident;
    let field_module = format_ident!(
        "{}_fields",
        ident_to_module_name(&name.to_string())
    );
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
    let mut initializers = Vec::new();
    let mut field_markers = Vec::new();
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
        let marker_ident = format_ident!(
            "{}",
            to_pascal_case(rust_name.trim_start_matches("r#"))
        );
        field_markers.push(quote! {
            #[derive(Clone, Copy, Debug, Default)]
            #field_visibility struct #marker_ident;
            impl ::oxidebot::CommandFieldTag for #marker_ident {
                const NAME: &'static str = #argument_name;
            }
        });
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
        #visibility mod #field_module {
            #(#field_markers)*
        }

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
    let root_builder = root_options.builder_tokens();
    let mut branch_builders = Vec::new();
    let mut match_arms = Vec::new();
    let mut nested_match_arms = Vec::new();
    let mut inferred_bounds = Vec::<syn::WherePredicate>::new();
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
                    impl ::oxidebot::CommandBranchTag for #ident {
                        type Command = super::#enum_name;
                        type Arguments = ::oxidebot::UnitBranch;
                        const PATH: &'static [&'static str] = &[#branch_name];
                    }
                });
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
                    inferred_bounds.push(syn::parse_quote!(#ty: ::oxidebot::CommandTree));
                    branch_markers.push(quote! {
                        #[derive(Clone, Copy, Debug, Default)]
                        #branch_visibility struct #ident;
                        impl ::oxidebot::CommandBranchTag for #ident {
                            type Command = super::#enum_name;
                            type Arguments = #branch_ty;
                            const PATH: &'static [&'static str] = &[#branch_name];
                            const MATCH_DESCENDANTS: bool = true;
                            const STRIP_PREFIX: usize = 1;
                        }
                    });
                    let mut builder = quote! {
                        let nested = <#ty as ::oxidebot::CommandTree>::command();
                        let mut branch = ::oxidebot::CommandBranch::new(#branch_name);
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
                                <#ty as ::oxidebot::FromCommandMatch>::from_match(&nested)?,
                            ));
                        }
                    });
                } else {
                    inferred_bounds.push(syn::parse_quote!(#ty: ::oxidebot::CommandArgs));
                    branch_markers.push(quote! {
                        #[derive(Clone, Copy, Debug, Default)]
                        #branch_visibility struct #ident;
                        impl ::oxidebot::CommandBranchTag for #ident {
                            type Command = super::#enum_name;
                            type Arguments = #branch_ty;
                            const PATH: &'static [&'static str] = &[#branch_name];
                        }
                    });
                    let mut builder = quote! {
                        let mut branch = ::oxidebot::CommandBranch::new(#branch_name)
                            .schema(<#ty as ::oxidebot::CommandArgs>::schema());
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
                                <#ty as ::oxidebot::CommandArgs>::from_arguments(&arguments)?,
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

        impl #impl_generics ::oxidebot::FromCommandMatch for #enum_name #type_generics #where_clause {
            fn from_match(
                result: &::oxidebot::CommandMatch,
            ) -> ::core::result::Result<Self, ::oxidebot::CommandParseError> {
                let __oxidebot_command = <Self as ::oxidebot::CommandTree>::command();
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
                        ::oxidebot::CommandParseError::InvalidValue {
                            value: __oxidebot_selected_branch.unwrap_or_default().to_owned(),
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
    subcommand: bool,
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
