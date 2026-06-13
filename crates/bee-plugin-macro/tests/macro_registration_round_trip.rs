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

#[bee_adapter(input, name = "simple")]
impl SimpleInput {
    #[bee_method(slot = "open")]
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
