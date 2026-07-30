//! Expansion for finite, strongly typed command choice enums.

use super::*;

pub(crate) fn expand_command_enum(input: DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
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
