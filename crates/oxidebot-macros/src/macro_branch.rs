//! Expansion and attribute parsing for typed command-tree branch handlers.

use super::*;

pub(crate) struct BranchAttribute {
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
pub(crate) fn expand_branch_function(
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
