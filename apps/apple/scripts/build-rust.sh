#!/bin/bash
# Builds the fluxtv-ffi static library for the platform Xcode is building and regenerates the
# Swift bindings in FluxTV/Core/FFI when the Rust API changed.
#
# Called by the "Build Rust core" phase of both app targets. It can also be run by hand:
#   PLATFORM_NAME=iphonesimulator CONFIGURATION=Debug BUILT_PRODUCTS_DIR=/tmp/out apps/apple/scripts/build-rust.sh
set -euo pipefail

export PATH="$HOME/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:$PATH"
# Xcode points SDKROOT at the iOS/tvOS SDK, which breaks host builds (build scripts, bindgen).
# The cc crate finds the right SDK per target by itself.
unset SDKROOT

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "$SCRIPT_DIR/../../.." && pwd)"
FFI_OUT="$SCRIPT_DIR/../FluxTV/Core/FFI"
TARGET_DIR="${CARGO_TARGET_DIR:-$REPO/target}"

if ! command -v cargo >/dev/null; then
    echo "error: cargo not found. Install Rust from https://rustup.rs" >&2
    exit 1
fi

case "${PLATFORM_NAME:?PLATFORM_NAME not set}" in
    iphoneos) TARGETS=(aarch64-apple-ios) ;;
    iphonesimulator)
        TARGETS=()
        for arch in ${ARCHS:-arm64}; do
            case "$arch" in
                arm64) TARGETS+=(aarch64-apple-ios-sim) ;;
                x86_64) TARGETS+=(x86_64-apple-ios) ;;
            esac
        done ;;
    appletvos) TARGETS=(aarch64-apple-tvos) ;;
    appletvsimulator) TARGETS=(aarch64-apple-tvos-sim) ;;
    *) echo "error: unsupported platform $PLATFORM_NAME" >&2; exit 1 ;;
esac

if [ "${CONFIGURATION:-Debug}" = "Release" ]; then PROFILE=apple-release; else PROFILE=apple-dev; fi

LIBS=()
for target in "${TARGETS[@]}"; do
    rustup target list --installed 2>/dev/null | grep -qx "$target" || rustup target add "$target"
    echo "Building fluxtv-ffi for $target ($PROFILE)"
    cargo build --manifest-path "$REPO/Cargo.toml" -p fluxtv-ffi --lib --profile "$PROFILE" --target "$target"
    LIBS+=("$TARGET_DIR/$target/$PROFILE/libfluxtv_ffi.a")
done

OUT_DIR="${BUILT_PRODUCTS_DIR:?BUILT_PRODUCTS_DIR not set}"
mkdir -p "$OUT_DIR"
if [ "${#LIBS[@]}" -gt 1 ]; then
    lipo -create "${LIBS[@]}" -output "$OUT_DIR/libfluxtv_ffi.a"
else
    cp -f "${LIBS[0]}" "$OUT_DIR/libfluxtv_ffi.a"
fi

# Bindings are generated from the metadata in the compiled library, so they always match it.
cargo build --manifest-path "$REPO/Cargo.toml" -p fluxtv-ffi --features bindgen --bin uniffi-bindgen --profile apple-dev
GEN="$(mktemp -d)"
trap 'rm -rf "$GEN"' EXIT
"$TARGET_DIR/apple-dev/uniffi-bindgen" "${LIBS[0]}" "$GEN" \
    --swift-sources --headers --modulemap \
    --module-name fluxtv_ffiFFI --modulemap-filename module.modulemap
mkdir -p "$FFI_OUT"
for f in "$GEN"/*; do
    name="$(basename "$f")"
    if ! cmp -s "$f" "$FFI_OUT/$name"; then
        echo "Updating generated $name"
        cp -f "$f" "$FFI_OUT/$name"
    fi
done
