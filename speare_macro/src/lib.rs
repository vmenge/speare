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

fn p(input: ItemImpl) -> Result<TokenStream, &'static str> {
    let self_type = &input.self_ty;

    let mut additional_impls = Vec::new();

    for impl_item in &input.items {
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
                let _self_arg = args_iter.next().ok_or("Expected 'self' as first arg")?;
                let msg_arg = args_iter.next().ok_or("Expected a message argument")?;

                let msg_type = if let FnArg::Typed(arg) = msg_arg {
                    &arg.ty
                } else {
                    return Err("Expected a typed argument for the message");
                };

                let t = match output {
                    ReturnType::Type(_, ref t) => t,
                    _ => return Err("Expected Result type for return value"),
                };

                let type_path = match t.as_ref() {
                    Type::Path(x) => x,
                    _ => return Err("Invalid return type"),
                };

                let path = type_path
                    .path
                    .segments
                    .last()
                    .ok_or("Invalid return type")?;

                if path.ident != "Result" {
                    return Err("Expected Result type in handler return value");
                }

                let angle_args = match &path.arguments {
                    syn::PathArguments::AngleBracketed(angle) => &angle.args,
                    _ => return Err("Result return type must have its generics declared."),
                };

                let mut arg_iter = angle_args.iter();
                let ok_type = arg_iter.next().ok_or("Expected Ok type")?;
                let err_type = arg_iter.next().ok_or("Expected Err type")?;

                let (ok_type, err_type) = (quote!(#ok_type), quote!(#err_type));

                additional_impls.push(quote! {
                    #[async_trait]
                    impl Handler<#msg_type> for #self_type {
                        type Reply = #ok_type;
                        type Error = #err_type;

                        async fn handle(&mut self, msg: #msg_type, ctx: &Ctx<Self>) -> Result<Self::Reply, Self::Error> {
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
