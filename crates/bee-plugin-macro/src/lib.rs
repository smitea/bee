//! S33.6: `#[bee_adapter]` proc-macro.
//!
//! Turns a hand-written `impl` block into the
//! FFI glue: 2-3 `unsafe extern "C" fn` + a
//! per-instance ctx struct + a `static
//! FOO_VTABLE: Vtable` constant.
//!
//! Wire format (vtable layout, Event bincode
//! schema) is defined in `bee-plugin-sdk` and
//! unchanged.

extern crate proc_macro;

use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::{format_ident, quote};
use syn::{
    parse::{Parse, ParseStream},
    parse_macro_input, Error, ImplItem, ImplItemFn, ItemImpl, LitStr, Token,
    Type,
};

struct AdapterArgs {
    kind: AdapterKind,
    #[allow(dead_code)]
    name: String,
}

enum AdapterKind {
    Input,
    Output,
    Handler,
}

impl Parse for AdapterArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let kind_ident: syn::Ident = input.parse()?;
        let kind = match kind_ident.to_string().as_str() {
            "input" => AdapterKind::Input,
            "output" => AdapterKind::Output,
            "handler" => AdapterKind::Handler,
            other => {
                return Err(Error::new(
                    kind_ident.span(),
                    format!(
                        "bee_adapter: kind must be 'input', 'output', or 'handler', got '{}'",
                        other
                    ),
                ));
            }
        };
        let mut name = String::from("<unnamed>");
        while !input.is_empty() {
            let _comma: Token![,] = input.parse()?;
            if input.is_empty() { break; }
            let ident: syn::Ident = input.parse()?;
            let _eq: Token![=] = input.parse()?;
            let val: LitStr = input.parse()?;
            if ident == "name" {
                name = val.value();
            } else {
                return Err(Error::new(
                    ident.span(),
                    format!("bee_adapter: unknown arg '{}'", ident),
                ));
            }
        }
        Ok(Self { kind, name })
    }
}

struct MethodArgs {
    slot: String,
}

impl Parse for MethodArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let ident: syn::Ident = input.parse()?;
        let _eq: Token![=] = input.parse()?;
        let val: LitStr = input.parse()?;
        if ident != "slot" {
            return Err(Error::new(
                ident.span(),
                format!("bee_method: unknown arg '{}'; expected `slot = \"...\"`", ident),
            ));
        }
        Ok(Self { slot: val.value() })
    }
}

#[proc_macro_attribute]
pub fn bee_adapter(args: TokenStream, input: TokenStream) -> TokenStream {
    let adapter_args = parse_macro_input!(args as AdapterArgs);
    let impl_block = parse_macro_input!(input as ItemImpl);

    match adapter_args.kind {
        AdapterKind::Input => gen_input_adapter(impl_block),
        AdapterKind::Output => gen_output_adapter(impl_block),
        AdapterKind::Handler => gen_handler(impl_block),
    }
}

#[proc_macro_attribute]
pub fn bee_method(args: TokenStream, input: TokenStream) -> TokenStream {
    // Pass-through: the impl block is processed
    // by `bee_adapter` in one pass; `bee_method`
    // attributes are stripped there.
    let _ = args;
    let _ = input;
    TokenStream::new()
}

fn get_struct_name(impl_block: &ItemImpl) -> syn::Result<proc_macro2::Ident> {
    if let Type::Path(tp) = &*impl_block.self_ty {
        Ok(tp
            .path
            .segments
            .last()
            .expect("impl has no self type")
            .ident
            .clone())
    } else {
        Err(Error::new(
            Span::call_site(),
            "bee_adapter: self_ty must be a path",
        ))
    }
}

fn vtable_ident(struct_name: &proc_macro2::Ident) -> proc_macro2::Ident {
    // Convert `MockInput` → `MOCK_INPUT_VTABLE`
    // (snake_case then SCREAMING_SNAKE_CASE).
    let snake = to_snake_case(&struct_name.to_string());
    let upper = snake.to_uppercase();
    format_ident!("{}_VTABLE", upper)
}

fn ffi_ident(struct_name: &proc_macro2::Ident, slot: &str) -> proc_macro2::Ident {
    let snake = to_snake_case(&struct_name.to_string());
    format_ident!("{}_{}", snake, slot)
}

