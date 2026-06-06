# 0009: Plugin multi-version coexistence; hash-based identity; strict ABI compatibility

When a Plugin is upgraded, running Pipelines must not be interrupted. We adopt **multi-version coexistence (B)** for MVP: multiple versions of the same logical Plugin can be loaded simultaneously. Old Pipelines continue with their bound version, new Pipelines can opt in to the new version. The Plugin Manager loads new versions into Registry on detection; old versions are unloaded only after all referencing Pipelines have stopped (or after explicit `bee plugin unload --force`).

**Plugin identity is content-hash based, not version-string based.** Each loaded Plugin is identified by `hash(sha256, plugin_binary_content)`. The version string in the Plugin Manifest is human-readable metadata; the hash is the binding truth. This protects against version-string drift (different authors tagging "1.0" inconsistently) and ensures two binary-different builds can never be confused for the same Plugin. The KV state key includes the hash: `state/task/{TaskId}/h{hash}/...`. Pipeline Authors can still write `binance:^1.0` and the runtime resolves it to the actual loaded Plugin by name + version range, but the binding contract is hash-based.

**ABI compatibility is enforced strictly.** Each Plugin Manifest declares an `abi_version` (e.g., `"1.0"`). Bee has a configured supported ABI version range. A Plugin is loaded only if its `abi_version` is in the supported range — incompatible Plugins are rejected outright at load time, with no fallback or "best effort" loading. ABI version bumps are independent of feature version bumps. This is stricter than SemVer alone: SemVer allows a major version break (user can opt in), but an ABI version break means Bee will refuse to load the Plugin entirely. The Plugin author must recompile against the current Bee SDK to ship a compatible upgrade.

Online state migration (option C from the original menu) is **deferred to 1.x**. For now, when a Plugin version is fully retired, the old state remains in KV until the TTL expires (default 7 days per ADR-0004); explicit `bee plugin migrate` is a 1.x addition.

## Consequences

- Zero-downtime Plugin upgrades for running Pipelines. Old version runs until the Pipeline naturally retires; state is isolated by hash.
- Hash-based identity eliminates version-string confusion and makes state isolation robust to author mis-tagging.
- Strict ABI enforcement prevents silent breakage from incompatible Plugins. Plugin authors must recompile against the current Bee SDK to ship upgrades.
- Multi-version coexistence costs memory: each loaded .so is ~MB-scale. `bee plugin list` shows all loaded versions with their hashes and refcounts.
- Version range syntax (`binance:^1.0` / `binance:latest`) is supported in Pipeline definitions and resolved by name + version range at submit time. Authors do not write hashes.
- Plugin authoring toolchain: `cargo build --release` of `bee-plugin-sdk` produces a .so. `bee plugin inspect <path>` shows the computed content hash and the declared `abi_version`. The hash is the Plugin's "true name" in the Registry.
- Failed ABI compatibility check at load time produces a clear error log; the Plugin is left on disk for inspection but is not registered.
- Old Plugin state cleanup: TTL-driven (7 days default). The user can extend TTL per Plugin or wait for natural expiration. Explicit cleanup / migration is a 1.x feature.
