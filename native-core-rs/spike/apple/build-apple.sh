#!/bin/sh
# Apple build + smoke test for the Rust spike. Run on a Mac.
#
# Produces:
#   build-apple/libomi_native_core_rs_macos.a   (universal arm64 + x86_64)
#   build-apple/libomi_native_core_rs_ios.a     (arm64 device)
#   build-apple/omi_policy_smoke                (ObjC++ test binary, run here)
#
# Requirements: rustup stable + Xcode command line tools. No CocoaPods, no
# CMake, no extra crates.
set -eu

here=$(cd "$(dirname "$0")" && pwd)
crate=$(cd "$here/../.." && pwd)
out="$crate/build-apple"
mkdir -p "$out"

for t in aarch64-apple-darwin x86_64-apple-darwin aarch64-apple-ios; do
  rustup target add "$t"
  cargo build --release --manifest-path "$crate/Cargo.toml" --target "$t"
done

lipo -create \
  "$crate/target/aarch64-apple-darwin/release/libomi_native_core_rs.a" \
  "$crate/target/x86_64-apple-darwin/release/libomi_native_core_rs.a" \
  -output "$out/libomi_native_core_rs_macos.a"
cp "$crate/target/aarch64-apple-ios/release/libomi_native_core_rs.a" \
  "$out/libomi_native_core_rs_ios.a"

# Objective-C++ smoke test against the universal macOS lib.
clang++ -x objective-c++ -std=c++20 -O2 \
  -I "$crate/cinclude" \
  "$here/omi_policy_smoke.mm" \
  "$out/libomi_native_core_rs_macos.a" \
  -framework Foundation \
  -o "$out/omi_policy_smoke"
"$out/omi_policy_smoke"

# iOS app targets link the same lib; on-device smoke (adjust for your target):
#   xcrun -sdk iphoneos clang++ -arch arm64 -x objective-c++ -std=c++20 -O2 \
#     -I "$crate/cinclude" "$here/omi_policy_smoke.mm" \
#     "$out/libomi_native_core_rs_ios.a" -framework Foundation \
#     -o "$out/omi_policy_smoke_ios"

# Simulator arm64 needs its own std: `rustup target add aarch64-apple-ios-sim`
# then cargo build --release --target aarch64-apple-ios-sim and lipo that in.

echo "build-apple: OK — artifacts in $out"
