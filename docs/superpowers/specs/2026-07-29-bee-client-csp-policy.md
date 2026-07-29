# Bee Client CSP Policy Design Note

**Date:** 2026-07-29
**Status:** Approved design

## Purpose

Replace the previously disabled Content-Security-Policy (`"csp": null`) on the Bee Client Tauri shell with a restrictive production policy that permits only the origins the Tauri IPC channel actually requires.

## Threat model

The Bee Client frontend is a Tauri 2.x React + Vite application. It is bundled at build time into local assets and loaded from the local filesystem via Tauri's custom `tauri://` (and during development, `http://tauri.localhost`) protocol. The frontend does not need to load any remote script, stylesheet, font, image, frame or worker. The only cross-origin interactions are:

1. **Tauri IPC**: the frontend calls `window.__TAURI_INTERNALS__.invoke(...)` to reach Rust commands. The wire protocol uses `ipc://` (production) and `http://ipc.localhost` (development) as the local origin.
2. **Static local assets**: every script, style and image shipped with the bundle.

A permissive or disabled CSP would allow an injected frontend dependency (or a future supply-chain compromise) to pull additional code from arbitrary origins and exfiltrate Application credentials and AdminServer control-plane responses.

## Policy

```
default-src 'self';
script-src 'self' 'ipc:' 'http://ipc.localhost';
style-src 'self' 'unsafe-inline';
img-src 'self' data: ipc: http://ipc.localhost;
font-src 'self' data:;
connect-src 'self' ipc: http://ipc.localhost;
frame-src 'none';
object-src 'none';
worker-src 'self' blob:;
form-action 'none';
base-uri 'self';
manifest-src 'self';
```

## Why each directive is required

| Directive | Value | Reason |
|---|---|---|
| `default-src` | `'self'` | Baseline; any directive not explicitly listed falls back to `default-src`, so this denies everything by default. |
| `script-src` | `'self' 'ipc:' 'http://ipc.localhost'` | The bundled JS is loaded from the local origin. Tauri's IPC plumbing uses the `ipc:` and `http://ipc.localhost` origins for any internally-bridged script execution. No remote scripts allowed. |
| `style-src` | `'self' 'unsafe-inline'` | Bundled CSS lives at `'self'`. React and Vite emit inline `style` attributes for runtime styling in many components; removing `'unsafe-inline'` would break the existing compiled CSSOM. No remote styles allowed. |
| `img-src` | `'self' data: ipc: http://ipc.localhost` | App icons live under `'self'` and as base64-embedded `data:` URIs. The IPC origin covers any image served via the Tauri asset protocol. |
| `font-src` | `'self' data:` | Bundled fonts live at `'self'`; icon-font SVGs and small inline fonts may use `data:`. No remote fonts allowed. |
| `connect-src` | `'self' ipc: http://ipc.localhost` | XHR / fetch targets are limited to the bundle origin plus the Tauri IPC endpoints. The AdminServer protocol is reached by the Rust side, not from the frontend. |
| `frame-src` | `'none'` | The app does not embed any iframe. |
| `object-src` | `'none'` | The app does not load Flash / Java applets / arbitrary plugins. |
| `worker-src` | `'self' blob:` | Allows the React dev-mode web worker and any future bundler-emitted blob workers. No remote workers. |
| `form-action` | `'none'` | The frontend never submits a form to an external origin. |
| `base-uri` | `'self'` | Prevents a `<base>` tag injection from re-pointing all relative URLs to an attacker origin. |
| `manifest-src` | `'self'` | If a web manifest is added later, it must live under the bundle origin. |

## Capabilities file

The Tauri capabilities file (`app/src-tauri/capabilities/default.json`) keeps the default permission set (`core:default`) only. No additional permissions (no `fs:`, no `shell:`, no `http:`) are granted because the Rust backend brokers all filesystem, dialog, cryptographic and AdminServer access on behalf of the frontend.

## Acceptance criteria

- `app.security.csp` is set to the restrictive policy above (no `null`).
- `app/src-tauri/capabilities/default.json` lists only `core:default`.
- `cargo check -p app` passes.
- The frontend boots and renders the existing UI when launched via `cargo tauri dev`.
- No request from the running app is observed against any remote origin beyond the bundle origin and the Tauri IPC origin.

## Out of scope

- Per-request nonce / hash-based CSP hardening (not required for a desktop bundle with no third-party scripts).
- Multi-cluster AdminServer reachability from the frontend (the AdminServer is reached from Rust, not the browser).
- CORS for external API calls (the client does not make any).