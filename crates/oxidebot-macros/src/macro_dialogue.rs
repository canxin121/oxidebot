//! Expansion logic for dialogue forms and typed root-state projection.
//!
//! Proc-macro derive entrypoints remain at crate root; this module owns their
//! shared option parsing and generated implementation details.

use super::*;

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

pub(crate) fn expand_dialogue_form(input: DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
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

pub(crate) fn expand_bot_state(input: DeriveInput) -> syn::Result<proc_macro2::TokenStream> {
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
