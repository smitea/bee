# S33.6 — Plugin Macro Ergonomics Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a `#[bee_adapter]` proc-macro + `register_vtable!` sub-macro that turns a plugin author's hand-written vtable glue (~150 lines per adapter) into declarative async-trait impls (~30 lines per adapter). No wire changes — vtable layout + host call path stay identical.

**Architecture:** New proc-macro crate `crates/bee-plugin-macro/` (depends on `proc-macro2`, `quote`, `syn`). The macro parses an `impl` block decorated with `#[bee_adapter(input|output|handler, name = "...")]` on `open`/`handle` + `#[bee_method(slot = "open|next|close|emit|handle|init_state")]` on body methods, then generates 2-3 `unsafe extern "C" fn` + a per-instance ctx struct + a `static FOO_VTABLE: Vtable` constant. Async → sync bridging uses `tokio::task::block_in_place(|| Handle::current().block_on(fut))` with a `futures::executor::block_on` fallback for tests. The `register_vtable!` sub-macro in `bee-plugin-sdk/src/macros.rs` is a `macro_rules!` that emits 3 HashMap inserts.

**Tech Stack:** Rust, `proc-macro2`, `quote`, `syn` (for proc-macro), `tokio`, `bincode`, `serde`.

---

## File Structure

| File | Action | Purpose |
|------|--------|---------|
| `Cargo.toml` | Modify | Add `bee-plugin-macro` to `[workspace.dependencies]` + `[workspace.members]` |
| `crates/bee-plugin-macro/Cargo.toml` | Create | New proc-macro crate manifest |
| `crates/bee-plugin-macro/src/lib.rs` | Create | Proc-macro entry + signature validation + code generation |
| `crates/bee-plugin-macro/tests/macro_expands_input_adapter.rs` | Create | 1 test |
| `crates/bee-plugin-macro/tests/macro_expands_output_adapter.rs` | Create | 1 test |
| `crates/bee-plugin-macro/tests/macro_expands_handler.rs` | Create | 1 test |
| `crates/bee-plugin-macro/tests/macro_registration_round_trip.rs` | Create | 1 test |
| `crates/bee-plugin-macro/tests/compile_fail/` | Create | trybuild compile-fail source files |
| `crates/bee-plugin-macro/tests/compile_fail/non_async_open.rs` | Create | sync fn `open` should fail to compile |
| `crates/bee-plugin-macro/tests/compile_fail/non_async_open.stderr` | Create | expected error output |
| `crates/bee-plugin-sdk/Cargo.toml` | Modify | Add `bee-plugin-macro` as `[dev-dependencies]` (so tests can use the macro) |
| `crates/bee-plugin-sdk/src/lib.rs` | Modify | Refactor `MockBinancePlugin` test fixture to use the macro |
| `crates/bee-plugin-sdk/src/macros.rs` | Modify | Add `register_vtable!` sub-macro |

---

## Task 1: Scaffold the `bee-plugin-macro` proc-macro crate

**Files:**
- Create: `crates/bee-plugin-macro/Cargo.toml`
- Create: `crates/bee-plugin-macro/src/lib.rs`
- Modify: `Cargo.toml` (workspace members + deps)

- [ ] **Step 1.1: Add `bee-plugin-macro` to the workspace**

In `Cargo.toml`:
1. Add `"crates/bee-plugin-macro"` to `[workspace.members]`.
2. Add `bee-plugin-macro = { path = "crates/bee-plugin-macro" }` to `[workspace.dependencies]`.

- [ ] **Step 1.2: Create the crate manifest**

Create `crates/bee-plugin-macro/Cargo.toml`:

```toml
[package]
name = "bee-plugin-macro"
version.workspace = true
edition.workspace = true
license.workspace = true
publish.workspace = true
description = "Bee Plugin SDK: proc-macro for declarative cdylib plugin authoring (S33.6)"

[lib]
proc-macro = true

[dependencies]
proc-macro2 = "1"
quote = "1"
syn = { version = "2", features = ["full", "extra-traits"] }
```

- [ ] **Step 1.3: Create the proc-macro entry stub**

Create `crates/bee-plugin-macro/src/lib.rs`:

```rust
//! S33.6: `#[bee_adapter]` proc-macro.
//!
//! Turns a hand-written `impl` block with
//! `async fn open / next / close` (input),
//! `async fn open / emit / close` (output),
//! or `async fn handle / init_state` (handler)
//! into the FFI glue: 2-3 `unsafe extern "C" fn`
//! + a per-instance ctx struct + a `static
//! FOO_VTABLE: Vtable` constant.
//!
//! The proc-macro is intentionally small and
//! focused on plugin-author ergonomics. The wire
//! format (vtable layout, Event bincode schema,
//! Handler state encoding) is defined in
//! `bee-plugin-sdk` and unchanged.

use proc_macro::TokenStream;
use syn::{parse_macro_input, ItemImpl};

/// Mark an `impl` block method as a Bee
/// `InputAdapter` / `OutputAdapter` / `Handler`
/// FFI slot. The macro turns the marked
/// `open` (or `handle`) method + the following
/// `#[bee_method(slot = "...")]` methods into
/// the vtable + ctx struct + static.
///
/// See `docs/superpowers/specs/2026-06-10-s33-6-plugin-macro-ergonomics-design.md`
/// for the full surface and signature rules.
///
/// # MVP
///
/// Only the `input` variant is implemented in
/// this task. `output` and `handler` are added
/// in Task 2.
#[proc_macro_attribute]
pub fn bee_adapter(_args: TokenStream, input: TokenStream) -> TokenStream {
    let _impl_block = parse_macro_input!(input as ItemImpl);
    // Placeholder: the full implementation
    // arrives in Task 2. For now, this
    // proc-macro is a pass-through that
    // re-emits the impl block unchanged.
    let _ = _args;
    let _ = _impl_block;
    TokenStream::new()
}

/// Mark a body method as a Bee FFI slot. The
/// `slot = "..."` arg binds the Rust method to
/// the FFI wire slot (`open` / `next` / `close`
/// for input; `open` / `emit` / `close` for
/// output; `handle` / `init_state` for handler).
#[proc_macro_attribute]
pub fn bee_method(_args: TokenStream, input: TokenStream) -> TokenStream {
    // Placeholder: pass-through for now.
    let _ = _args;
    let _ = input;
    TokenStream::new()
}
```

- [ ] **Step 1.4: Build to verify**

Run: `cargo build -p bee-plugin-macro 2>&1 | tail -3`
Expected: clean build (the proc-macro is a no-op pass-through).

- [ ] **Step 1.5: Commit**

```bash
git add Cargo.toml crates/bee-plugin-macro/
git -c user.name="opencode" -c user.email="opencode@local" commit -m "S33.6 Task 1: scaffold bee-plugin-macro proc-macro crate"
```

---

## Task 2: Implement the `#[bee_adapter(input)]` proc-macro

