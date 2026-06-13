//! S33.6: `#[bee_adapter]` proc-macro.
//!
//! MVP stub: pass-through. Tasks 2-4 implement
//! the input / output / handler variants.

use proc_macro::TokenStream;
use syn::{parse_macro_input, ItemImpl};

#[proc_macro_attribute]
pub fn bee_adapter(_args: TokenStream, input: TokenStream) -> TokenStream {
    let _impl_block = parse_macro_input!(input as ItemImpl);
    let _ = _args;
    let _ = _impl_block;
    TokenStream::new()
}

#[proc_macro_attribute]
pub fn bee_method(_args: TokenStream, input: TokenStream) -> TokenStream {
    let _ = _args;
    let _ = input;
    TokenStream::new()
}
