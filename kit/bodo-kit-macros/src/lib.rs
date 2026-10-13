//! Attribute macros for Bodo Kit.
//!
//! Use them through `bodo_kit_core`, which re-exports them, so the code they generate can find its helpers.

#![forbid(unsafe_code)]

use proc_macro::TokenStream;
use quote::quote;
use syn::spanned::Spanned;

/// Marks a function that runs on the audio thread or an RT worker, and checks it.
///
/// The function's docs must say `RT-safe`. The body is wrapped in `bodo_kit_core::rt::guarded`, which:
///
/// - in debug builds, reports any allocation or deallocation through `assert_no_alloc`, when the program installs
///   its allocator;
/// - in builds made with `RTSAN_ENABLE` set, enters RealtimeSanitizer's real-time context, which also catches locks,
///   blocking calls and I/O.
///
/// In release builds without RealtimeSanitizer the wrapper compiles away. It works on free functions and methods;
/// `async` and `const` functions are rejected.
#[proc_macro_attribute]
pub fn rt_safe(attr: TokenStream, item: TokenStream) -> TokenStream {
    match expand_rt_safe(attr.into(), item.into()) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

fn expand_rt_safe(attr: proc_macro2::TokenStream, item: proc_macro2::TokenStream) -> syn::Result<proc_macro2::TokenStream> {
    if !attr.is_empty() {
        return Err(syn::Error::new(attr.span(), "#[rt_safe] takes no arguments"));
    }
    let mut function: syn::ItemFn = syn::parse2(item)?;
    if let Some(token) = &function.sig.asyncness {
        return Err(syn::Error::new(token.span, "#[rt_safe] can't be used on an async fn"));
    }
    if let Some(token) = &function.sig.constness {
        return Err(syn::Error::new(token.span, "#[rt_safe] can't be used on a const fn"));
    }
    if !documents_rt_safe(&function.attrs) {
        return Err(syn::Error::new(function.sig.ident.span(), "an #[rt_safe] function must say `RT-safe` in its docs"));
    }
    let body = function.block.clone();
    *function.block = syn::parse_quote!({ ::bodo_kit_core::rt::guarded(|| #body) });
    Ok(quote!(#function))
}

/// Whether the doc comments in `attrs` contain `RT-safe`.
fn documents_rt_safe(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().filter(|attr| attr.path().is_ident("doc")).any(|attr| match &attr.meta {
        syn::Meta::NameValue(syn::MetaNameValue { value: syn::Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(text), .. }), .. }) => {
            text.value().contains("RT-safe")
        }
        _ => false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expand(source: &str) -> Result<String, String> {
        let item: proc_macro2::TokenStream = source.parse().map_err(|err| format!("{err}"))?;
        expand_rt_safe(proc_macro2::TokenStream::new(), item).map(|tokens| tokens.to_string()).map_err(|err| err.to_string())
    }

    #[test]
    fn wraps_the_body() {
        let out = expand("/// Adds. RT-safe.\nfn add(a: u32, b: u32) -> u32 { a + b }").unwrap();
        assert!(out.contains(":: bodo_kit_core :: rt :: guarded (| | { a + b })"), "{out}");
        assert!(out.contains("fn add (a : u32 , b : u32) -> u32"), "{out}");
    }

    #[test]
    fn works_on_methods() {
        assert!(expand("/// RT-safe.\npub fn process(&mut self, out: &mut [f32]) { out.fill(0.0); }").is_ok());
    }

    #[test]
    fn rejects_what_it_cannot_check() {
        assert_eq!(expand("fn f() {}").unwrap_err(), "an #[rt_safe] function must say `RT-safe` in its docs");
        assert_eq!(expand("/// RT-safe.\nasync fn f() {}").unwrap_err(), "#[rt_safe] can't be used on an async fn");
        assert_eq!(expand("/// RT-safe.\nconst fn f() {}").unwrap_err(), "#[rt_safe] can't be used on a const fn");
        let with_args = expand_rt_safe("fast".parse().unwrap(), "/// RT-safe.\nfn f() {}".parse().unwrap());
        assert_eq!(with_args.unwrap_err().to_string(), "#[rt_safe] takes no arguments");
    }
}