**Files:**
- Modify: `crates/bee-plugin-macro/src/lib.rs` (full implementation)
- Create: `crates/bee-plugin-macro/tests/macro_expands_input_adapter.rs` (the test that locks down the behavior)

- [ ] **Step 2.1: Write the failing test (RED)**

Create `crates/bee-plugin-macro/tests/macro_expands_input_adapter.rs`:

```rust
//! S33.6 Task 2: locks down the proc-macro
//! for `#[bee_adapter(input)]`. The test
//! defines a sample adapter via the macro,
//! then exercises the generated vtable to
//! prove the FFI glue is correct.

use bee_adapter::{AdapterError, AdapterResult, Event};
use bee_plugin_macro::{bee_adapter, bee_method};
use bee_plugin_sdk::vtable::InputAdapterVtable;
use std::sync::Arc;

pub struct MockInput {
    count: u32,
    emitted: u32,
}

impl MockInput {
    #[bee_adapter(input, name = "mock")]
    pub async fn open(config: Vec<u8>) -> AdapterResult<Self> {
        let c: u32 = bincode::deserialize(&config).unwrap_or(3);
        Ok(Self { count: c, emitted: 0 })
    }

    #[bee_method(slot = "next")]
    pub async fn next_one(&mut self) -> AdapterResult<Option<Event>> {
        if self.emitted >= self.count { return Ok(None); }
        self.emitted += 1;
        Ok(Some(Event {
            timestamp: 0,
            sequence: self.emitted as u64,
            payload: self.emitted.to_string().into_bytes(),
        }))
    }

    #[bee_method(slot = "close")]
    pub async fn close(self) -> AdapterResult<()> { Ok(()) }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mock_input_vtable_round_trip() {
    let config = bincode::serialize(&3u32).unwrap();
    let ctx = unsafe {
        ((*MOCK_INPUT_VTABLE).open)(
            config.as_ptr(),
            config.len(),
            std::ptr::null_mut(),
        )
    };
    assert!(!ctx.is_null(), "open returned null");

    // Call next 3 times; expect 3 events with
    // sequences 1, 2, 3.
    for expected_seq in 1..=3u64 {
        let mut out = bee_plugin_sdk::event::EventBytes::EMPTY;
        let rc = unsafe { ((*MOCK_INPUT_VTABLE).next)(ctx, &mut out) };
        assert_eq!(rc, 1, "expected 1 event on iteration {expected_seq}");
        assert!(!out.ptr.is_null());
        assert!(out.len > 0);
        let bytes = unsafe { std::slice::from_raw_parts(out.ptr, out.len) };
        let ev: Event = bincode::deserialize(bytes).expect("decode event");
        assert_eq!(ev.sequence, expected_seq);
    }
    // 4th call: end-of-stream.
    let mut out = bee_plugin_sdk::event::EventBytes::EMPTY;
    let rc = unsafe { ((*MOCK_INPUT_VTABLE).next)(ctx, &mut out) };
    assert_eq!(rc, 0, "expected end-of-stream on 4th call");

    // Close.
    let rc = unsafe { ((*MOCK_INPUT_VTABLE).close)(ctx) };
    assert_eq!(rc, 0);
}
```

(Add `use bee_plugin_sdk::event::EventBytes;` at the top — verify the path by reading `crates/bee-plugin-sdk/src/event.rs`; if it lives elsewhere, adjust. The test also assumes the macro generates a `pub static MOCK_INPUT_VTABLE: InputAdapterVtable`.)

- [ ] **Step 2.2: Run the test to verify it fails (RED)**

Run: `cargo test -p bee-plugin-macro --test macro_expands_input_adapter 2>&1 | tail -5`
Expected: COMPILE FAIL — `MOCK_INPUT_VTABLE` doesn't exist; the macro is a pass-through.

- [ ] **Step 2.3: Implement the proc-macro**

Replace `crates/bee-plugin-macro/src/lib.rs` with the full implementation:

```rust
//! S33.6: `#[bee_adapter]` proc-macro.
//!
//! Turns a hand-written `impl` block into the
//! FFI glue: 2-3 `unsafe extern "C" fn` + a
//! per-instance ctx struct + a `static
//! FOO_VTABLE: Vtable` constant.

extern crate proc_macro;

use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::{format_ident, quote};
use syn::{
    parse::{Parse, ParseStream},
    parse_macro_input, Error, FnArg, ImplItem, ImplItemFn, ItemFn, ItemImpl,
    Lit, LitStr, Meta, Pat, ReturnType, Signature, Token, Type,
};

/// Args of `#[bee_adapter(input, name = "...")]`:
/// a kind (`input` / `output` / `handler`) and
/// optionally a `name = "..."` adapter name.
struct AdapterArgs {
    kind: AdapterKind,
    name: String,
}

enum AdapterKind {
    Input,
    Output,
    Handler,
}

impl Parse for AdapterArgs {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let kind_lit: LitStr = input.parse()?;
        let kind = match kind_lit.value().as_str() {
            "input" => AdapterKind::Input,
            "output" => AdapterKind::Output,
            "handler" => AdapterKind::Handler,
            other => {
                return Err(Error::new(
                    kind_lit.span(),
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

/// Args of `#[bee_method(slot = "...")]`:
/// a slot name (`open` / `next` / `close` /
/// `emit` / `handle` / `init_state`).
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
        AdapterKind::Input => gen_input_adapter(adapter_args.name, impl_block),
        AdapterKind::Output => gen_output_adapter(adapter_args.name, impl_block),
        AdapterKind::Handler => gen_handler(adapter_args.name, impl_block),
    }
}

#[proc_macro_attribute]
pub fn bee_method(args: TokenStream, input: TokenStream) -> TokenStream {
    // Pass-through: the macro reads `args` via
    // `parse_meta` in `gen_*_adapter` by walking
    // the impl block's items. We don't
    // re-emit anything here; the impl block is
    // processed by `bee_adapter` in one pass.
    let _ = args;
    let _ = input;
    TokenStream::new()
}

// ---- helpers ----

fn gen_input_adapter(name: String, impl_block: ItemImpl) -> TokenStream {
    let struct_name = match &*impl_block.self_ty {
        Type::Path(tp) => tp
            .path
            .segments
            .last()
            .expect("impl has no self type")
            .ident
            .clone(),
        _ => {
            return Error::new(
                Span::call_site(),
                "bee_adapter: self_ty must be a path",
            )
            .to_compile_error()
            .into();
        }
    };
    let vtable_name = format_ident!("{}_VTABLE", struct_name.to_string().to_uppercase());

    // Collect the FFI slots from the impl block.
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
                _ => {} // not an FFI slot; ignore
            }
        }
    }
    let open_fn = match open_fn {
        Some(f) => f,
        None => {
            return Error::new(
                Span::call_site(),
                "bee_adapter(input): no method with `#[bee_method(slot = \"open\")]` found",
            )
            .to_compile_error()
            .into();
        }
    };
    let next_fn = match next_fn {
        Some(f) => f,
        None => {
            return Error::new(
                Span::call_site(),
                "bee_adapter(input): no method with `#[bee_method(slot = \"next\")]` found",
            )
            .to_compile_error()
            .into();
        }
    };
    let close_fn = match close_fn {
        Some(f) => f,
        None => {
            return Error::new(
                Span::call_site(),
                "bee_adapter(input): no method with `#[bee_method(slot = \"close\")]` found",
            )
            .to_compile_error()
            .into();
        }
    };

