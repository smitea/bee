use bee_adapter::AdapterResult;
use bee_plugin_macro::{bee_adapter, bee_method};
use bee_plugin_sdk::{
    AdapterDescriptor, Factory, HandlerDescriptor, PluginHandle, PluginManifest, PluginName, PluginResult,
};

#[derive(serde::Serialize, serde::Deserialize)]
pub struct FibState {
    pub prev2: i128,
    pub prev1: i128,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct FibEvent {
    pub n: u64,
}

pub struct FibStepHandler;

#[bee_adapter(handler, name = "fib_step")]
impl FibStepHandler {
    #[bee_method(slot = "init_state")]
    pub async fn init_state() -> AdapterResult<FibState> {
        Ok(FibState { prev2: 0, prev1: 1 })
    }

    #[bee_method(slot = "handle")]
    pub async fn handle(mut state: FibState, _event: FibEvent) -> AdapterResult<(FibState, i128)> {
        let current = state.prev2;
        state.prev2 = state.prev1;
        state.prev1 = current + state.prev1;
        Ok((state, current))
    }
}

pub struct FibSeedHandler;

#[bee_adapter(handler, name = "fib_seed")]
impl FibSeedHandler {
    #[bee_method(slot = "init_state")]
    pub async fn init_state() -> AdapterResult<()> {
        Ok(())
    }

    #[bee_method(slot = "handle")]
    pub async fn handle(state: (), event: FibEvent) -> AdapterResult<((), i128)> {
        let val = if event.n == 0 {
            0
        } else if event.n == 1 {
            1
        } else {
            0
        };
        Ok((state, val))
    }
}

pub fn plugin_manifest() -> PluginManifest {
    PluginManifest {
        name: PluginName("perf_fib".into()),
        feature_version: "1.0.0".into(),
        abi_version: "v1".into(),
        adapters: vec![],
        handlers: vec![
            HandlerDescriptor {
                name: "fib_step".into(),
            },
            HandlerDescriptor {
                name: "fib_seed".into(),
            },
        ],
    }
}

pub struct PerfFibFactory;

impl Factory for PerfFibFactory {
    fn manifest() -> PluginManifest {
        plugin_manifest()
    }

    fn init() -> PluginResult<PluginHandle> {
        let input_adapters = std::collections::HashMap::new();
        let output_adapters = std::collections::HashMap::new();
        let mut handlers = std::collections::HashMap::new();
        
        bee_plugin_sdk::register_vtable! {
            input_adapters, output_adapters, handlers;
            handler "fib_step" => &FIB_STEP_HANDLER_VTABLE,
            handler "fib_seed" => &FIB_SEED_HANDLER_VTABLE,
        }

        Ok(PluginHandle {
            manifest: Self::manifest(),
            inner: std::sync::Arc::new(()),
            input_adapters,
            output_adapters,
            handlers,
        })
    }
}

bee_plugin_sdk::cdylib_plugin!(PerfFibFactory);