fn to_snake_case(s: &str) -> String {
    // Insert `_` before each uppercase letter
    // (except the first) and lowercase.
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if c.is_ascii_uppercase() && i > 0 {
            out.push('_');
        }
        out.push(c.to_ascii_lowercase());
    }
    out
}

fn err(msg: &str) -> TokenStream {
    Error::new(Span::call_site(), msg).to_compile_error().into()
}

fn gen_input_adapter(impl_block: ItemImpl) -> TokenStream {
    let struct_name = match get_struct_name(&impl_block) {
        Ok(n) => n,
        Err(e) => return e.to_compile_error().into(),
    };
    let vtable_name = vtable_ident(&struct_name);

    let mut open_fn: Option<ImplItemFn> = None;
    let mut next_fn: Option<ImplItemFn> = None;
    let mut close_fn: Option<ImplItemFn> = None;
    for item in &impl_block.items {
        if let ImplItem::Fn(f) = item {
            let slot = match extract_slot(&f.attrs) {
                Ok(s) => s,
                Err(e) => return e.to_compile_error().into(),
            };
            match slot.as_deref() {
                Some("open") => open_fn = Some(f.clone()),
                Some("next") => next_fn = Some(f.clone()),
                Some("close") => close_fn = Some(f.clone()),
                _ => {}
            }
        }
    }
    let has_custom_open = open_fn.is_some();
    let has_custom_close = close_fn.is_some();
    if let Some(f) = &open_fn {
        if f.sig.asyncness.is_none() {
            return err(
                "bee_adapter(input): `open` must be `async fn` (S33.6 signature check)",
            );
        }
    }
    let open_rust = open_fn.as_ref().map(|f| f.sig.ident.clone());
    let next_fn = match next_fn {
        Some(f) => f,
        None => return err("`#[bee_method(slot = \"next\")]` not found"),
    };
    let close_rust = close_fn.as_ref().map(|f| f.sig.ident.clone());

    let ctx_ty = format_ident!("{}InputCtx", struct_name);
    let open_ffi = ffi_ident(&struct_name, "input_open");
    let next_ffi = ffi_ident(&struct_name, "input_next");
    let close_ffi = ffi_ident(&struct_name, "input_close");
    let next_rust = &next_fn.sig.ident;

    let mut impl_block = impl_block;
    for item in &mut impl_block.items {
        if let ImplItem::Fn(f) = item {
            f.attrs.retain(|a| !is_bee_method_attr(a));
        }
    }

    // Generate the open FFI body — either custom
    // (calls the user's open method) or default
    // (uses Default::default()).
    let open_body = if has_custom_open {
        let open_rust = open_rust.expect("has_custom_open");
        quote! {
            let adapter = {
                let config = unsafe {
                    ::std::slice::from_raw_parts(config_ptr, config_len).to_vec()
                };
                let fut = async move {
                    <#struct_name>::#open_rust(config).await
                };
                match ::tokio::runtime::Handle::try_current() {
                    Ok(h) => ::tokio::task::block_in_place(|| h.block_on(fut)),
                    Err(_) => ::futures::executor::block_on(fut),
                }
            };
        }
    } else {
        quote! {
            let adapter: ::std::result::Result<#struct_name, bee_adapter::AdapterError> =
                Ok(<#struct_name as ::std::default::Default>::default());
        }
    };

    // Generate the close FFI body — either custom
    // (calls the user's close method) or default
    // (just drops the adapter).
    let close_body = if has_custom_close {
        let close_rust = close_rust.expect("has_custom_close");
        quote! {
            if let Some(adapter) = adapter {
                let fut = adapter.#close_rust();
                let _ = match ::tokio::runtime::Handle::try_current() {
                    Ok(h) => ::tokio::task::block_in_place(|| h.block_on(fut)),
                    Err(_) => ::futures::executor::block_on(fut),
                };
            }
        }
    } else {
        quote! {
            // No custom close — just drop the adapter.
            drop(adapter);
        }
    };

    quote! {
        struct #ctx_ty {
            inner: ::tokio::sync::Mutex<Option<#struct_name>>,
        }

        unsafe extern "C" fn #open_ffi(
            config_ptr: *const u8,
            config_len: usize,
            _err_out: *mut bee_plugin_sdk::event::EventBytes,
        ) -> *mut ::std::ffi::c_void {
            #open_body
            let adapter = match adapter {
                Ok(a) => a,
                Err(_) => return ::std::ptr::null_mut(),
            };
            let ctx = #ctx_ty {
                inner: ::tokio::sync::Mutex::new(Some(adapter)),
            };
            ::std::boxed::Box::into_raw(::std::boxed::Box::new(ctx))
                as *mut ::std::ffi::c_void
        }

        unsafe extern "C" fn #next_ffi(
            ctx: *mut ::std::ffi::c_void,
            out: *mut bee_plugin_sdk::event::EventBytes,
        ) -> i32 {
            let ctx = unsafe { &*(ctx as *const #ctx_ty) };
            let mut guard = match ctx.inner.try_lock() {
                Ok(g) => g,
                Err(_) => return -1,
            };
            let adapter = match guard.as_mut() {
                Some(a) => a,
                None => {
                    *out = bee_plugin_sdk::event::EventBytes::EMPTY;
                    return 0;
                }
            };
            let fut = adapter.#next_rust();
            let result = match ::tokio::runtime::Handle::try_current() {
                Ok(h) => ::tokio::task::block_in_place(|| h.block_on(fut)),
                Err(_) => ::futures::executor::block_on(fut),
            };
            match result {
                Ok(Some(event)) => {
                    let bytes = match bincode::serialize(&event) {
                        Ok(b) => b,
                        Err(_) => return -1,
                    };
                    let len = bytes.len();
                    let ptr = bytes.as_ptr();
                    ::std::mem::forget(bytes);
                    *out = bee_plugin_sdk::event::EventBytes { ptr, len };
                    1
                }
                Ok(None) => {
                    *out = bee_plugin_sdk::event::EventBytes::EMPTY;
                    0
                }
                Err(_) => -1,
            }
        }

        unsafe extern "C" fn #close_ffi(
            ctx: *mut ::std::ffi::c_void,
        ) -> i32 {
            if ctx.is_null() { return 0; }
            let ctx = unsafe {
                ::std::boxed::Box::from_raw(ctx as *mut #ctx_ty)
            };
            let adapter = match ctx.inner.try_lock() {
                Ok(mut g) => g.take(),
                Err(_) => return -1,
            };
            #close_body
            0
        }

        pub static #vtable_name: bee_plugin_sdk::vtable::InputAdapterVtable =
            bee_plugin_sdk::vtable::InputAdapterVtable {
                open: #open_ffi,
                next: #next_ffi,
                close: #close_ffi,
            };

        #impl_block
    }
    .into()
}

