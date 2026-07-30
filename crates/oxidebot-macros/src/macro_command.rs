//! Expansion for the #[command] attribute macro.
//!
//! The crate root keeps the proc-macro entrypoint; this module transforms a
//! handler function into a generated feature and its typed argument schema.

use super::*;

pub(crate) fn expand_command_function(
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