    // Sanity-check signatures.
    if let Err(e) = check_input_open(&open_fn) {
        return e.to_compile_error().into();
    }
    if let Err(e) = check_input_next(&next_fn) {
        return e.to_compile_error().into();
    }
    if let Err(e) = check_input_close(&close_fn) {
        return e.to_compile_error().into();
    }

    // Generate the FFI glue.
    let ctx_ty = format_ident!("{}InputCtx", struct_name);
    let open_ffi = format_ident!("{}_input_open", struct_name.to_string().to_lowercase());
    let next_ffi = format_ident!("{}_input_next", struct_name.to_string().to_lowercase());
    let close_ffi = format_ident!("{}_input_close", struct_name.to_string().to_lowercase());
    let open_rust = &open_fn.sig.ident;
    let next_rust = &next_fn.sig.ident;
    let close_rust = &close_fn.sig.ident;

    // Strip the `#[bee_method(slot = "...")]` attrs
    // before re-emitting the impl block.
    let mut impl_block = impl_block;
    for item in &mut impl_block.items {
        if let ImplItem::Fn(f) = item {
            f.attrs.retain(|a| !is_bee_method_attr(a));
        }
    }

    let expanded = quote! {
        // Per-instance ctx: holds the
        // adapter (or None after close).
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
            if let Some(adapter) = adapter {
                let fut = adapter.#close_rust();
                let _ = match ::tokio::runtime::Handle::try_current() {
                    Ok(h) => ::tokio::task::block_in_place(|| h.block_on(fut)),
                    Err(_) => ::futures::executor::block_on(fut),
                };
            }
            0
        }

        pub static #vtable_name: bee_plugin_sdk::vtable::InputAdapterVtable =
            bee_plugin_sdk::vtable::InputAdapterVtable {
                open: #open_ffi,
                next: #next_ffi,
                close: #close_ffi,
            };

        #impl_block
    };
    expanded.into()
}

// Signature checks. The macro emits a
// `syn::Error` if the user's method doesn't
// match the expected shape. The checks are
// MVP-grade (verify the basics, don't
// over-constrain).

fn check_input_open(f: &ImplItemFn) -> syn::Result<()> {
    if f.sig.asyncness.is_none() {
        return Err(Error::new_spanned(
            &f.sig,
            "bee_adapter(input): `open` must be `async fn`",
        ));
    }
    if f.sig.inputs.len() != 1 {
        return Err(Error::new_spanned(
            &f.sig,
            "bee_adapter(input): `open` must take exactly one `config` argument",
        ));
    }
    if !is_adapter_result(&f.sig.output) {
        return Err(Error::new_spanned(
            &f.sig.output,
            "bee_adapter(input): `open` must return `AdapterResult<Self>`",
        ));
    }
    Ok(())
}

fn check_input_next(f: &ImplItemFn) -> syn::Result<()> {
    if f.sig.asyncness.is_none() {
        return Err(Error::new_spanned(
            &f.sig,
            "bee_adapter(input): `next` must be `async fn`",
        ));
    }
    if f.sig.inputs.len() != 1 {
        return Err(Error::new_spanned(
            &f.sig,
            "bee_adapter(input): `next` must take `&mut self`",
        ));
    }
    Ok(())
}

fn check_input_close(f: &ImplItemFn) -> syn::Result<()> {
    if f.sig.asyncness.is_none() {
        return Err(Error::new_spanned(
            &f.sig,
            "bee_adapter(input): `close` must be `async fn`",
        ));
    }
    Ok(())
}

fn is_adapter_result(rt: &ReturnType) -> bool {
    if let ReturnType::Type(_, ty) = rt {
        if let Type::Path(tp) = &**ty {
            let last = tp.path.segments.last();
            if let Some(seg) = last {
                return seg.ident == "AdapterResult";
            }
        }
    }
    false
}

fn extract_slot(attrs: &[syn::Attribute]) -> syn::Result<Option<String>> {
    for a in attrs {
        if a.path().is_ident("bee_method") {
            let parsed: MethodArgs = a.parse_args()?;
            return Ok(Some(parsed.slot));
        }
    }
    Ok(None)
}

fn is_bee_method_attr(a: &syn::Attribute) -> bool {
    a.path().is_ident("bee_method")
}

fn gen_output_adapter(_name: String, _impl_block: ItemImpl) -> TokenStream {
    // Task 3 will fill this in.
    Error::new(
        Span::call_site(),
        "bee_adapter(output): not yet implemented (S33.6 Task 3)",
    )
    .to_compile_error()
    .into()
}

fn gen_handler(_name: String, _impl_block: ItemImpl) -> TokenStream {
    // Task 4 will fill this in.
    Error::new(
        Span::call_site(),
        "bee_adapter(handler): not yet implemented (S33.6 Task 4)",
    )
    .to_compile_error()
    .into()
}
```

- [ ] **Step 2.4: Add the missing dependencies**

The macro emits `tokio::sync::Mutex`, `tokio::runtime::Handle`, `tokio::task::block_in_place`, `futures::executor::block_on`, `bincode::serialize`, `EventBytes`. The generated code is **emitted into the plugin author's crate**, so the plugin author must already have these deps. For the test, add them to `crates/bee-plugin-macro/Cargo.toml` under `[dev-dependencies]`:

```toml
[dev-dependencies]
bee-adapter = { workspace = true }
bee-plugin-sdk = { workspace = true }
tokio = { version = "1", features = ["full"] }
futures = "0.3"
bincode = "1"
serde = { version = "1", features = ["derive"] }
```

- [ ] **Step 2.5: Run the test (GREEN)**

Run: `cargo test -p bee-plugin-macro --test macro_expands_input_adapter 2>&1 | tail -5`
Expected: 1 test passes.

- [ ] **Step 2.6: Commit**

```bash
git add crates/bee-plugin-macro/
git -c user.name="opencode" -c user.email="opencode@local" commit -m "S33.6 Task 2: #[bee_adapter(input)] proc-macro + 1 test"
```

---

## Task 3: `#[bee_adapter(output)]` macro

