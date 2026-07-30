//! Expansion for strongly typed command-argument schemas.
//!
//! The derive entrypoint stays at crate root; this module emits schema,
//! extraction, validation, and field-local completion bindings.

use super::*;

pub(crate) fn expand_command_args(input: DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
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