fn gen_output_adapter(impl_block: ItemImpl) -> TokenStream {
    let struct_name = match get_struct_name(&impl_block) {
        Ok(n) => n,
        Err(e) => return e.to_compile_error().into(),
    };
    let vtable_name = vtable_ident(&struct_name);

    let mut open_fn: Option<ImplItemFn> = None;
    let mut emit_fn: Option<ImplItemFn> = None;
    let mut close_fn: Option<ImplItemFn> = None;
    for item in &impl_block.items {
        if let ImplItem::Fn(f) = item {
            let slot = match extract_slot(&f.attrs) {
                Ok(s) => s,
                Err(e) => return e.to_compile_error().into(),
            };
            match slot.as_deref() {
                Some("open") => open_fn = Some(f.clone()),
                Some("emit") => emit_fn = Some(f.clone()),
                Some("close") => close_fn = Some(f.clone()),
                _ => {}
            }
        }
    }
    let open_fn = match open_fn {
        Some(f) => f,
        None => return err("`#[bee_method(slot = \"open\")]` not found"),
    };
    if open_fn.sig.asyncness.is_none() {
        return err(
            "bee_adapter(output): `open` must be `async fn` (S33.6 signature check)",
        );
    }
    let emit_fn = match emit_fn {
        Some(f) => f,
        None => return err("`#[bee_method(slot = \"emit\")]` not found"),
    };
    let close_fn = match close_fn {
        Some(f) => f,
        None => return err("`#[bee_method(slot = \"close\")]` not found"),
    };

    let ctx_ty = format_ident!("{}OutputCtx", struct_name);
    let open_ffi = ffi_ident(&struct_name, "output_open");
    let emit_ffi = ffi_ident(&struct_name, "output_emit");
    let close_ffi = ffi_ident(&struct_name, "output_close");
    let open_rust = &open_fn.sig.ident;
    let emit_rust = &emit_fn.sig.ident;
    let close_rust = &close_fn.sig.ident;

    let mut impl_block = impl_block;
    for item in &mut impl_block.items {
        if let ImplItem::Fn(f) = item {
            f.attrs.retain(|a| !is_bee_method_attr(a));
        }
    }

    quote! {
        struct #ctx_ty {
            inner: ::tokio::sync::Mutex<Option<#struct_name>>,
        }

        unsafe extern "C" fn #open_ffi(
            config_ptr: *const u8,
            config_len: usize,
            _err_out: *mut bee_plugin_sdk::event::EventBytes,
        ) -> *mut ::std::ffi::c_void {
            let config = unsafe {
                ::std::slice::from_raw_parts(config_ptr, config_len).to_vec()
            };
            let fut = async move {
                <#struct_name>::#open_rust(config).await
            };
            let adapter = match ::tokio::runtime::Handle::try_current() {
                Ok(h) => ::tokio::task::block_in_place(|| h.block_on(fut)),
                Err(_) => ::futures::executor::block_on(fut),
            };
            let adapter = match adapter {
                Ok(a) => a,
                Err(_) => return ::std::ptr::null_mut(),
            };
            let ctx = #ctx_ty {
                inner: ::tokio::sync::Mutex::new(Some(adapter)),
            };
            ::std::boxed::Box::into_raw(::std::boxed::Box::new(ctx))
                as *mut ::std::ffi::c_void
        }

        unsafe extern "C" fn #emit_ffi(
            ctx: *mut ::std::ffi::c_void,
            event_ptr: *const u8,
            event_len: usize,
            err_out: *mut bee_plugin_sdk::event::EventBytes,
        ) -> i32 {
            let ctx = unsafe { &*(ctx as *const #ctx_ty) };
            let bytes = unsafe {
                ::std::slice::from_raw_parts(event_ptr, event_len)
            };
            let event: bee_adapter::Event = match bincode::deserialize(bytes) {
                Ok(e) => e,
                Err(_) => return -1,
            };
            let mut guard = match ctx.inner.try_lock() {
                Ok(g) => g,
                Err(_) => return -1,
            };
            let adapter = match guard.as_mut() {
                Some(a) => a,
                None => return -1,
            };
            let fut = adapter.#emit_rust(event);
            let result = match ::tokio::runtime::Handle::try_current() {
                Ok(h) => ::tokio::task::block_in_place(|| h.block_on(fut)),
                Err(_) => ::futures::executor::block_on(fut),
            };
            match result {
                Ok(()) => 0,
                Err(e) => {
                    // Write the error message to err_out
                    // as an Event-shaped blob so the host
                    // can pick it up via the same decoder.
                    if !err_out.is_null() {
                        let err_event = bee_adapter::Event {
                            timestamp: 0,
                            sequence: 0,
                            payload: format!("{e}").into_bytes(),
                        };
                        let bytes = match bincode::serialize(&err_event) {
                            Ok(b) => b,
                            Err(_) => return -1,
                        };
                        let len = bytes.len();
                        let ptr = bytes.as_ptr();
                        ::std::mem::forget(bytes);
                        unsafe {
                            *err_out = bee_plugin_sdk::event::EventBytes { ptr, len };
                        }
                    }
                    -1
                }
            }
        }

        unsafe extern "C" fn #close_ffi(
            ctx: *mut ::std::ffi::c_void,
        ) -> i32 {
            if ctx.is_null() { return 0; }
            let ctx = unsafe {
                ::std::boxed::Box::from_raw(ctx as *mut #ctx_ty)
            };
            let adapter = match ctx.inner.try_lock() {
                Ok(mut g) => g.take(),
                Err(_) => return -1,
            };
            if let Some(adapter) = adapter {
                let fut = adapter.#close_rust();
                let _ = match ::tokio::runtime::Handle::try_current() {
                    Ok(h) => ::tokio::task::block_in_place(|| h.block_on(fut)),
                    Err(_) => ::futures::executor::block_on(fut),
                };
            }
            0
        }

        pub static #vtable_name: bee_plugin_sdk::vtable::OutputAdapterVtable =
            bee_plugin_sdk::vtable::OutputAdapterVtable {
                open: #open_ffi,
                emit: #emit_ffi,
                close: #close_ffi,
            };

        #impl_block
    }
    .into()
}