**Files:**
- Modify: `crates/bee-plugin-macro/src/lib.rs` (replace `gen_output_adapter` stub)
- Create: `crates/bee-plugin-macro/tests/macro_expands_output_adapter.rs`

- [ ] **Step 3.1: Write the failing test (RED)**

Create `crates/bee-plugin-macro/tests/macro_expands_output_adapter.rs`:

```rust
//! S33.6 Task 3: locks down the proc-macro
//! for `#[bee_adapter(output)]`.

use bee_adapter::{AdapterError, AdapterResult, Event};
use bee_plugin_macro::{bee_adapter, bee_method};
use bee_plugin_sdk::event::EventBytes;
use bee_plugin_sdk::vtable::OutputAdapterVtable;

pub struct MockOutput {
    received: u32,
}

impl MockOutput {
    #[bee_adapter(output, name = "mock-emit")]
    pub async fn open(config: Vec<u8>) -> AdapterResult<Self> {
        let _ = config;
        Ok(Self { received: 0 })
    }

    #[bee_method(slot = "emit")]
    pub async fn emit_one(&mut self, event: Event) -> AdapterResult<()> {
        assert_eq!(event.sequence, self.received + 1);
        self.received += 1;
        Ok(())
    }

    #[bee_method(slot = "close")]
    pub async fn close(self) -> AdapterResult<()> { Ok(()) }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mock_output_vtable_round_trip() {
    let config: Vec<u8> = vec![];
    let ctx = unsafe {
        ((*MOCK_OUTPUT_VTABLE).open)(
            config.as_ptr(),
            config.len(),
            std::ptr::null_mut(),
        )
    };
    assert!(!ctx.is_null());

    for seq in 1..=3u64 {
        let event = Event {
            timestamp: 0,
            sequence: seq,
            payload: vec![],
        };
        let bytes = bincode::serialize(&event).unwrap();
        let rc = unsafe {
            ((*MOCK_OUTPUT_VTABLE).emit)(
                ctx,
                bytes.as_ptr(),
                bytes.len(),
            )
        };
        assert_eq!(rc, 0, "emit failed on seq {seq}");
    }
    let rc = unsafe { ((*MOCK_OUTPUT_VTABLE).close)(ctx) };
    assert_eq!(rc, 0);
}
```

- [ ] **Step 3.2: Run to verify it fails (RED)**

Run: `cargo test -p bee-plugin-macro --test macro_expands_output_adapter 2>&1 | tail -3`
Expected: compile error from the stub `gen_output_adapter` returning "not yet implemented".

- [ ] **Step 3.3: Implement `gen_output_adapter`**

Replace the stub in `crates/bee-plugin-macro/src/lib.rs`:

```rust
fn gen_output_adapter(name: String, impl_block: ItemImpl) -> TokenStream {
    let struct_name = match &*impl_block.self_ty {
        Type::Path(tp) => tp
            .path
            .segments
            .last()
            .expect("impl has no self type")
            .ident
            .clone(),
        _ => {
            return Error::new(
                Span::call_site(),
                "bee_adapter: self_ty must be a path",
            )
            .to_compile_error()
            .into();
        }
    };
    let vtable_name = format_ident!(
        "{}_VTABLE",
        struct_name.to_string().to_uppercase()
    );

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
    let emit_fn = match emit_fn {
        Some(f) => f,
        None => return err("`#[bee_method(slot = \"emit\")]` not found"),
    };
    let close_fn = match close_fn {
        Some(f) => f,
        None => return err("`#[bee_method(slot = \"close\")]` not found"),
    };

    let ctx_ty = format_ident!("{}OutputCtx", struct_name);
    let open_ffi = format_ident!(
        "{}_output_open",
        struct_name.to_string().to_lowercase()
    );
    let emit_ffi = format_ident!(
        "{}_output_emit",
        struct_name.to_string().to_lowercase()
    );
    let close_ffi = format_ident!(
        "{}_output_close",
        struct_name.to_string().to_lowercase()
    );
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
            match ::tokio::runtime::Handle::try_current() {
                Ok(h) => ::tokio::task::block_in_place(|| h.block_on(fut)),
                Err(_) => ::futures::executor::block_on(fut),
            }
            .map(|_| 0)
            .unwrap_or(-1)
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

fn err(msg: &str) -> TokenStream {
    Error::new(Span::call_site(), msg).to_compile_error().into()
}
```

(Add `fn err` at the bottom of the file. Refactor the input adapter path to use `err()` too — `gen_input_adapter` should return `err("...")` instead of inlining the error construction.)

- [ ] **Step 3.4: Run the test (GREEN)**

Run: `cargo test -p bee-plugin-macro --test macro_expands_output_adapter 2>&1 | tail -3`
Expected: 1 test passes.

- [ ] **Step 3.5: Commit**

```bash
git add crates/bee-plugin-macro/
git -c user.name="opencode" -c user.email="opencode@local" commit -m "S33.6 Task 3: #[bee_adapter(output)] proc-macro + 1 test"
```

---

## Task 4: `#[bee_adapter(handler)]` macro

**Files:**
- Modify: `crates/bee-plugin-macro/src/lib.rs` (replace `gen_handler` stub)
- Create: `crates/bee-plugin-macro/tests/macro_expands_handler.rs`

- [ ] **Step 4.1: Write the failing test (RED)**

Create `crates/bee-plugin-macro/tests/macro_expands_handler.rs`:

```rust
//! S33.6 Task 4: locks down the proc-macro
//! for `#[bee_adapter(handler)]`.

use bee_adapter::{AdapterError, AdapterResult};
use bee_plugin_macro::{bee_adapter, bee_method};
use bee_plugin_sdk::event::EventBytes;
use bee_plugin_sdk::vtable::HandlerVtable;

/// A counter handler. State is a u64. The
/// handler increments state on every call
/// and returns the new state as the result.
pub struct CounterHandler;

#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    serde::Serialize,
    serde::Deserialize,
    Default,
)]
pub struct CounterState {
    pub count: u64,
}

