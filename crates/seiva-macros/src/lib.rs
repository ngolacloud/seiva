//! Procedural macros for Delonix Seiva.
//!
//! See [`macro@facade`]. The design rule (ADR 0001, and the "no hidden magic"
//! principle) is that a macro keeps the user's code as written and only ADDS
//! generated items; everything it generates has a documented hand-written
//! equivalent, shown in `delonix-seiva`'s `contract` module docs.

use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{
    Data, DeriveInput, Fields, FnArg, GenericArgument, ImplItem, ItemImpl, ItemTrait, LitStr, Pat,
    PathArguments, ReturnType, TraitItem, Type,
};

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
/// per-method helper attributes (`#[requires(..)]`, `#[public]`, `#[policy(..)]`,
/// `#[cached(..)]`) are consumed. Alongside it, the macro generates a
///
/// # Authorization is fail-closed
///
/// Every method must declare its access policy (OWASP API5). A method with neither
/// `#[requires("perm")]` nor `#[public]` is a **compile error**, so forgetting to
/// guard a route cannot silently expose it. `#[public]` is the explicit, auditable
/// way to serve a method without authorization.
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
        let mut is_public = false;
        let mut errors: Vec<syn::Error> = Vec::new();
        method.attrs.retain(|attr| {
            if attr.path().is_ident("requires") {
                match attr.parse_args::<LitStr>() {
                    Ok(lit) => requires = lit.value(),
                    Err(err) => errors.push(err),
                }
                false
            } else if attr.path().is_ident("public") {
                // An explicit, auditable opt-out of the permission guard. Must take
                // no arguments: `#[public]`, not `#[public(..)]`.
                if !matches!(attr.meta, syn::Meta::Path(_)) {
                    errors.push(syn::Error::new_spanned(
                        attr,
                        "`#[public]` takes no arguments",
                    ));
                }
                is_public = true;
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

        // Authorization is fail-CLOSED at compile time (OWASP API5 — broken
        // function-level authorization): every facade method must state its access
        // policy. Forgetting `#[requires(..)]` no longer produces a silently public
        // route — it fails the build with an actionable message. A route meant to be
        // public says so explicitly with `#[public]`, which keeps `requires` empty —
        // the sentinel the guard reads as "no authorization" (see `permission_guard`).
        if requires.is_empty() && !is_public {
            return syn::Error::new_spanned(
                &method.sig,
                format!(
                    "facade method `{}` declares no access policy: add \
                     `#[requires(\"permission\")]` to guard it, or `#[public]` to \
                     expose it without authorization (an explicit, auditable choice)",
                    method.sig.ident
                ),
            )
            .to_compile_error()
            .into();
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

        // Register the facade in the link-time inventory so the runtime can
        // discover every mounted facade for `/openapi.json` (OWASP API9) without
        // a hand-maintained list.
        ::delonix_seiva::inventory::submit! {
            ::delonix_seiva::contract::FacadeRegistration {
                descriptor: <#facade_ty as ::delonix_seiva::contract::Facade>::DESCRIPTOR,
            }
        }
    }
    .into()
}

/// Derives `delonix_seiva::schema::WireSchema` for a DTO so it contributes its
/// own JSON Schema fragment to the OpenAPI generator, with no reflection (ADR
/// 0007). Applied as `#[derive(seiva::WireSchema)]` (re-exported as
/// `delonix_seiva::WireSchema`).
///
/// It implements `WireSchema` — an `object` schema whose properties are the
/// struct's fields, each field's schema composed from its type via
/// `<FieldTy as WireSchema>::schema()` — and registers the type by name in the
/// link-time schema inventory so a generator holding only a type-name string can
/// resolve the real shape. A field typed `Option<T>` is optional (left out of
/// `required`) and carries `T`'s schema.
///
/// Supports a struct with named fields — the shape of a wire DTO. A tuple
/// struct, a unit struct or an enum is rejected with a compile error, since those
/// have no field-name → schema mapping to document.
#[proc_macro_derive(WireSchema)]
pub fn derive_wire_schema(item: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(item as DeriveInput);
    let ident = &input.ident;
    let name_str = ident.to_string();

    // A generic DTO would need its schema parameterised per instantiation; the
    // by-name registry keys on a single name, so reject generics explicitly
    // rather than emit a registration that cannot be resolved.
    if !input.generics.params.is_empty() {
        return syn::Error::new_spanned(
            &input.generics,
            "WireSchema does not support generic types",
        )
        .to_compile_error()
        .into();
    }

    // The body of the generated `schema()`: an `object` for a struct with named
    // fields, or a string-`enum` for a C-like (unit-variant) enum.
    let schema_body = match &input.data {
        Data::Struct(data) => match &data.fields {
            Fields::Named(named) => {
                let field_descs = named.named.iter().map(|field| {
                    let field_name = field.ident.as_ref().expect("named field").to_string();
                    // An `Option<T>` field is not required and documents `T`'s schema.
                    let (schema_ty, required) = match option_inner(&field.ty) {
                        Some(inner) => (inner, false),
                        None => (field.ty.clone(), true),
                    };
                    quote! {
                        ::delonix_seiva::schema::Field {
                            name: #field_name,
                            schema: <#schema_ty as ::delonix_seiva::schema::WireSchema>::schema,
                            required: #required,
                        }
                    }
                });
                quote! { ::delonix_seiva::schema::object_schema(&[ #(#field_descs),* ]) }
            }
            _ => {
                return syn::Error::new_spanned(
                    ident,
                    "WireSchema supports a struct with named fields (a wire DTO)",
                )
                .to_compile_error()
                .into();
            }
        },
        Data::Enum(data) => {
            // Only C-like enums (every variant a unit) map faithfully to a string
            // enum. A data-carrying variant would need a `oneOf`, not yet supported.
            if let Some(bad) = data
                .variants
                .iter()
                .find(|v| !matches!(v.fields, Fields::Unit))
            {
                return syn::Error::new_spanned(
                    bad,
                    "WireSchema supports only unit-variant (C-like) enums; a data-carrying \
                     variant would need a `oneOf`, which is not yet supported",
                )
                .to_compile_error()
                .into();
            }
            let variant_names = data.variants.iter().map(|v| v.ident.to_string());
            quote! { ::delonix_seiva::schema::enum_schema(&[ #(#variant_names),* ]) }
        }
        Data::Union(_) => {
            return syn::Error::new_spanned(ident, "WireSchema does not support a union")
                .to_compile_error()
                .into();
        }
    };

    quote! {
        impl ::delonix_seiva::schema::WireSchema for #ident {
            fn schema() -> ::delonix_seiva::serde_json::Value {
                #schema_body
            }
        }

        // Register the type by name so a generator with only a type-name string
        // (the facade descriptor carries names, not shapes) resolves the real
        // schema through `delonix_seiva::schema::registered_schema`.
        ::delonix_seiva::inventory::submit! {
            ::delonix_seiva::schema::SchemaRegistration {
                name: #name_str,
                schema: <#ident as ::delonix_seiva::schema::WireSchema>::schema,
            }
        }
    }
    .into()
}

/// The inner type `T` of an `Option<T>`, or `None` if the type is not a plain
/// `Option<_>`. Matches on the final path segment, so `Option`, `std::option::Option`
/// and `core::option::Option` are all recognised.
fn option_inner(ty: &Type) -> Option<Type> {
    let Type::Path(type_path) = ty else {
        return None;
    };
    let segment = type_path.path.segments.last()?;
    if segment.ident != "Option" {
        return None;
    }
    let PathArguments::AngleBracketed(args) = &segment.arguments else {
        return None;
    };
    args.args.iter().find_map(|arg| match arg {
        GenericArgument::Type(inner) => Some(inner.clone()),
        _ => None,
    })
}

/// Converts a `CamelCase` trait name into a `snake_case` identifier fragment, so
/// the trait `Notes` yields the router builder `notes_router` and `UserAccounts`
/// yields `user_accounts_router`.
fn to_snake_case(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 4);
    for (i, ch) in name.chars().enumerate() {
        if ch.is_uppercase() {
            if i != 0 {
                out.push('_');
            }
            out.extend(ch.to_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

/// Whether a parameter type is the framework context (`Ctx` or `&Ctx`), which a
/// generated controller skips when mapping parameters to the wire. The match is
/// on the type's final path segment, so `&Ctx`, `Ctx`, `&self::Ctx` and
/// `crate::ctx::Ctx` are all recognised.
fn is_ctx_type(ty: &Type) -> bool {
    match ty {
        Type::Reference(reference) => is_ctx_type(&reference.elem),
        Type::Path(type_path) => type_path
            .path
            .segments
            .last()
            .is_some_and(|segment| segment.ident == "Ctx"),
        _ => false,
    }
}

/// Generates an axum [`Router`](https://docs.rs/axum)-builder from a facade trait
/// impl, mounting each method as an HTTP route whose path **version** and
/// **required permission** are read from the facade descriptor at run time — never
/// hard-coded. Apply it to the trait impl of a facade already marked with
/// [`macro@facade`]:
///
/// ```ignore
/// #[seiva::controller]
/// impl Notes for NotesService { /* the trait methods */ }
/// ```
///
/// Alongside the **re-emitted, unchanged** impl, it generates a public function
/// `fn <trait>_router(service) -> axum::Router` (the trait `Notes` yields
/// `notes_router`). Inject a service value, get back a router ready to `.merge`
/// into an application. The impl itself is not touched — "no hidden magic": the
/// macro only ADDS the builder.
///
/// # Routing convention
///
/// The **resource** is the facade's descriptor name, lowercased and pluralised:
/// a name already ending in `s` is taken as plural (`Notes` → `notes`), a
/// consonant + `y` becomes `ies` (`Inventory` → `inventories`), otherwise `s` is
/// appended (`User` → `users`). The rule is intentionally small; name a facade to
/// suit it. The **version** is `descriptor.version`
/// verbatim. The method name chooses the HTTP method and path shape:
///
/// | method name | HTTP route                          | success | request data |
/// |-------------|-------------------------------------|---------|--------------|
/// | `create`    | `POST /{version}/{resource}`        | `201`   | JSON body    |
/// | `get`       | `GET /{version}/{resource}/{id}`    | `200`   | path `{id}`  |
/// | `list`      | `GET /{version}/{resource}`         | `200`   | query (opt.) |
/// | *other*     | `POST /{version}/{resource}/{name}` | `200`   | JSON body    |
///
/// The required permission for each route is the method's
/// `#[requires(..)]`, read from the descriptor and enforced by
/// `seiva_http::permission_guard`. A method whose `requires` is empty is
/// unguarded.
///
/// # The generated handler
///
/// For every route the handler extracts the injected service from axum state,
/// deserialises the request into the facade's parameter DTO, calls the facade
/// method, maps the error to a `seiva_http::Problem` and serialises the `Ok`
/// value to JSON:
///
/// - **write** methods (`create` and the `POST` fallback): the single primary
///   parameter is read with `seiva_http::Valid<T>` (a validated JSON body); a
///   write with no primary parameter takes no body.
/// - **`get`**: the single primary parameter is read from the path with
///   `axum::extract::Path<T>`.
/// - **`list`**: an optional single primary parameter is read from the query
///   string with `axum::extract::Query<T>`.
///
/// # Simplifying assumptions (first cut)
///
/// - The facade's **own** parameter DTO is the wire type: a write DTO must
///   implement `serde::Deserialize` + `seiva_http::Validate`, a `get`/`list`
///   parameter must implement `serde::Deserialize`, and every `Ok` type must
///   implement `serde::Serialize`.
/// - The method's **error** type must implement `Into<seiva_http::Problem>`, so
///   each facade maps its failures to an RFC 9457 document (define
///   `impl From<MyError> for seiva_http::Problem` in the interface layer).
/// - A `&Ctx`/`Ctx` parameter is a framework context and is **skipped** when
///   mapping to the wire; it leaves at most one *primary* parameter per method.
/// - The service is injected as axum state, so its type must be
///   `Clone + Send + Sync + 'static`.
/// - The generated handlers return `Result<_, Problem>`; `Problem` is kept small
///   enough (its strings are `Box<str>`) that it never trips
///   `clippy::result_large_err`, so the consuming crate needs no lint allowance.
///
/// Anything outside these shapes is a **compile error**, never silently wrong
/// code:
///
/// - applied to something other than `impl Trait for Type`;
/// - a `get` method without exactly one primary parameter;
/// - a `list` or write method with more than one primary parameter;
/// - any argument passed to `#[controller]` (it takes none).
///
/// # The hand-written equivalent
///
/// For the `Notes` facade (one guarded `create`), the macro generates exactly
/// what `seiva-http`'s `notes_router` writes by hand — the route path and the
/// guard permission both taken from `NotesFacade::DESCRIPTOR`:
///
/// ```ignore
/// pub fn notes_router<__S>(service: __S) -> axum::Router
/// where
///     __S: Notes + Clone + Send + Sync + 'static,
/// {
///     let descriptor = <NotesFacade as delonix_seiva::contract::Facade>::DESCRIPTOR;
///     let resource = /* "notes", from descriptor.name */;
///     let create = descriptor.method("create").expect("…");
///     axum::Router::new()
///         .route(
///             &format!("/{}/{}", descriptor.version, resource), // "/v1/notes"
///             axum::routing::post(/* handler calling service.create */).layer(
///                 axum::middleware::from_fn_with_state(
///                     seiva_http::RequiredPermission(create.requires), // "notes.create"
///                     seiva_http::permission_guard,
///                 ),
///             ),
///         )
///         .with_state(service)
/// }
/// ```
#[proc_macro_attribute]
pub fn controller(attr: TokenStream, item: TokenStream) -> TokenStream {
    let attr = proc_macro2::TokenStream::from(attr);
    if !attr.is_empty() {
        return syn::Error::new_spanned(attr, "`#[seiva::controller]` takes no arguments")
            .to_compile_error()
            .into();
    }

    let item_impl = syn::parse_macro_input!(item as ItemImpl);

    // The impl must name a trait (`impl Trait for Type`): the trait path drives
    // the method calls and the `<Trait>Facade` descriptor lookup.
    let Some((trait_path, _)) = item_impl.trait_.clone() else {
        return syn::Error::new_spanned(
            &item_impl.self_ty,
            "`#[seiva::controller]` expects a trait impl, e.g. `impl Notes for NotesService`",
        )
        .to_compile_error()
        .into();
    };

    // `NotesFacade` lives beside the `Notes` trait: take the trait path and
    // rename its final segment, dropping any trait generic arguments.
    let mut facade_path = trait_path.clone();
    if let Some(last) = facade_path.segments.last_mut() {
        last.ident = format_ident!("{}Facade", last.ident);
        last.arguments = syn::PathArguments::None;
    }

    let trait_ident = &trait_path.segments.last().unwrap().ident;
    let fn_name = format_ident!("{}_router", to_snake_case(&trait_ident.to_string()));
    let self_ty = &item_impl.self_ty;

    // Build one leaf `Router` per method so that two methods sharing a path (e.g.
    // `create` POST and `list` GET both on `/{v}/{resource}`) coexist via `merge`
    // while keeping their own per-method permission guard.
    let mut route_blocks: Vec<proc_macro2::TokenStream> = Vec::new();
    for impl_item in &item_impl.items {
        let ImplItem::Fn(method) = impl_item else {
            continue;
        };
        let method_ident = &method.sig.ident;
        let method_name = method_ident.to_string();

        // Primary parameters: every typed argument that is not the `&Ctx` context.
        let primary: Vec<&Type> = method
            .sig
            .inputs
            .iter()
            .filter_map(|input| match input {
                FnArg::Typed(pat_type) if !is_ctx_type(&pat_type.ty) => Some(pat_type.ty.as_ref()),
                _ => None,
            })
            .collect();

        // Derive the HTTP method, path, request extractor, call argument and the
        // success status from the convention. `extractor` is a 0-or-1
        // element lists so the `#(, #x)*` splices add nothing for an empty one.
        let http_fn: proc_macro2::TokenStream;
        let path_expr: proc_macro2::TokenStream;
        let mut extractor: Vec<proc_macro2::TokenStream> = Vec::new();
        let ok_status: proc_macro2::TokenStream;

        // Call arguments in the method's DECLARED order: a `&Ctx`/`Ctx` parameter
        // gets the injected request context (the authenticated `Principal`, so a
        // handler can scope by tenant server-side); the single wire parameter gets
        // the extracted value. The `Ctx` extractor is `FromRequestParts`, so it is
        // pushed BEFORE the (body) wire extractor, which axum requires last.
        let mut ordered_call: Vec<proc_macro2::TokenStream> = Vec::new();
        let mut uses_ctx = false;
        for input in &method.sig.inputs {
            let FnArg::Typed(pat_type) = input else {
                continue;
            };
            if is_ctx_type(&pat_type.ty) {
                uses_ctx = true;
                ordered_call.push(match pat_type.ty.as_ref() {
                    Type::Reference(_) => quote!(&__ctx),
                    _ => quote!(__ctx.clone()),
                });
            } else {
                ordered_call.push(quote!(__arg));
            }
        }
        if uses_ctx {
            extractor.push(quote!(__ctx: ::seiva_http::Ctx));
        }

        match method_name.as_str() {
            "get" => {
                if primary.len() != 1 {
                    return syn::Error::new_spanned(
                        &method.sig,
                        "a `get` controller method must take exactly one id parameter \
                         (besides an optional `&Ctx`)",
                    )
                    .to_compile_error()
                    .into();
                }
                let ty = primary[0];
                http_fn = quote!(get);
                path_expr = quote!(::std::format!(
                    "/{}/{}/{{id}}",
                    __descriptor.version,
                    __resource
                ));
                extractor.push(quote!(::axum::extract::Path(__arg): ::axum::extract::Path<#ty>));
                ok_status = quote!(::axum::http::StatusCode::OK);
            }
            "list" => {
                if primary.len() > 1 {
                    return syn::Error::new_spanned(
                        &method.sig,
                        "a `list` controller method must take at most one query parameter \
                         (besides an optional `&Ctx`)",
                    )
                    .to_compile_error()
                    .into();
                }
                http_fn = quote!(get);
                path_expr = quote!(::std::format!("/{}/{}", __descriptor.version, __resource));
                if let Some(ty) = primary.first() {
                    extractor
                        .push(quote!(::axum::extract::Query(__arg): ::axum::extract::Query<#ty>));
                }
                ok_status = quote!(::axum::http::StatusCode::OK);
            }
            other => {
                // `create` and every other name are write routes (a `POST`).
                if primary.len() > 1 {
                    return syn::Error::new_spanned(
                        &method.sig,
                        "a write controller method must take at most one body parameter \
                         (besides an optional `&Ctx`)",
                    )
                    .to_compile_error()
                    .into();
                }
                http_fn = quote!(post);
                if other == "create" {
                    path_expr = quote!(::std::format!("/{}/{}", __descriptor.version, __resource));
                    ok_status = quote!(::axum::http::StatusCode::CREATED);
                } else {
                    path_expr = quote!(::std::format!(
                        "/{}/{}/{}",
                        __descriptor.version,
                        __resource,
                        #method_name
                    ));
                    ok_status = quote!(::axum::http::StatusCode::OK);
                }
                if let Some(ty) = primary.first() {
                    extractor.push(quote!(::seiva_http::Valid(__arg): ::seiva_http::Valid<#ty>));
                }
            }
        }

        let missing = format!("facade descriptor is missing method `{method_name}`");
        route_blocks.push(quote! {
            {
                let __method = __descriptor
                    .method(#method_name)
                    .expect(#missing);
                let __path = #path_expr;
                let __leaf: ::axum::Router<#self_ty> = ::axum::Router::new().route(
                    &__path,
                    ::axum::routing::#http_fn(
                        |
                            ::axum::extract::State(__service): ::axum::extract::State<#self_ty>
                            #(, #extractor)*
                        | async move {
                            match <#self_ty as #trait_path>::#method_ident(
                                &__service #(, #ordered_call)*
                            ).await {
                                ::core::result::Result::Ok(__value) =>
                                    ::core::result::Result::<
                                        ::axum::response::Response,
                                        ::seiva_http::Problem,
                                    >::Ok(::axum::response::IntoResponse::into_response((
                                        #ok_status,
                                        ::axum::Json(__value),
                                    ))),
                                ::core::result::Result::Err(__err) =>
                                    ::core::result::Result::Err(
                                        ::core::convert::Into::into(__err),
                                    ),
                            }
                        },
                    )
                    .layer(::axum::middleware::from_fn_with_state(
                        ::seiva_http::RequiredPermission(__method.requires),
                        ::seiva_http::permission_guard,
                    )),
                );
                __router = __router.merge(__leaf);
            }
        });
    }

    // Copy the impl's generics/where-clause onto the builder and add the axum
    // state bound on the concrete service type.
    let (impl_generics, _type_generics, where_clause) = item_impl.generics.split_for_impl();
    let user_predicates = where_clause.map(|wc| {
        let predicates = &wc.predicates;
        quote!(#predicates)
    });

    let fn_doc = format!(
        "Builds the `{trait_ident}` controller router, mounting each facade \
         method as an HTTP route. Generated by `#[seiva::controller]`; the route \
         version and each route's required permission come from the facade \
         descriptor at run time. Inject a `Clone + Send + Sync + 'static` service."
    );

    quote! {
        #item_impl

        #[doc = #fn_doc]
        pub fn #fn_name #impl_generics (service: #self_ty) -> ::axum::Router
        where
            #self_ty: ::core::clone::Clone
                + ::core::marker::Send
                + ::core::marker::Sync
                + 'static,
            #user_predicates
        {
            let __descriptor =
                <#facade_path as ::delonix_seiva::contract::Facade>::DESCRIPTOR;
            // Resource path segment from the descriptor. The rule lives in
            // `delonix_seiva::contract::resource_segment` so route generation and
            // the boot-time collision check (`route_collisions`) share one source
            // of truth and cannot drift apart.
            let __resource = ::delonix_seiva::contract::resource_segment(__descriptor.name);
            let mut __router: ::axum::Router<#self_ty> = ::axum::Router::new();
            #(#route_blocks)*
            __router.with_state(service)
        }
    }
    .into()
}
