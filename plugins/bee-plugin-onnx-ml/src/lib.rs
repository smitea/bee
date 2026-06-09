//! `bee-plugin-onnx-ml` — production-grade ONNX ML model Handlers (S39).
//!
//! S39 is the second **Handler** plugin (after S38 ta-indicators).
//! Handler plugins register SQL UDFs; the host calls
//! `handle(state, event) -> (new_state, result)` via the `HandlerVtable`.
//!
//! This file is the **scaffold** (S39 a). The full implementation —
//! real `tract-onnx` runtime + FinBERT sentiment + user-supplied
//! decision model — lands in S39 (b).
