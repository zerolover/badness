#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
DIST_DIR="$REPO_ROOT/dist/badness-ffi"
LIB_SOURCE_DIR="$REPO_ROOT/target/release"
INCLUDE_DIR="$DIST_DIR/inc/badness"
LIB_DIR="$DIST_DIR/libs"

if ! command -v cargo >/dev/null 2>&1; then
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    . "$HOME/.cargo/env"
fi

mkdir -p "$INCLUDE_DIR" "$LIB_DIR"

# badness is a single package (crate-type = ["rlib", "cdylib"]); --lib skips
# building the `badness` CLI binary, which the FFI bundle doesn't need.
cargo build --manifest-path "$REPO_ROOT/Cargo.toml" --release --lib

cp "$REPO_ROOT/include/badness_ffi.h" "$INCLUDE_DIR/badness_ffi.h"
cp "$REPO_ROOT/include/badness.hpp" "$INCLUDE_DIR/badness.hpp"
if [ "$(uname -s)" = "Darwin" ]; then
    cp "$LIB_SOURCE_DIR/libbadness.dylib" "$LIB_DIR/"
    install_name_tool -id "@rpath/libbadness.dylib" "$LIB_DIR/libbadness.dylib"
else
    cp "$LIB_SOURCE_DIR/libbadness.so" "$LIB_DIR/"
fi

find "$DIST_DIR" -type f | sort
