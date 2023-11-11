extern crate proc_macro;

use proc_macro::TokenStream;
use quote::quote;
use syn::{
    parse::{Parse, ParseStream},
    parse_macro_input, parse_quote, FnArg, Ident, ImplItem, ItemImpl, ReturnType, Token, Type,
};

/// Allows you to register subscriptions to any global publishes of a message.
///
/// ## Example
/// ```ignore
///struct Cat;
///
///#[process]
///impl Cat {
///    #[subscriptions]
///    async fn subs(&self, evt: &EventBus<Self>) {
///        evt.subscribe::<SayHi>().await;
///    }
///
///    #[handler]
///    async fn hi(&mut self, msg: SayHi) -> Reply<(), ()> {
///        println!("MEOW!");
///        reply(())
///    }
///}
///```
/// Learn more on [The Speare Book](https://vmenge.github.io/speare/pub_sub.html)
#[proc_macro_attribute]
pub fn subscriptions(_attr: TokenStream, item: TokenStream) -> TokenStream {
    item
}

/// Defines a custom behaviour that is run as soon as the `tokio::task` in which the `Process` lives on is started.
///
/// ## Example
/// ```ignore
///#[process]
///impl Counter {
///    #[on_init]
///    async fn init(&mut self, ctx: &Ctx<Self>) {
///        println!("Hello!");
///    }
///}
/// ```
/// Learn more on [The Speare Book](https://vmenge.github.io/speare/pub_sub.html)
#[proc_macro_attribute]
pub fn on_init(_attr: TokenStream, item: TokenStream) -> TokenStream {
    item
}

/// Defines a custom behaviour to run when the `Process` is terminated.
///
/// ## Example
/// ```ignore
///#[process]
///impl Counter {
///    #[on_exit]
///    async fn exit(&mut self, ctx: &Ctx<Self>) {
///        println!("Goodbye!");
///    }
///}
/// ```
/// Learn more on [The Speare Book](https://vmenge.github.io/speare/pub_sub.html)
#[proc_macro_attribute]
pub fn on_exit(_attr: TokenStream, item: TokenStream) -> TokenStream {
    item
}

/// Allows handling of messages sent to this `Process`
///
/// ## Example
/// ```ignore
/// use speare::*;
///
///struct IncreaseBy(u64);
///struct RecursiveIncrease;
///
///struct Counter {
///    count: u64,
///}
///
///#[process]
///impl Counter {
///    #[handler]
///    async fn increment(&mut self, msg: IncreaseBy) -> Reply<(),()> {
///        self.count += msg.0;
///        reply(())
///    }
///
///    #[handler]
///    async fn inc_loop(&mut self, msg: RecursiveIncrease, ctx: &Ctx<Self>) -> Reply<(),()> {
///        self.count += 1;
///        ctx.tell(ctx.this(), RecursiveIncrease).await;
///        reply(())
///    }
///}
/// ```
/// Learn more on [The Speare Book](https://vmenge.github.io/speare/pub_sub.html)
#[proc_macro_attribute]
pub fn handler(_attr: TokenStream, item: TokenStream) -> TokenStream {
    item
}

struct ProcessArgs {
    error_type: Option<syn::Type>,
}

impl Parse for ProcessArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut error_type = None;

        if !input.is_empty() {
            let error_kw: Ident = input.parse()?;
            if error_kw == "Error" {
                input.parse::<Token![=]>()?;
                let lookahead = input.lookahead1();
                if lookahead.peek(syn::token::Lt) || lookahead.peek(syn::Ident) {
                    error_type = Some(input.parse()?);
                } else {
                    return Err(lookahead.error());
                }
            } else {
                return Err(syn::Error::new_spanned(
                    error_kw,
                    "Expected 'Error' keyword",
                ));
            }
        }

        Ok(ProcessArgs { error_type })
    }
}

