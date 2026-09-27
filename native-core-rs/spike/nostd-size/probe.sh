#!/bin/sh
# Reproduces the no_std size probe from SPIKE.md: copies the crate to a temp
# dir, flips lib.rs to no_std with an aborting panic handler, builds release
# staticlibs (host + aarch64-apple-darwin), links the differential driver, and
# prints the linked, stripped size next to the C++ baseline.
#
# The probe copy is throwaway (cargo test there is out of scope); numbers are
# reported in SPIKE.md. Rust's #[panic_handler] is required even under
# panic = "abort", which the probe verifies by building both ways.
set -eu

here=$(cd "$(dirname "$0")" && pwd)
crate_src=$(cd "$here/../.." && pwd)
probe=${TMPDIR:-/tmp}/omi-nostd-probe

rm -rf "$probe"
cp -R "$crate_src" "$probe"
rm -rf "$probe/target" "$probe/build-apple"

sed -e '1i #![no_std]' \
    -e 's/use std::ffi::CStr;/use core::ffi::CStr;/' \
    -e 's/std::ffi::c_char/core::ffi::c_char/g' \
    -e 's/std::ptr::/core::ptr::/g' \
    -e 's/std::slice::/core::slice::/g' \
    "$probe/src/lib.rs" > "$probe/src/lib.rs.new"
mv "$probe/src/lib.rs.new" "$probe/src/lib.rs"
cat >> "$probe/src/lib.rs" <<'EOF'

#[panic_handler]
fn spike_panic(_: &core::panic::PanicInfo<'_>) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
EOF

cargo build --release --manifest-path "$probe/Cargo.toml"
cargo build --release --manifest-path "$probe/Cargo.toml" \
  --target aarch64-apple-darwin

out=${TMPDIR:-/tmp}/omi-nostd-probe-out
mkdir -p "$out"
${CXX:-g++} -std=c++20 -O2 \
  -I "$probe/cinclude" \
  "$probe/spike/differential/driver.cpp" \
  "$probe/target/release/libomi_native_core_rs.a" \
  -lpthread -ldl -Wl,--gc-sections \
  -o "$out/driver_nostd_gc"
strip -s "$out/driver_nostd_gc"

"$out/driver_nostd_gc" "$crate_src/spike/differential/vectors.txt" \
  > "$out/nostd.txt"
"$out/driver_nostd_gc" "$crate_src/spike/differential/vectors.txt" \
  | diff -q - "$out/nostd.txt" >/dev/null

echo "no_std staticlib (host):        $(wc -c < "$probe/target/release/libomi_native_core_rs.a") bytes"
echo "no_std staticlib (apple darwin):$(wc -c < "$probe/target/aarch64-apple-darwin/release/libomi_native_core_rs.a") bytes"
echo "linked + stripped (host):       $(wc -c < "$out/driver_nostd_gc") bytes"
