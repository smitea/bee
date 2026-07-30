# bee_plugin_sample_kline

Sample Bee plugin used to verify the **Reload from disk** flow in
Bee Client's **Settings > Plugins** panel.

The plugin exposes:

| Kind   | Name   | Purpose                                                                    |
|--------|--------|----------------------------------------------------------------------------|
| Input  | `kline`   | Open a config-shaped blob `{ url, symbol, interval }` and emit a synthetic price event. |
| Handler| `ema`     | Stateful exponential moving average over the input stream.                  |
| Output | `emit`    | Sink that records every emitted row (the host's `close` drops it).          |

The plugin deliberately stays self-contained — no live network
calls, no host KV — so the Reload from disk flow can be exercised
without any external dependencies.

## Build

The plugin ships as a Rust `cdylib`. To build it and install the
resulting library into the Bee Client default plugin directory:

```bash
./scripts/build-sample-plugin.sh
# or
./scripts/build-sample-plugin.sh release    # explicit profile
```

The script:

1. Runs `cargo build --release` inside `plugins/sample-kline/`.
2. Copies `libbee_plugin_sample_kline.{dylib,so,dll}` into
   `${BEE_PLUGIN_DIR:-~/.bee/plugins}/`.

`BEE_PLUGIN_DIR` overrides the destination when set.

To build only (without copying):

```bash
cargo build --release -p bee_plugin_sample_kline
```

The artifact lands at
`target/release/libbee_plugin_sample_kline.dylib` on macOS
(`.so` on Linux, `.dll` on Windows).

## Verify the Reload from disk flow

After `./scripts/build-sample-plugin.sh`:

1. Launch Bee Client (`cargo tauri dev` in `app/`).
2. Open **Settings** from the sidebar gear icon.
3. Select the **Plugins** category in the left column.
4. Click **Reload from disk**.
5. The **Plugins registered** badge increments by one and the row
   below shows:

   | ID (sha256) | Name | Version | Adapters | Handlers |
   |-------------|------|---------|----------|----------|
   | `e3b0c44…`  | `sample-kline` | `0.1.0` | `kline`, `emit` | `ema` |

The `id` is the hex-encoded sha256 of the `.dylib` file contents
(ADR-0009), so it changes whenever the plugin is rebuilt.

## Tests

The end-to-end Reload-from-disk flow is exercised by
`app/src-tauri/tests/sample_plugin_load.rs`:

- builds (or reuses) the cdylib,
- copies it into a tempdir,
- runs `PluginRegistry::scan_directory`,
- asserts the summary exposes `sample-kline` + its adapters / handler,
- calls `plugin_schema("sample-kline")` and asserts the `kline`
  adapter is in the returned schema,
- loads the same `.dylib` twice and asserts the content-hash
  PluginId is stable.

```bash
cd app/src-tauri
cargo test --test sample_plugin_load
```