fn gen_handler(impl_block: ItemImpl) -> TokenStream {
    let struct_name = match get_struct_name(&impl_block) {
        Ok(n) => n,
        Err(e) => return e.to_compile_error().into(),
    };
    let vtable_name = vtable_ident(&struct_name);

    let mut handle_fn: Option<ImplItemFn> = None;
    let mut init_state_fn: Option<ImplItemFn> = None;
    for item in &impl_block.items {
        if let ImplItem::Fn(f) = item {
            let slot = match extract_slot(&f.attrs) {
                Ok(s) => s,
                Err(e) => return e.to_compile_error().into(),
            };
            match slot.as_deref() {
                Some("handle") => handle_fn = Some(f.clone()),
                Some("init_state") => init_state_fn = Some(f.clone()),
                _ => {}
            }
        }
    }
    let handle_fn = match handle_fn {
        Some(f) => f,
        None => return err("`#[bee_method(slot = \"handle\")]` not found"),
    };
    if handle_fn.sig.asyncness.is_none() {
        return err(
            "bee_adapter(handler): `handle` must be `async fn` (S33.6 signature check)",
        );
    }

    // Extract the user's state type (the first
    // arg of `handle`) and event type (the second
    // arg). The macro will bincode-deserialize
    // the state blob to the user's type.
    let state_ty = match handle_fn.sig.inputs.first() {
        Some(syn::FnArg::Typed(pt)) => &*pt.ty,
        _ => return err("`handle` first arg must be a typed state parameter"),
    };
    let event_ty = match handle_fn.sig.inputs.get(1) {
        Some(syn::FnArg::Typed(pt)) => &*pt.ty,
        _ => return err("`handle` second arg must be a typed event parameter"),
    };

    let handle_ffi = ffi_ident(&struct_name, "handler_handle");
    let init_state_ffi = ffi_ident(&struct_name, "handler_init_state");
    let handle_rust = &handle_fn.sig.ident;
    // If the user provided a custom
    // `init_state`, capture its ident +
    // return type. Otherwise, default
    // `init_state` returns empty bytes.
    let has_custom_init = init_state_fn.is_some();
    let (init_state_rust, init_state_ret_ty): (
        Option<proc_macro2::Ident>,
        Type,
    ) = if let Some(isf) = &init_state_fn {
        (
            Some(isf.sig.ident.clone()),
            match &isf.sig.output {
                syn::ReturnType::Type(_, t) => (**t).clone(),
                _ => Type::Tuple(syn::TypeTuple {
                    elems: Default::default(),
                    paren_token: Default::default(),
                }),
            },
        )
    } else {
        (
            None,
            Type::Path(syn::TypePath {
                qself: None,
                path: syn::parse_quote!(::std::vec::Vec<u8>),
            }),
        )
    };

    let mut impl_block = impl_block;
    for item in &mut impl_block.items {
        if let ImplItem::Fn(f) = item {
            f.attrs.retain(|a| !is_bee_method_attr(a));
        }
    }

    // The `init_state` body: either a
    // call to the user's async fn
    // (serializing the result) or an
    // empty `Vec<u8>`.
    let init_state_body: proc_macro2::TokenStream = if has_custom_init {
        let init_state_rust = init_state_rust
            .expect("init_state_rust must be Some when has_custom_init");
        quote! {
            {
                let fut = async {
                    <#struct_name>::#init_state_rust().await
                };
                let state: #init_state_ret_ty = match ::tokio::runtime::Handle::try_current() {
                    Ok(h) => ::tokio::task::block_in_place(
                        || h.block_on(fut)
                    ),
                    Err(_) => ::futures::executor::block_on(fut),
                };
                match state {
                    Ok(s) => match bincode::serialize(&s) {
                        Ok(b) => b,
                        Err(_) => ::std::vec::Vec::new(),
                    },
                    Err(_) => ::std::vec::Vec::new(),
                }
            }
        }
    } else {
        quote! { ::std::vec::Vec::new() }
    };

    quote! {
        // Handler vtable is stateless at the
        // FFI level: the host owns the state
        // blob; the handler is a pure
        // associated function.
        unsafe extern "C" fn #handle_ffi(
            state_ptr: *const u8,
            state_len: usize,
            event_ptr: *const u8,
            event_len: usize,
            new_state_out: *mut bee_plugin_sdk::event::EventBytes,
            result_out: *mut bee_plugin_sdk::event::EventBytes,
            _err_out: *mut bee_plugin_sdk::event::EventBytes,
        ) -> i32 {
            let state_bytes = unsafe {
                ::std::slice::from_raw_parts(state_ptr, state_len)
            };
            let event_bytes = unsafe {
                ::std::slice::from_raw_parts(event_ptr, event_len)
            };
            let state: #state_ty = match bincode::deserialize(state_bytes) {
                Ok(s) => s,
                Err(_) => return -1,
            };
            let event: #event_ty = match bincode::deserialize(event_bytes) {
                Ok(e) => e,
                Err(_) => return -1,
            };
            let fut = async move {
                <#struct_name>::#handle_rust(state, event).await
            };
            let result = match ::tokio::runtime::Handle::try_current() {
                Ok(h) => ::tokio::task::block_in_place(|| h.block_on(fut)),
                Err(_) => ::futures::executor::block_on(fut),
            };
            match result {
                Ok((new_state, result)) => {
                    let new_state_bytes = match bincode::serialize(&new_state) {
                        Ok(b) => b,
                        Err(_) => return -1,
                    };
                    let result_bytes = match bincode::serialize(&result) {
                        Ok(b) => b,
                        Err(_) => return -1,
                    };
                    let n_len = new_state_bytes.len();
                    let n_ptr = new_state_bytes.as_ptr();
                    let r_len = result_bytes.len();
                    let r_ptr = result_bytes.as_ptr();
                    ::std::mem::forget(new_state_bytes);
                    ::std::mem::forget(result_bytes);
                    *new_state_out = bee_plugin_sdk::event::EventBytes {
                        ptr: n_ptr,
                        len: n_len,
                    };
                    *result_out = bee_plugin_sdk::event::EventBytes {
                        ptr: r_ptr,
                        len: r_len,
                    };
                    0
                }
                Err(_) => -1,
            }
        }

        /// `init_state`: returns the
        /// bincode-encoded initial state
        /// blob. If the user provided a
        /// custom `#[bee_method(slot =
        /// "init_state")]` async fn, the
        /// macro calls it and serializes the
        /// result. Otherwise, returns an
        /// empty `Vec<u8>`.
        unsafe extern "C" fn #init_state_ffi(
            out: *mut bee_plugin_sdk::event::EventBytes,
        ) -> i32 {
            let bytes: ::std::vec::Vec<u8> = #init_state_body;
            let len = bytes.len();
            let ptr = bytes.as_ptr();
            ::std::mem::forget(bytes);
            *out = bee_plugin_sdk::event::EventBytes { ptr, len };
            0
        }

        pub static #vtable_name: bee_plugin_sdk::vtable::HandlerVtable =
            bee_plugin_sdk::vtable::HandlerVtable {
                handle: #handle_ffi,
                init_state: #init_state_ffi,
            };

        #impl_block
    }
    .into()
}

fn is_bee_method_attr_path(path: &syn::Path) -> bool {
    // Match `#[bee_method]` or `#[bee_plugin_macro::bee_method]`
    // (or any other qualified path ending in `bee_method`).
    if path.is_ident("bee_method") {
        return true;
    }
    path.segments
        .last()
        .map(|s| s.ident == "bee_method")
        .unwrap_or(false)
}

fn extract_slot(attrs: &[syn::Attribute]) -> syn::Result<Option<String>> {
    for a in attrs {
        if is_bee_method_attr_path(a.path()) {
            let parsed: MethodArgs = a.parse_args()?;
            return Ok(Some(parsed.slot));
        }
    }
    Ok(None)
}

fn is_bee_method_attr(a: &syn::Attribute) -> bool {
    is_bee_method_attr_path(a.path())
}
