#!/bin/sh
# Differential parity check: build the same FFI driver against
# (A) the native-core C++ implementation and (B) the Rust staticlib,
# run both over vectors.txt, and diff the outputs byte-for-byte.
# Runs on any host with g++/clang++ + cargo (verified on linux x86_64).
set -eu

here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../../.." && pwd)
out=${TMPDIR:-/tmp}/omi-native-core-rs-diff
mkdir -p "$out"

# B-side staticlib (host target is fine: the ABI is what's under test).
cargo build --release --manifest-path "$repo/native-core-rs/Cargo.toml"

# A-side: driver + original C++ sources, against the drop-in header.
${CXX:-g++} -std=c++20 -O2 -Wall -Wextra \
  -I "$repo/native-core-rs/cinclude" \
  -I "$repo/native-core/include" \
  "$here/driver.cpp" \
  "$repo/native-core/src/omi_backend_policy.cpp" \
  "$repo/native-core/src/omi_native_boundary.cpp" \
  -o "$out/driver_cpp"

# B-side: driver + Rust staticlib (pull in libdl/libpthread for Rust std).
${CXX:-g++} -std=c++20 -O2 -Wall -Wextra \
  -I "$repo/native-core-rs/cinclude" \
  "$here/driver.cpp" \
  "$repo/native-core-rs/target/release/libomi_native_core_rs.a" \
  -lpthread -ldl \
  -o "$out/driver_rs"

"$out/driver_cpp" "$here/vectors.txt" > "$out/cpp.txt"
"$out/driver_rs" "$here/vectors.txt" > "$out/rs.txt"

if diff -u "$out/cpp.txt" "$out/rs.txt"; then
  probes=$(grep -cv -e '^#' -e '^$' "$here/vectors.txt")
  echo "differential: PASS — ${probes} probes + null battery identical between C++ and Rust"
else
  echo "differential: FAIL — outputs differ (see diff above)"
  exit 1
fi
