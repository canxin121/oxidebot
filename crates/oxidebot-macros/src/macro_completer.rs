//! Expansion for the state-aware command completer attribute.

use super::*;

pub(crate) fn expand_completer_function(
    mut function: ItemFn,
) -> syn::Result<proc_macro2::TokenStream> {
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