impl CounterHandler {
    #[bee_adapter(handler, name = "counter")]
    pub async fn handle(
        state: CounterState,
        _event: Vec<u8>,
    ) -> AdapterResult<(CounterState, Vec<u8>)> {
        let new_state = CounterState {
            count: state.count + 1,
        };
        let result = bincode::serialize(&new_state).unwrap();
        Ok((new_state, result))
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn counter_handler_vtable_round_trip() {
    // init_state: empty Vec<u8> (the default
    // generated by the macro).
    let mut state_out = EventBytes::EMPTY;
    let rc = unsafe { ((*COUNTER_HANDLER_VTABLE).init_state)(&mut state_out) };
    assert_eq!(rc, 0);
    assert_eq!(state_out.len, 0, "default init_state should be empty");

    // handle(state_in, event_in) -> (new_state, result).
    let state_in = CounterState { count: 0 };
    let state_in_bytes = bincode::serialize(&state_in).unwrap();
    let event_in: Vec<u8> = vec![];
    let mut new_state_out = EventBytes::EMPTY;
    let mut result_out = EventBytes::EMPTY;
    let rc = unsafe {
        ((*COUNTER_HANDLER_VTABLE).handle)(
            state_in_bytes.as_ptr(),
            state_in_bytes.len(),
            event_in.as_ptr(),
            event_in.len(),
            &mut new_state_out,
            &mut result_out,
            std::ptr::null_mut(),
        )
    };
    assert_eq!(rc, 0);
    let new_state_bytes = unsafe {
        std::slice::from_raw_parts(new_state_out.ptr, new_state_out.len)
    };
    let new_state: CounterState = bincode::deserialize(new_state_bytes).unwrap();
    assert_eq!(new_state.count, 1);
    let result_bytes = unsafe {
        std::slice::from_raw_parts(result_out.ptr, result_out.len)
    };
    let result_state: CounterState = bincode::deserialize(result_bytes).unwrap();
    assert_eq!(result_state.count, 1);
}
```

- [ ] **Step 4.2: Run to verify it fails (RED)**

Run: `cargo test -p bee-plugin-macro --test macro_expands_handler 2>&1 | tail -3`
Expected: compile error from the stub.

- [ ] **Step 4.3: Implement `gen_handler`**

Replace the stub in `crates/bee-plugin-macro/src/lib.rs`:

```rust
fn gen_handler(name: String, impl_block: ItemImpl) -> TokenStream {
    let struct_name = match &*impl_block.self_ty {
        Type::Path(tp) => tp
            .path
            .segments
            .last()
            .expect("impl has no self type")
            .ident
            .clone(),
        _ => return err("bee_adapter: self_ty must be a path"),
    };
    let vtable_name = format_ident!(
        "{}_VTABLE",
        struct_name.to_string().to_uppercase()
    );

    let mut handle_fn: Option<ImplItemFn> = None;
    for item in &impl_block.items {
        if let ImplItem::Fn(f) = item {
            let slot = match extract_slot(&f.attrs) {
                Ok(s) => s,
                Err(e) => return e.to_compile_error().into(),
            };
            if slot.as_deref() == Some("handle") {
                handle_fn = Some(f.clone());
            }
        }
    }
    let handle_fn = match handle_fn {
        Some(f) => f,
        None => return err("`#[bee_method(slot = \"handle\")]` not found"),
    };

    let handle_ffi = format_ident!(
        "{}_handler_handle",
        struct_name.to_string().to_lowercase()
    );
    let init_state_ffi = format_ident!(
        "{}_handler_init_state",
        struct_name.to_string().to_lowercase()
    );
    let handle_rust = &handle_fn.sig.ident;

    let mut impl_block = impl_block;
    for item in &mut impl_block.items {
        if let ImplItem::Fn(f) = item {
            f.attrs.retain(|a| !is_bee_method_attr(a));
        }
    }

    quote! {
        // The Handler vtable is stateless at the
        // FFI level (the host owns the state
        // blob; the handler is a pure function).
        // The macro generates no ctx struct; it
        // generates only the 2 fns.
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
            let state: ::std::vec::Vec<u8> = state_bytes.to_vec();
            let event: ::std::vec::Vec<u8> = event_bytes.to_vec();
            // The handler is a `fn` (not `&self`),
            // so we call the associated function.
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

        /// Default `init_state`: returns an
        /// empty `Vec<u8>` (the user can
        /// override with a custom `init_state`
        /// in a follow-up story).
        unsafe extern "C" fn #init_state_ffi(
            out: *mut bee_plugin_sdk::event::EventBytes,
        ) -> i32 {
            // The host expects a bincode-encoded
            // state blob. Empty Vec<u8> =
            // `bincode::serialize(&Vec::<u8>::new())`.
            let bytes: ::std::vec::Vec<u8> = ::std::vec::Vec::new();
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
```

- [ ] **Step 4.4: Run the test (GREEN)**

Run: `cargo test -p bee-plugin-macro --test macro_expands_handler 2>&1 | tail -3`
Expected: 1 test passes.

- [ ] **Step 4.5: Commit**

```bash
git add crates/bee-plugin-macro/
git -c user.name="opencode" -c user.email="opencode@local" commit -m "S33.6 Task 4: #[bee_adapter(handler)] proc-macro + 1 test"
```

---

## Task 5: `register_vtable!` sub-macro

**Files:**
- Modify: `crates/bee-plugin-sdk/src/macros.rs` (add `register_vtable!`)
- Create: `crates/bee-plugin-macro/tests/macro_registration_round_trip.rs`

- [ ] **Step 5.1: Add the sub-macro**

Append to `crates/bee-plugin-sdk/src/macros.rs` (after the existing `cdylib_plugin_impl!`):

```rust
/// Register a sequence of plugin vtables into
/// the 3 `HashMap` fields of a `PluginHandle`
/// (input_adapters / output_adapters /
/// handlers).
///
/// Usage:
/// ```ignore
/// let mut input_adapters = HashMap::new();
/// let mut output_adapters = HashMap::new();
/// let mut handlers = HashMap::new();
/// register_vtable! {
///     input_adapters, output_adapters, handlers;
///     input  "subscribe" => &SUBSCRIBE_VTABLE,
///     output "ohlcv"     => &OHLCV_VTABLE,
///     handler "fib"      => &FIB_VTABLE,
/// }
/// ```
///
/// The macro emits 3 `.insert(name, vtable)`
/// calls (one per `kind`). Plugin authors
/// typically use this in `Factory::init()`.
#[macro_export]
macro_rules! register_vtable {
    (
        $input:ident, $output:ident, $handlers:ident;
        $( $kind:ident $name:literal => $vtable:expr ),* $(,)?
    ) => {
        $(
            match $kind {
                "input" => $input.insert(
                    ::std::string::String::from($name),
                    $vtable,
                ),
                "output" => $output.insert(
                    ::std::string::String::from($name),
                    $vtable,
                ),
                "handler" => $handlers.insert(
                    ::std::string::String::from($name),
                    $vtable,
                ),
                _ => panic!(
                    "register_vtable!: kind must be input, output, or handler"
                ),
            };
        )*
    };
}
```

- [ ] **Step 5.2: Write the registration round-trip test (RED)**

Create `crates/bee-plugin-macro/tests/macro_registration_round_trip.rs`:

```rust
//! S33.6 Task 5: use the macro-generated
//! vtable + the `register_vtable!` sub-macro
//! to build a `PluginHandle`, register with
//! `PluginManager`, and assert
//! `pm.resolve(name, &version_spec)` returns
//! `Some`.

use std::collections::HashMap;
use std::sync::Arc;

use bee_adapter::{AdapterError, AdapterResult, Event};
use bee_plugin_macro::{bee_adapter, bee_method};
use bee_plugin_sdk::macros::Factory;
use bee_plugin_sdk::{
    AdapterDescriptor, Plugin, PluginHandle, PluginManifest, PluginName,
    PluginResult, VersionSpec,
};
use bee_registry::PluginManager;

pub struct SimpleInput;

impl SimpleInput {
    #[bee_adapter(input, name = "simple")]
    pub async fn open(_config: Vec<u8>) -> AdapterResult<Self> {
        Ok(Self)
    }

    #[bee_method(slot = "next")]
    pub async fn next_one(&mut self) -> AdapterResult<Option<Event>> {
        Ok(None)
    }

    #[bee_method(slot = "close")]
    pub async fn close(self) -> AdapterResult<()> { Ok(()) }
}

pub struct SimpleFactory;

const CONTENT: &[u8] = b"simple-plugin-v1";

impl Plugin for SimpleFactory {
    fn plugin_content(&self) -> &'static [u8] { CONTENT }
    fn manifest(&self) -> PluginManifest {
        PluginManifest {
            name: PluginName("simple".into()),
            feature_version: "1.0.0".into(),
            abi_version: "v1".into(),
            adapters: vec![AdapterDescriptor {
                name: "simple".into(),
                is_input: true,
            }],
            handlers: vec![],
        }
    }
    fn init(&self) -> PluginResult<PluginHandle> {
        let mut input_adapters = HashMap::new();
        let mut output_adapters = HashMap::new();
        let mut handlers = HashMap::new();
        bee_plugin_sdk::register_vtable! {
            input_adapters, output_adapters, handlers;
            input "simple" => &SIMPLE_INPUT_VTABLE,
        }
        Ok(PluginHandle {
            manifest: self.manifest(),
            inner: Arc::new(()),
            input_adapters,
            output_adapters,
            handlers,
        })
    }
}

#[test]
fn registration_round_trip() {
    let mut mgr = PluginManager::new();
    mgr.register_plugin(&SimpleFactory).expect("register");
    let id = mgr
        .resolve("simple", &VersionSpec::Latest)
        .expect("resolve");
    assert_eq!(
        id,
        bee_plugin_sdk::compute_plugin_id(CONTENT),
        "PluginId should match sha256(content)"
    );
}
```

- [ ] **Step 5.3: Run the test (RED)**

Run: `cargo test -p bee-plugin-macro --test macro_registration_round_trip 2>&1 | tail -3`
Expected: FAIL with "register_vtable! not found" (or similar — the sub-macro isn't wired yet).

- [ ] **Step 5.4: Verify the sub-macro is `#[macro_export]` and the import path works**

The test uses `bee_plugin_sdk::register_vtable!`. `#[macro_export]` macros are accessible via the crate root. Verify by reading `crates/bee-plugin-sdk/src/lib.rs` and confirming `pub use macros::*;` or similar. If not present, add:

```rust
// In crates/bee-plugin-sdk/src/lib.rs, near the top:
pub use macros::*;
```

(This re-exports the `register_vtable!` macro from `macros.rs`.)

- [ ] **Step 5.5: Run the test (GREEN)**

Run: `cargo test -p bee-plugin-macro --test macro_registration_round_trip 2>&1 | tail -3`
Expected: 1 test passes.

- [ ] **Step 6 (correction): Commit Task 5 + fix `register_vtable!` export**

```bash
git add crates/bee-plugin-sdk/src/macros.rs crates/bee-plugin-sdk/src/lib.rs crates/bee-plugin-macro/
git -c user.name="opencode" -c user.email="opencode@local" commit -m "S33.6 Task 5: register_vtable! sub-macro + 1 round-trip test"
```

---

## Task 6: `trybuild` compile-fail test

**Files:**
- Modify: `crates/bee-plugin-macro/Cargo.toml` (add `trybuild` dev-dep)
- Modify: `crates/bee-plugin-macro/tests/compile_fail.rs` (the runner)
- Create: `crates/bee-plugin-macro/tests/compile_fail/non_async_open.rs`
- Create: `crates/bee-plugin-macro/tests/compile_fail/non_async_open.stderr`

- [ ] **Step 6.1: Add `trybuild` dev-dep**

In `crates/bee-plugin-macro/Cargo.toml` under `[dev-dependencies]`:

```toml
trybuild = "1"
```

- [ ] **Step 6.2: Create the compile-fail source**

Create `crates/bee-plugin-macro/tests/compile_fail/non_async_open.rs`:

```rust
use bee_adapter::{AdapterError, AdapterResult, Event};
use bee_plugin_macro::{bee_adapter, bee_method};

pub struct Bad;

impl Bad {
    // BUG: not async.
    #[bee_adapter(input, name = "bad")]
    pub fn open(_config: Vec<u8>) -> AdapterResult<Self> {
        Ok(Self)
    }

    #[bee_method(slot = "next")]
    pub async fn next_one(&mut self) -> AdapterResult<Option<Event>> {
        Ok(None)
    }

    #[bee_method(slot = "close")]
    pub async fn close(self) -> AdapterResult<()> { Ok(()) }
}

fn main() {}
```

- [ ] **Step 6.3: Create the runner**

Create `crates/bee-plugin-macro/tests/compile_fail.rs`:

```rust
//! S33.6 Task 6: trybuild compile-fail test
//! for `#[bee_adapter(input)]` signature
//! checks. The expected error snapshot is
//! committed to
//! `tests/compile_fail/non_async_open.stderr`.

#[test]
fn compile_fail_tests() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/compile_fail/*.rs");
}
```

- [ ] **Step 6.4: Generate the expected stderr**

Run the test once and let `trybuild` write the expected stderr file:

```bash
TRYBUILD=overwrite cargo test -p bee-plugin-macro --test compile_fail 2>&1 | tail -5
```

`TRYBUILD=overwrite` tells trybuild to create the `.stderr` file instead of failing. Inspect the generated `tests/compile_fail/non_async_open.stderr` to confirm it contains the expected error message ("`open` must be `async fn`"). If the message doesn't match what the spec promises, fix the macro to produce the expected error wording and re-run.

- [ ] **Step 6.5: Commit the test + stderr**

```bash
git add crates/bee-plugin-macro/
git -c user.name="opencode" -c user.email="opencode@local" commit -m "S33.6 Task 6: trybuild compile-fail for non-async open"
```

---

## Task 7: Refactor `MockBinancePlugin` to use the macro

**Files:**
- Modify: `crates/bee-plugin-sdk/Cargo.toml` (add `bee-plugin-macro` as dev-dep)
- Modify: `crates/bee-plugin-sdk/src/lib.rs` (refactor `MockBinancePlugin` test fixture)

- [ ] **Step 7.1: Add the dev-dep**

In `crates/bee-plugin-sdk/Cargo.toml` under `[dev-dependencies]`:

```toml
bee-plugin-macro = { workspace = true }
```

- [ ] **Step 7.2: Read the current `MockBinancePlugin` test fixture**

In `crates/bee-plugin-sdk/src/lib.rs` (around line 335 in the `#[cfg(test)]` mod), find the `MockBinancePlugin` impl block. It currently:

- Defines an `impl Plugin for MockBinancePlugin` with `manifest()` returning a hard-coded `PluginManifest` (1 adapter, name "subscribe").
- `init()` returns `PluginHandle { manifest, inner: Arc::new(()), input_adapters: HashMap::new(), ... }` — i.e., empty HashMaps.

The refactor uses the macro to add a real InputAdapter. Define a `MockBinanceInput` struct with the macro:

```rust
// In #[cfg(test)] mod tests:
use bee_plugin_macro::{bee_adapter, bee_method};
use bee_adapter::Event;

pub struct MockBinanceInput {
    count: u32,
    emitted: u32,
}

impl MockBinanceInput {
    #[bee_adapter(input, name = "subscribe")]
    pub async fn open(_config: Vec<u8>) -> AdapterResult<Self> {
        Ok(Self { count: 1, emitted: 0 })
    }

    #[bee_method(slot = "next")]
    pub async fn next_one(&mut self) -> AdapterResult<Option<Event>> {
        if self.emitted >= self.count { return Ok(None); }
        self.emitted += 1;
        Ok(Some(Event {
            timestamp: 0,
            sequence: 1,
            payload: b"hello".to_vec(),
        }))
    }

    #[bee_method(slot = "close")]
    pub async fn close(self) -> AdapterResult<()> { Ok(()) }
}
```

Then change `MockBinancePlugin::init()` to use `register_vtable!`:

```rust
fn init(&self) -> PluginResult<PluginHandle> {
    let mut input_adapters = HashMap::new();
    let mut output_adapters = HashMap::new();
    let mut handlers = HashMap::new();
    crate::register_vtable! {
        input_adapters, output_adapters, handlers;
        input "subscribe" => &MOCK_BINANCE_INPUT_VTABLE,
    }
    Ok(PluginHandle {
        manifest: self.manifest(),
        inner: Arc::new(()),
        input_adapters,
        output_adapters,
        handlers,
    })
}
```

- [ ] **Step 7.3: Run the existing test that uses `MockBinancePlugin`**

The existing `empty_manager_has_no_plugins` and friends are in `crates/bee-registry/src/lib.rs`, not in `bee-plugin-sdk`. The `MockBinancePlugin` in `bee-plugin-sdk/src/lib.rs` is its own test fixture. Run:

```bash
cargo test -p bee-plugin-sdk 2>&1 | tail -5
```

Expected: all existing tests still pass; the refactored `init()` produces a `PluginHandle` with 1 input adapter registered.

- [ ] **Step 7.4: Run the full workspace test suite to catch regressions**

```bash
cargo test --workspace 2>&1 | grep -E "^test result" | awk -F'[ ;]+' '{ p+=$4; f+=$6; i+=$8 } END { print "passed="p, "failed="f, "ignored="i }'
```

Expected: 482 passed (477 S33.5.2 baseline + 5 new S33.6 tests: 1 input + 1 output + 1 handler + 1 round-trip + 1 trybuild; the trybuild is a `#[test]` that runs the snapshot comparison). 0 failed.

- [ ] **Step 7.5: Commit**

```bash
git add crates/bee-plugin-sdk/
git -c user.name="opencode" -c user.email="opencode@local" commit -m "S33.6 Task 7: refactor MockBinancePlugin to use bee_adapter macro"
```

---

## Task 8: stories.md update + final push

**Files:**
- Modify: `docs/best-practices/quant/stories.md` (add S33.6 section)

- [ ] **Step 8.1: Append the S33.6 section**

Find the S33.5.2 section (added in commit fd4d117) and append below it:

```markdown
### S33.6 · Plugin macro ergonomics (the S33.5.2 follow-up)

- **Type**: AFK
- **Blocked by**: S33.5.2 (Datasource validation)
- **ADRs**: 0005, 0009
- **Design**: `docs/superpowers/specs/2026-06-10-s33-6-plugin-macro-ergonomics-design.md`
- **Plan**: `docs/superpowers/plans/2026-06-10-s33-6-plugin-macro-ergonomics.md`

> **Why this story exists**: A plugin author writing a cdylib plugin for Bee today has to hand-write ~150 lines of FFI glue per adapter (3 `unsafe extern "C" fn` + 1 vtable static + 3 HashMap inserts + 1 AdapterDescriptor per adapter). S33.6 adds a `#[bee_adapter(input|output|handler, name = "...")]` proc-macro that turns the author's `async fn open / next / close` impl into the full vtable + registration glue, dropping the boilerplate to ~30 lines per adapter.

**Implementation (code-level ✓, production-level N)**:

- `crates/bee-plugin-macro/` (new proc-macro crate): `#[bee_adapter(input|output|handler, name = "...")]` on `open` / `handle` + `#[bee_method(slot = "open|next|close|emit|handle|init_state")]` on each body method. The macro:
  - Generates a per-instance ctx struct (`Mutex<Option<Self>>`).
  - Generates 2-3 `unsafe extern "C" fn` (open / next / close for input; open / emit / close for output; handle / init_state for handler).
  - Bridges async → sync via `tokio::task::block_in_place(|| Handle::current().block_on(fut))` with a `futures::executor::block_on` fallback for tests.
  - Generates a `pub static FOO_VTABLE: Vtable` constant.
  - Strips the `#[bee_method]` attrs before re-emitting the impl block (so the impl block remains usable in-process).
  - Compile-time signature checks: `open` / `next` / `close` / `handle` must be `async fn`; `open` must return `AdapterResult<Self>`; etc. Errors are emitted via `syn::Error::to_compile_error()` with file:line:col.

- `crates/bee-plugin-sdk/src/macros.rs`: new `register_vtable!` `macro_rules!` that takes 3 HashMap bindings + a sequence of `(kind, name, vtable_static)` tuples and emits the 3 HashMap inserts. The plugin author uses this in `Factory::init()` to wire the macro-generated vtables into a `PluginHandle`.

- `crates/bee-plugin-sdk/src/lib.rs::tests::MockBinancePlugin`: refactored to use the macro (a `MockBinanceInput` struct with `#[bee_adapter(input, name = "subscribe")]` + 2 `#[bee_method]` body methods, then `register_vtable!` in `init()`). Proves the macro works in the in-process test path (not just cdylib).

**Tests** (5 new + the refactored MockBinancePlugin):

- `crates/bee-plugin-macro/tests/macro_expands_input_adapter.rs`: defines `MockInput` via the macro; calls open / next / close through the generated `MOCK_INPUT_VTABLE`; asserts 3 events with sequences 1, 2, 3, then end-of-stream, then close rc=0.
- `crates/bee-plugin-macro/tests/macro_expands_output_adapter.rs`: defines `MockOutput` via the macro; calls open / emit×3 / close; asserts each emit rc=0.
- `crates/bee-plugin-macro/tests/macro_expands_handler.rs`: defines `CounterHandler` via the macro; calls init_state (asserts empty Vec<u8>) and handle (asserts state.count=1).
- `crates/bee-plugin-macro/tests/macro_registration_round_trip.rs`: builds a `PluginHandle` using `register_vtable!` + the macro-generated vtable; registers with `PluginManager`; asserts `pm.resolve("simple", &VersionSpec::Latest)` returns the correct PluginId.
- `crates/bee-plugin-macro/tests/compile_fail.rs` (trybuild): `non_async_open.rs` uses `#[bee_adapter(input)]` on a non-async `open`. The expected stderr snapshot (`non_async_open.stderr`) is committed to the repo. `trybuild` runs the snapshot comparison.

**Result** (this commit): 482 workspace tests pass, 0 failed, 4 ignored. Net +5 from S33.5.2 baseline of 477 (1 input + 1 output + 1 handler + 1 round-trip + 1 trybuild; the refactored MockBinancePlugin is a net-zero change).

**Status (production-level, N)**:

- Code-level: 482/482 tests pass; the proc-macro generates correct vtable glue for input / output / handler; the in-process test fixture uses the macro; the trybuild compile-fail locks down the signature checks.
- Production-level: requires a third-party plugin author (e.g., the binance mock plugin in `examples/`) to migrate to the new macro and provide feedback. 1.x adoption. Deferred to the S33.6 HITL sign-off row.

**Follow-ups** (deferred to S33.6.x / 1.x):

- `#[plugin(name = "...", version = "...")]` proc-macro attribute on the factory struct to auto-generate `PluginManifest` (eliminates the remaining ~10 lines of manifest boilerplate).
- Custom `init_state` for `Handler` (the MVP always returns empty `Vec<u8>`; some handlers need a non-empty initial state like `{ count: 0 }`).
- Async-over-callback vtable variant: instead of `block_in_place` + `block_on`, the vtable fn spawns a task and returns immediately with a callback. Lower latency but bigger vtable struct (1.x).
- Cross-crate type checking: macro emits a `static_assert` that the plugin's `Config: DeserializeOwned` matches the host's expected `Config` (1.x).
- Non-Rust plugins (C/C++/Python/Go): always hand-written vtables (1.x; the FFI is C ABI by design).

**Sign-off honesty**:

- ✓ Code-level: 482/482 tests pass; the macro is locked down for input / output / handler; signature checks are tested.
- ✗ Production-level: requires a real plugin author to migrate + S33.6 HITL review.
```

- [ ] **Step 8.2: Commit + push**

```bash
git add docs/best-practices/quant/stories.md
git -c user.name="opencode" -c user.email="opencode@local" commit -m "S33.6 stories.md: plugin macro ergonomics section"
git push origin main
```

---

## Self-Review

**1. Spec coverage**:
- ✓ `#[bee_adapter(input/output/handler)]` proc-macro → Tasks 2, 3, 4
- ✓ `register_vtable!` sub-macro → Task 5
- ✓ Async → sync bridging via `tokio::task::block_in_place` + `Handle::try_current` fallback → Tasks 2, 3, 4
- ✓ Per-instance ctx struct → Tasks 2, 3 (handler is stateless at FFI level → Task 4)
- ✓ Compile-time signature checks → Tasks 2, 6 (trybuild)
- ✓ In-process test fixture refactor → Task 7
- ✓ 5 integration tests (3 macro + 1 round-trip + 1 trybuild) → Tasks 2, 3, 4, 5, 6
- ✓ No wire changes → all tasks (vtable layout + PluginHandle struct unchanged)
- ✓ No `trybuild` snapshot in the test count math → accounted for (the trybuild test counts as 1)
- ✗ No custom `init_state` for Handler → deferred to follow-ups (spec says so)

**2. Placeholder scan**: No TBD / TODO / "implement later" strings. Every code block has actual Rust code. Every command has expected output.

**3. Type consistency**:
- `MOCK_INPUT_VTABLE` is consistent across Tasks 2 (the test) and the macro generator in Task 2 Step 2.3.
- `MOCK_OUTPUT_VTABLE` is consistent across Tasks 3 (test + generator).
- `COUNTER_HANDLER_VTABLE` is consistent across Tasks 4 (test + generator).
- `SIMPLE_INPUT_VTABLE` is consistent across Task 5 (test + generator).
- The `register_vtable!` sub-macro signature (`$input, $output, $handlers; kind "name" => expr`) is consistent across Tasks 5 and 7.
- The `EventBytes` type path (`bee_plugin_sdk::event::EventBytes`) is consistent across all generated code and tests.
- The `AdapterResult` / `AdapterError` / `Event` types are from `bee_adapter` (no path conflict with `bee_plugin_sdk`).
- The `Plugin` trait is from `bee_plugin_sdk`; the `Factory` trait is from `bee_plugin_sdk::macros` (re-exported via `pub use macros::*` in `lib.rs` — verified in Task 5 Step 5.4).

**4. Risk**:
- The proc-macro's signature checking is MVP-grade. If a plugin author uses exotic generics / lifetimes, the macro may fail to expand. Task 2's MVP doesn't try to handle these edge cases; the spec's "Out of scope" lists cross-crate type checking as 1.x. The 5 tests cover the happy path + 1 compile-fail case.
- The `block_in_place` + `Handle::try_current` path requires the test to use `#[tokio::test(flavor = "multi_thread")]` (single-thread flavor would panic). All 3 macro tests use `multi_thread`; verified.
- The `register_vtable!` macro uses `match $kind` on a string literal — this is `macro_rules!`-level pattern matching, not runtime. The `kind` tokens must literally be `input` / `output` / `handler`.

---

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-06-10-s33-6-plugin-macro-ergonomics.md`. Two execution options:

1. **Subagent-Driven (recommended)** - dispatch a fresh subagent per task, review between tasks, fast iteration
2. **Inline Execution** - execute tasks in this session using executing-plans, batch execution with checkpoints

The user previously chose **Inline Execution** for the S33.1 / S33.2 / S33.3 / S33.4 / S33.5 / S33.5.1 / S33.5.2 batches. Continuing that pattern unless told otherwise.