#[proc_macro_attribute]
pub fn process(attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as ItemImpl);

    let (impl_generics, _, where_clause) = input.generics.split_for_impl();
    let self_type = &input.self_ty;

    let args = parse_macro_input!(attr as ProcessArgs);
    let error_type = args.error_type.unwrap_or_else(|| parse_quote! { () });

    let on_init_impl = process_on_init(&input);
    let on_exit_impl = process_on_exit(&input);
    let subscriptions = process_subscriptions(&input);

    let handlers_code = match p(input.clone()) {
        Ok(v) => v,
        Err(e) => {
            let error = syn::Error::new_spanned(input.clone(), e);
            error.to_compile_error().into()
        }
    };

    let process_impl = quote! {
        #[async_trait]
        impl #impl_generics Process for #self_type #where_clause {
            type Error = #error_type;

            #on_init_impl
            #on_exit_impl
            #subscriptions
        }
    };

    let mut token_stream = TokenStream::from(process_impl);
    token_stream.extend(handlers_code);

    println!("{}", token_stream);

    token_stream
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

                let ctx_arg_present = inputs.iter().any(matches_ctx_arg);
                let fn_call = if ctx_arg_present {
                    quote! { self.#fn_name(msg, ctx).await }
                } else {
                    quote! { self.#fn_name(msg).await }
                };

                additional_impls.push(quote! {
                    #[async_trait]
                    impl #impl_generics Handler<#msg_type> for #self_type #where_clause {
                        type Ok = #ok_type;
                        type Err = #err_type;

                        async fn handle(&mut self, msg: #msg_type, ctx: &Ctx<Self>) -> Reply<Self::Ok, Self::Err> {
                            #fn_call
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

fn matches_ctx_arg(arg: &FnArg) -> bool {
    if let FnArg::Typed(pat_type) = arg {
        if let Type::Reference(type_reference) = &*pat_type.ty {
            // Now check if the inner type is Ctx<Self>
            if let Type::Path(type_path) = &*type_reference.elem {
                // Check the type path for `Ctx` and the generic argument for `Self`
                return type_path.path.segments.iter().any(|segment| {
                    if segment.ident == "Ctx" {
                        if let syn::PathArguments::AngleBracketed(angle_args) = &segment.arguments {
                            angle_args.args.iter().any(matches_self)
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                });
            }
        }
    }
    false
}

fn matches_self(arg: &syn::GenericArgument) -> bool {
    if let syn::GenericArgument::Type(Type::Path(type_path)) = arg {
        // Check if the generic argument is `Self`
        type_path.path.is_ident("Self")
    } else {
        false
    }
}

fn process_on_init(input: &ItemImpl) -> Option<proc_macro2::TokenStream> {
    input.items.iter().find_map(|impl_item| {
        if let ImplItem::Fn(method) = impl_item {
            if method
                .attrs
                .iter()
                .any(|attr| attr.path().is_ident("on_init"))
            {
                let fn_name = &method.sig.ident;

                return Some(quote! {
                    async fn on_init(&mut self, ctx: &Ctx<Self>) {
                        self.#fn_name(ctx).await;
                    }
                });
            }
        }

        None
    })
}

fn process_on_exit(input: &ItemImpl) -> Option<proc_macro2::TokenStream> {
    input.items.iter().find_map(|impl_item| {
        if let ImplItem::Fn(method) = impl_item {
            if method
                .attrs
                .iter()
                .any(|attr| attr.path().is_ident("on_exit"))
            {
                let fn_name = &method.sig.ident;

                return Some(quote! {
                    async fn on_exit(&mut self, ctx: &Ctx<Self>) {
                        self.#fn_name(ctx).await;
                    }
                });
            }
        }

        None
    })
}

fn process_subscriptions(input: &ItemImpl) -> Option<proc_macro2::TokenStream> {
    input.items.iter().find_map(|impl_item| {
        if let ImplItem::Fn(method) = impl_item {
            if method
                .attrs
                .iter()
                .any(|attr| attr.path().is_ident("subscriptions"))
            {
                let fn_name = &method.sig.ident;

                return Some(quote! {
                    async fn subscriptions(&self, evt: &EventBus<Self>) {
                        self.#fn_name(evt).await;
                    }
                });
            }
        }

        None
    })
}
