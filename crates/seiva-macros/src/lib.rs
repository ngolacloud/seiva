//! Procedural macros for Delonix Seiva.
//!
//! See [`macro@facade`]. The design rule (ADR 0001, and the "no hidden magic"
//! principle) is that a macro keeps the user's code as written and only ADDS
//! generated items; everything it generates has a documented hand-written
//! equivalent, shown in `delonix-seiva`'s `contract` module docs.

use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{FnArg, ItemTrait, LitStr, Pat, ReturnType, TraitItem};

/// A method parameter captured from the trait: its name and canonical type.
struct ParamInfo {
    name: String,
    ty: String,
}

/// A facade method captured from the trait.
struct MethodInfo {
    name: String,
    requires: String,
    params: Vec<ParamInfo>,
    output: String,
}

/// Renders a type (or any token stream) as a canonical, space-stable string so
/// descriptor consumers (the gRPC and OpenAPI generators) can match on it.
/// `quote!` puts a space between every token; we drop the spaces that sit
/// against punctuation but keep one between two word tokens (e.g. `dyn Trait`),
/// then normalise a comma to be followed by exactly one space.
fn render_tokens(tokens: proc_macro2::TokenStream) -> String {
    let raw = tokens.to_string();
    let wordish = |c: char| c.is_alphanumeric() || c == '_';
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        if c == ' ' {
            let prev = out.chars().last();
            let next = *chars.peek().unwrap_or(&' ');
            if prev.is_some_and(wordish) && wordish(next) {
                out.push(' ');
            }
        } else {
            out.push(c);
        }
    }
    // Re-insert a single space after every comma (none was kept above).
    out.split(',')
        .map(str::trim_start)
        .collect::<Vec<_>>()
        .join(", ")
}

/// Marks a trait as a module's versioned public contract.
///
/// Applied as `#[seiva::facade(version = "v1")]` (the `version` defaults to
/// `"v1"` if omitted). The trait is re-emitted unchanged except that the
/// per-method helper attributes (`#[requires(..)]`, `#[policy(..)]`,
/// `#[cached(..)]`) are consumed. Alongside it, the macro generates a
/// zero-sized `<Trait>Facade` type implementing `delonix_seiva::contract::Facade`,
/// whose `DESCRIPTOR` lists the facade name, version, and every method with the
/// permission its `#[requires(..)]` declared. That descriptor is read by the
/// `.proto`/gRPC generator and by the `[[uses]]`/`[[allow]]` contract checks.
#[proc_macro_attribute]
pub fn facade(attr: TokenStream, item: TokenStream) -> TokenStream {
    let mut version = String::from("v1");
    let parser = syn::meta::parser(|meta| {
        if meta.path.is_ident("version") {
            version = meta.value()?.parse::<LitStr>()?.value();
            Ok(())
        } else {
            Err(meta.error("unknown `facade` option; expected `version = \"..\"`"))
        }
    });
    syn::parse_macro_input!(attr with parser);

    let mut item_trait = syn::parse_macro_input!(item as ItemTrait);

    // Collect each method's name, required permission and full signature
    // (parameters + return type), and strip the helper attributes so the
    // re-emitted trait compiles as ordinary Rust.
    let mut methods: Vec<MethodInfo> = Vec::new();
    for trait_item in &mut item_trait.items {
        let TraitItem::Fn(method) = trait_item else {
            continue;
        };
        let mut requires = String::new();
        let mut errors: Vec<syn::Error> = Vec::new();
        method.attrs.retain(|attr| {
            if attr.path().is_ident("requires") {
                match attr.parse_args::<LitStr>() {
                    Ok(lit) => requires = lit.value(),
                    Err(err) => errors.push(err),
                }
                false
            } else {
                // `policy` and `cached` are recognised contract attributes whose
                // behaviour lands in a later phase; consume them now so the
                // trait compiles, rather than leaving an unknown attribute.
                !(attr.path().is_ident("policy") || attr.path().is_ident("cached"))
            }
        });
        if let Some(err) = errors.into_iter().next() {
            return err.to_compile_error().into();
        }

        // Parameters: skip the `&self` receiver; capture each typed argument's
        // name and canonical type. A non-ident pattern (rare in a facade) is
        // rendered from its tokens so nothing is silently lost.
        let mut params: Vec<ParamInfo> = Vec::new();
        for input in &method.sig.inputs {
            let FnArg::Typed(pat_type) = input else {
                continue; // the receiver
            };
            let name = match pat_type.pat.as_ref() {
                Pat::Ident(pat_ident) => pat_ident.ident.to_string(),
                other => render_tokens(quote!(#other)),
            };
            let ty = &pat_type.ty;
            params.push(ParamInfo {
                name,
                ty: render_tokens(quote!(#ty)),
            });
        }

        // Return type: `()` when the method returns nothing. `async fn` desugars
        // to a future, but syn reports the return type as written in the trait.
        let output = match &method.sig.output {
            ReturnType::Default => "()".to_string(),
            ReturnType::Type(_, ty) => render_tokens(quote!(#ty)),
        };

        methods.push(MethodInfo {
            name: method.sig.ident.to_string(),
            requires,
            params,
            output,
        });
    }

    let vis = &item_trait.vis;
    let trait_ident = &item_trait.ident;
    let facade_ty = format_ident!("{}Facade", trait_ident);
    let name_str = trait_ident.to_string();
    let doc = format!(
        "Machine-readable contract descriptor for the `{trait_ident}` facade, \
         generated by `#[seiva::facade]`."
    );
    let method_descs = methods.iter().map(|method| {
        let MethodInfo {
            name,
            requires,
            params,
            output,
        } = method;
        let param_descs = params.iter().map(|ParamInfo { name, ty }| {
            quote! {
                ::delonix_seiva::contract::Param { name: #name, ty: #ty }
            }
        });
        quote! {
            ::delonix_seiva::contract::MethodDescriptor {
                name: #name,
                requires: #requires,
                params: &[ #(#param_descs),* ],
                output: #output,
            }
        }
    });

    quote! {
        // `async fn` in a public trait warns by default; a facade is a contract,
        // not a `dyn`-dispatched object, so the warning does not apply here.
        #[allow(async_fn_in_trait)]
        #item_trait

        #[doc = #doc]
        #vis struct #facade_ty;

        impl ::delonix_seiva::contract::Facade for #facade_ty {
            const DESCRIPTOR: ::delonix_seiva::contract::FacadeDescriptor =
                ::delonix_seiva::contract::FacadeDescriptor {
                    name: #name_str,
                    version: #version,
                    methods: &[ #(#method_descs),* ],
                };
        }
    }
    .into()
}
