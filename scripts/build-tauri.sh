#!/usr/bin/env bash
# Build the Tauri Bee GUI for production.
#
# Usage:
#   scripts/build-tauri.sh               # current-host triple (aarch64-apple-darwin)
#   scripts/build-tauri.sh --no-bundle   # just compile the binary + frontend, skip bundling
#   scripts/build-tauri.sh --release-paths  # print where the output went
#
# First build takes ~30 min on M2 (Tauri 2.x + WKWebView + webkit deps). Subsequent
# builds are minutes. The CI workflow on `feat/taui-cleanup` runs `npm ci` +
# `tsc --noEmit` + `vite build` only (no Rust compile) to keep CI fast.

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
APP_DIR="$ROOT/app"

NO_BUNDLE=false
PRINT_PATHS=false
for arg in "$@"; do
  case "$arg" in
    --no-bundle)  NO_BUNDLE=true ;;
    --release-paths) PRINT_PATHS=true ;;
    *) echo "unknown arg: $arg" >&2; exit 1 ;;
  esac
done

echo "==> Bee GUI Tauri build (cwd: $ROOT)"
echo "    node:  $(node --version)"
echo "    npm:   $(npm --version)"
echo "    rust:  $(rustup run stable rustc --version)"
echo

# 1. Frontend deps
echo "==> npm ci"
(cd "$APP_DIR" && npm ci --no-audit --no-fund)

# 2. Typecheck
echo "==> tsc --noEmit"
(cd "$APP_DIR" && npx tsc --noEmit)

# 3. Frontend bundle
echo "==> vite build"
(cd "$APP_DIR" && npm run build)

# 4. Rust build (the slow step)
echo "==> cargo build (Tauri 2.x — first build ~30 min on M2)"
(cd "$APP_DIR/src-tauri" && rustup run stable cargo build --release)

if [[ "$PRINT_PATHS" == true ]]; then
  echo
  echo "==> Output paths:"
  for p in \
    "$APP_DIR/dist/index.html" \
    "$APP_DIR/src-tauri/target/release/app" \
    "$APP_DIR/src-tauri/target/release/bundle/macos/Bee GUI.app" \
    "$APP_DIR/src-tauri/target/release/bundle/dmg/Bee GUI_0.1.0_aarch64.dmg" \
    "$APP_DIR/src-tauri/target/release/bundle/deb/bee-gui_0.1.0_amd64.deb" \
    "$APP_DIR/src-tauri/target/release/bundle/msi/Bee GUI_0.1.0_x64_en-US.msi" \
    "$APP_DIR/src-tauri/target/release/bundle/nsis/Bee GUI_0.1.0_x64-setup.exe" \
    "$APP_DIR/src-tauri/target/release/bundle/appimage/bee-gui_0.1.0_amd64.AppImage" \
  ; do
    [[ -e "$p" ]] && echo "    ✓ $p"
  done
fi

echo
echo "==> Done."
if [[ "$NO_BUNDLE" == false ]]; then
  echo "    Full bundling (.dmg / .deb / .msi / .AppImage) takes longer."
  echo "    Pass --no-bundle to skip installer creation."
fi