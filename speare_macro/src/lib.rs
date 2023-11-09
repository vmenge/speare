extern crate proc_macro;

use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, FnArg, ImplItem, ItemImpl, ReturnType, Type};

#[proc_macro_attribute]
pub fn handler(_attr: TokenStream, item: TokenStream) -> TokenStream {
    item
}

#[proc_macro_attribute]
pub fn process(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as ItemImpl);

    match p(input.clone()) {
        Ok(v) => v,
        Err(e) => {
            let error = syn::Error::new_spanned(input, e);
            error.to_compile_error().into()
        }
    }
}

fn p(mut input: ItemImpl) -> Result<TokenStream, &'static str> {
    let (impl_generics, _, where_clause) = input.generics.split_for_impl();

    let self_type = &input.self_ty;

    let mut additional_impls = Vec::new();

    for impl_item in &mut input.items {
        if let ImplItem::Fn(method) = impl_item {
            let has_handler_attr = method
                .attrs
                .iter()
                .any(|attr| attr.path().is_ident("handler"));

            if has_handler_attr {
                let fn_name = &method.sig.ident;
                let inputs = &method.sig.inputs;
                let output = &method.sig.output;

                let mut args_iter = inputs.iter();
                args_iter.next().ok_or("Expected 'self' as first arg")?;
                let msg_arg = args_iter.next().ok_or("Expected a message argument")?;

                let msg_type = if let FnArg::Typed(arg) = msg_arg {
                    &(*arg.ty)
                } else {
                    return Err("Expected a typed argument for the message");
                };

                let output_type = match output {
                    ReturnType::Type(_, type_) => type_,
                    _ => return Err("Expected Result type for return value"),
                };

                let segment = match output_type.as_ref() {
                    Type::Path(type_path) => type_path
                        .path
                        .segments
                        .last()
                        .ok_or("Expected Result type for return value")?,
                    _ => return Err("Expected Result type for return value"),
                };

                if segment.ident != "Reply" {
                    return Err("Expected Reply type in handler return value");
                }

                let angle_args = match &segment.arguments {
                    syn::PathArguments::AngleBracketed(angle) => angle,
                    _ => return Err("Result return type must have its generics declared."),
                };

                let args = &angle_args.args;

                if args.len() != 2 {
                    return Err("Expected two generics for Reply type, Ok type and Err type");
                }

                let ok_type = &args[0];
                let err_type = &args[1];

                additional_impls.push(quote! {
                    #[async_trait]
                    impl #impl_generics Handler<#msg_type> for #self_type #where_clause {
                        type Ok = #ok_type;
                        type Err = #err_type;

                        async fn handle(&mut self, msg: #msg_type, ctx: &Ctx<Self>) -> Reply<Self::Ok, Self::Err> {
                            self.#fn_name(msg, ctx).await
                        }
                    }
                });
            }
        }
    }

    let expanded = quote! {
        #input
        #(#additional_impls)*
    };

    Ok(TokenStream::from(expanded))
}
