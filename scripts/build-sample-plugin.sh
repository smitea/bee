#!/usr/bin/env bash
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/.." && pwd)"
plugin_dir="$root/plugins/sample-kline"

profile="${1:-release}"

case "$(uname -s)" in
    Darwin) ext="dylib" ;;
    Linux)  ext="so" ;;
    MINGW*|MSYS*|CYGWIN*) ext="dll" ;;
    *) echo "Unsupported OS" >&2; exit 1 ;;
esac

if [[ "$profile" == "release" ]]; then
    target_dir="$root/target/release"
    build_args=(--release)
else
    target_dir="$root/target/debug"
    build_args=()
fi

echo "==> Building bee_plugin_sample_kline ($profile)"
( cd "$plugin_dir" && cargo build "${build_args[@]}" )

src_lib="$target_dir/libbee_plugin_sample_kline.$ext"
if [[ ! -f "$src_lib" ]]; then
    echo "expected $src_lib to exist after build" >&2
    exit 1
fi

dest_dir="${BEE_PLUGIN_DIR:-$HOME/.bee/plugins}"
mkdir -p "$dest_dir"
dest="$dest_dir/libbee_plugin_sample_kline.$ext"
cp "$src_lib" "$dest"

echo "==> Installed plugin"
echo "    source : $src_lib"
echo "    dest   : $dest"
echo
echo "Open Bee Client, go to Settings > Plugins, and click \"Reload from disk\""
echo "to see the sample-kline plugin appear in the list."