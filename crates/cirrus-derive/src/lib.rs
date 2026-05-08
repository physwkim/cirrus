//! `#[derive(Device)]` and helper attributes for cirrus.
//!
//! Generates from a struct definition:
//!
//! - `MyDevice::new(prefix: &str) -> Arc<Self>` — walks `#[signal(...)]`
//!   fields, builds each `Signal<T, B>` with the resolved PV name, and
//!   `#[device("subprefix")]` fields by recursively calling `Sub::new`.
//! - `MyDevice::connect_all(timeout) -> Result<()>` — concurrently connects
//!   every signal via `try_join_all`.
//! - `MyDevice::name() -> &str` — the prefix passed to `new`.
//!
//! Field attribute syntax:
//!
//! ```text
//! #[signal(rw, "{prefix}.VAL")]                   pub setpoint: Signal<f64, B>,
//! #[signal(ro, "{prefix}.RBV", kind = hinted)]    pub readback: Signal<f64, B>,
//! #[signal(rw, "{prefix}.VELO", kind = config)]   pub velocity: Signal<f64, B>,
//! #[device("{prefix}:x")]                         pub x: Arc<Motor>,
//! ```
//!
//! `#[signal(...)]` and `#[device(...)]` are *helper attributes* of the
//! derive — they have no standalone proc_macro_attribute and are resolved
//! through `#[derive(Device)]`.

#![deny(missing_docs)]

extern crate proc_macro;

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::quote;
use syn::{parse_macro_input, Data, DeriveInput, Field, Fields, Meta};

/// Derive `Device` for a struct.
#[proc_macro_derive(Device, attributes(signal, device))]
pub fn derive_device(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;
    let (impl_g, ty_g, where_g) = input.generics.split_for_impl();

    let fields = match &input.data {
        Data::Struct(s) => match &s.fields {
            Fields::Named(named) => &named.named,
            _ => {
                return syn::Error::new_spanned(
                    name,
                    "#[derive(Device)] only supports structs with named fields",
                )
                .to_compile_error()
                .into();
            }
        },
        _ => {
            return syn::Error::new_spanned(name, "#[derive(Device)] only supports structs")
                .to_compile_error()
                .into();
        }
    };

    let mut field_inits: Vec<TokenStream2> = Vec::new();
    let mut connect_calls: Vec<TokenStream2> = Vec::new();
    let mut has_name_field = false;

    for field in fields {
        let id = field.ident.as_ref().unwrap();

        if id == "name" {
            has_name_field = true;
            field_inits.push(quote! {
                name: prefix.to_string(),
            });
            continue;
        }

        if let Some(template) = parse_string_attr(field, "signal") {
            let kind_expr = parse_kind(field);
            field_inits.push(quote! {
                #id: ::cirrus_devices::Signal::new(
                    ::std::sync::Arc::new(::cirrus_devices::__derive::default_backend(
                        &::cirrus_devices::__derive::expand(#template, prefix),
                    )),
                    ::cirrus_devices::SignalConfig {
                        source: ::cirrus_devices::__derive::expand(#template, prefix),
                        kind: #kind_expr,
                        name: ::cirrus_devices::__derive::expand(#template, prefix),
                    },
                ),
            });
            connect_calls.push(quote! {
                ::cirrus_devices::__derive::connect_signal(&self.#id, timeout)
            });
            continue;
        }

        if let Some(template) = parse_string_attr(field, "device") {
            let ty = strip_arc(&field.ty);
            field_inits.push(quote! {
                #id: <#ty>::new(&::cirrus_devices::__derive::expand(#template, prefix)),
            });
            connect_calls.push(quote! {
                ::std::boxed::Box::pin(async move { self.#id.connect_all(timeout).await })
            });
            continue;
        }

        // Unattributed field — must be Default-constructible.
        field_inits.push(quote! {
            #id: ::core::default::Default::default(),
        });
    }

    if !has_name_field {
        return syn::Error::new_spanned(
            name,
            "#[derive(Device)] requires a `name: String` field on the struct",
        )
        .to_compile_error()
        .into();
    }

    let connect_block = if connect_calls.is_empty() {
        quote! { Ok(()) }
    } else {
        quote! {
            ::cirrus_devices::__derive::try_join_all_connects(vec![
                #( ::std::boxed::Box::pin(#connect_calls)
                    as ::std::pin::Pin<::std::boxed::Box<
                        dyn ::std::future::Future<Output = ::cirrus_core::error::Result<()>>
                            + ::std::marker::Send
                            + '_
                    >>, )*
            ]).await
        }
    };

    let expanded = quote! {
        impl #impl_g #name #ty_g #where_g {
            /// Build the device, allocating each signal under `prefix`.
            pub fn new(prefix: &str) -> ::std::sync::Arc<Self> {
                ::std::sync::Arc::new(Self { #( #field_inits )* })
            }

            /// Stable device name (the prefix passed to `new`).
            pub fn name(&self) -> &str {
                &self.name
            }

            /// Connect every signal concurrently.
            pub async fn connect_all(
                &self,
                timeout: ::std::time::Duration,
            ) -> ::cirrus_core::error::Result<()> {
                #connect_block
            }
        }
    };
    TokenStream::from(expanded)
}

/// Extract the first string literal in `#[name(...)]` if present.
fn parse_string_attr(field: &Field, name: &str) -> Option<String> {
    for attr in &field.attrs {
        if !attr.path().is_ident(name) {
            continue;
        }
        if let Meta::List(ml) = &attr.meta {
            let toks = ml.tokens.to_string();
            if let Some(start) = toks.find('"') {
                if let Some(end) = toks[start + 1..].find('"') {
                    return Some(toks[start + 1..start + 1 + end].to_string());
                }
            }
        }
    }
    None
}

/// Parse `kind = hinted|config|normal|omitted` out of `#[signal(...)]`.
fn parse_kind(field: &Field) -> TokenStream2 {
    for attr in &field.attrs {
        if !attr.path().is_ident("signal") {
            continue;
        }
        if let Meta::List(ml) = &attr.meta {
            let toks = ml.tokens.to_string();
            if let Some(idx) = toks.find("kind") {
                let tail = &toks[idx + 4..]; // skip "kind"
                let tail = tail.trim_start_matches([' ', '=', ',', '\t']);
                let word: String = tail
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                return match word.as_str() {
                    "hinted" => quote! { ::cirrus_core::Kind::Hinted },
                    "config" => quote! { ::cirrus_core::Kind::Config },
                    "normal" => quote! { ::cirrus_core::Kind::Normal },
                    "omitted" => quote! { ::cirrus_core::Kind::Omitted },
                    _ => quote! { ::cirrus_core::Kind::Normal },
                };
            }
        }
    }
    quote! { ::cirrus_core::Kind::Normal }
}

/// If the type is `Arc<T>`, strip to `T` (so `T::new(...)` resolves).
fn strip_arc(ty: &syn::Type) -> syn::Type {
    if let syn::Type::Path(tp) = ty {
        if let Some(last) = tp.path.segments.last() {
            if last.ident == "Arc" {
                if let syn::PathArguments::AngleBracketed(ab) = &last.arguments {
                    if let Some(syn::GenericArgument::Type(inner)) = ab.args.first() {
                        return inner.clone();
                    }
                }
            }
        }
    }
    ty.clone()
}
