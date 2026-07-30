//! Expansion for enum-backed command trees.
//!
//! This derives branch schemas and typed branch matching while the proc-macro
//! entrypoint remains in the crate root.

use super::*;

pub(crate) fn expand_bot_command(input: DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
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
