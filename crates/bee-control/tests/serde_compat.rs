//! S33.5.2: smoke test that the `Datasource`
//! struct round-trips through bincode. This
//! guards the S33.5.2 change to add
//! `Serialize, Deserialize` derives to
//! `Datasource` + its SDK dependencies.

use bee_control::datasource::{Datasource, DatasourceStatus};
use bee_plugin_sdk::{PluginId, VersionSpec};

#[test]
fn datasource_bincode_roundtrip() {
    let ds = Datasource::new(
        "binance".to_string(),
        0,
        "binance".to_string(),
        PluginId("abc123".to_string()),
        VersionSpec::Latest,
        "{}".to_string(),
    );
    let bytes = bincode::serialize(&ds).expect("bincode serialize");
    let restored: Datasource =
        bincode::deserialize(&bytes).expect("bincode deserialize");
    assert_eq!(ds, restored);
    let paused = Datasource {
        status: DatasourceStatus::Paused,
        ..ds.clone()
    };
    let bytes2 = bincode::serialize(&paused).expect("serialize paused");
    let restored2: Datasource =
        bincode::deserialize(&bytes2).expect("deserialize paused");
    assert_eq!(restored2.status, DatasourceStatus::Paused);
}
