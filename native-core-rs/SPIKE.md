# Spike: Rust for `native-core` (C ABI staticlib)

Feasibility question: can the C/C++ core in `native-core/` be replaced or
wrapped with Rust compiled for Apple targets, keeping the Objective-C++
boundary files thin? Verdict up front: **yes, technically clean** — the
existing boundary is already a pure `extern "C"` ABI, a Rust staticlib is a
link-time drop-in, behavior parity is mechanically provable (and was proven,
below), and a `no_std` build costs ~8 KB of linked size over the C++
baseline. The real cost is toolchain: every macOS/CI build gains a Rust
dependency that today's "plain C++ sources in the Xcode target" setup does
not have.

Everything in this directory is spike scaffolding, not product code.
Branch: `spike/rust-native-core` (off `origin/v5`).

## 1. What native-core provides, and who consumes it

`native-core` is four C++20 modules behind one C ABI (all pure logic: no
syscalls, no threads, caller-provided buffers, no heap allocation except one
small `std::string` in `omi_backend_is_allowed_v5_hostname`). 27 exported
functions:

| Module (source) | Exported C symbols | Consumed by |
|---|---|---|
| `omi_native_boundary.cpp` | `omi_calculate_packet_checksum`, `omi_normalize_packet`, `omi_get_native_capabilities` | `react-native/ios/RnRuntime/OmiCppBoundary.mm` (normalize, capabilities), Android `jni_bridge.cpp` (same); `omi_calculate_packet_checksum` is internal-only |
| `omi_backend_policy.cpp` | `omi_backend_route_strip`, `omi_backend_is_capture_path`, `omi_backend_request_timeout_seconds`, `omi_backend_example_platform_supported`, `omi_backend_is_loopback_hostname`, `omi_backend_is_cloud_hostname`, `omi_backend_is_allowed_v5_hostname`, `omi_backend_software_plane_is_new` | macOS `OmiBackendModule.mm` (7 of 8 + `http_plan_request`), Apple `OmiRequestTimeout.h` (timeout), Android `jni_bridge.cpp` |
| `omi_backend_recording.cpp` | `omi_backend_recording_captured_at_valid`, `..._captured_at_equal`, `..._retryable_status`, `..._same_context`, `..._remembered_identity`, `..._remembered_current`, `..._device_valid`, `..._budget_ok` | Apple `OmiRecordingPolicy.h` (6) and `OmiRecordingJournals.h` (2), Android `jni_bridge.cpp` |
| `omi_backend_http.cpp` | `omi_backend_http_request_valid`, `omi_backend_http_plan_request`, `omi_backend_http_execute` (+ injector callback type), `omi_backend_recording_owner_key_valid`, `..._receipt_valid`, `..._uuid_valid`, `..._journal_relpath`, `..._path_owned` | Apple `OmiRecordingJournals.h` (5 validators + relpath), macOS `OmiBackendModule.mm` (plan), Android `jni_bridge.cpp` |

Notes:

- `react-native/apple/OmiAudioCapture.mm` uses AudioToolbox directly and
  imports **no** native-core symbols; it is unaffected by this spike.
- The macOS Xcode project compiles the three backend `.cpp` files directly as
  target sources (`project.pbxproj` file refs to `../../../native-core/src/*`),
  with `$(SRCROOT)/../../native-core/include` on the header search path; the
  iOS project does the same for all four `.cpp` files. No CocoaPods, no SPM,
  no CMake in the app build. Host tests run via `scripts/test-native-core`
  (cmake + ctest); Android builds through NDK CMake (`jni_bridge.cpp`).

## 2. What this spike built

`native-core-rs/` — a zero-dependency Rust crate (std only, MIT OR Apache-2.0)
re-implementing **10 of the 27 functions** over the identical C ABI:

- the full policy module (8 functions), which is what Apple consumes most;
- `omi_calculate_packet_checksum` + `omi_normalize_packet` (framing + CRC-32).

Remaining modules are equally mechanical (validators and one fn-pointer
planner); the spike scope was chosen to prove the pattern, not to finish the
port. Header `cinclude/omi_native_core_rs.h` is declaration-identical to the
original headers, so consumers keep including the originals.

Release profile: `opt-level = "z"`, fat LTO, 1 codegen unit, `panic = "abort"`
(no unwinding across FFI), `strip = "debuginfo"`. The library never allocates
and has no panic paths; null-pointer and overflow codes mirror the C++ exactly.

## 3. Proof

Run on this machine (linux x86_64, rustc/cargo 1.98.1 — Apple hosts use the
same commands):

1. **Rust unit tests, vectors ported from the C++ suites**
   (`cargo test`, `tests/parity.rs`): 8/8 pass — route strip (incl. nulls,
   cap 0, overflow), all 20 capture-path cases, all timeout cases (incl. UUID
   ids, empty id, `#frag`), 16 example-platform cases, hostname cases
   (`[::1]`, `API.OMI.ME`, `workers.dev` negatives), software-plane table,
   CRC-32 known vector `0xCBF43926` for `"123456789"`, and framed-packet
   round-trip plus every error status.
2. **Differential harness** (`spike/differential/run.sh`): one C++ driver
   linked twice — against the original `omi_backend_*.cpp` and against the
   Rust staticlib — runs 112 probes + a null battery over both and diffs the
   output byte-for-byte: **PASS, identical**, for both the std and the no_std
   Rust builds.
3. **C++ baseline unchanged**: all four `native-core` host suites
   (policy, http, recording, boundary — 13 assertions in boundary alone) pass
   when compiled directly with g++ here (cmake absent in this sandbox;
   `scripts/test-native-core` remains the canonical runner on dev machines).
4. **Symbol exports**: `nm` on the ELF archive and `llvm-nm` on the Mach-O
   archives show all 10 `omi_*` / `_omi_*` symbols defined in each of
   host, `aarch64-apple-darwin`, and `aarch64-apple-ios` staticlibs.
5. **Apple cross-compile from Linux**: `cargo build --release --target
   aarch64-apple-darwin` and `--target aarch64-apple-ios` both succeed from
   this non-mac host — a `staticlib` is just an archive, so no Apple linker or
   SDK is needed at this stage. Linking and running happen on the Mac
   (`spike/apple/build-apple.sh`, untested here by necessity) using exactly
   the linkage proven by the C++ driver above.

### Size and build-time cost

| Variant | staticlib `.a` | linked + stripped driver* |
|---|---|---|
| C++ baseline (two `.cpp` files) | — | 23,496 B |
| Rust, std, LTO, panic=abort | 22.6 MB host / 18.1 MB darwin / 17.8 MB ios | ~337 KB (`--gc-sections` + strip) |
| Rust, `no_std` probe | 7.6 MB host / 6.0 MB darwin | **31,192 B** (`--gc-sections` + strip) |

\* "What the app actually absorbs": archive size is misleading because rustc
bundles whole std/core rlibs into the `.a`; the final link only pulls
referenced members and Apple's `-dead_strip` (default in Xcode) does the rest.
Linux `--gc-sections` + `strip` is a conservative proxy. Reproduce the no_std
row with `spike/nostd-size/probe.sh`.

Build time per target: 0.15–0.35 s for this crate after the first build.
One-time toolchain setup: rustup (~1 min) + `rustup target add` per Apple
target (std download, seconds each).

### Toolchain/loop findings worth knowing

- `#[panic_handler]` is required even under `panic = "abort"` in `no_std`
  staticlibs (verified: removing it fails the build).
- rustup's `llvm-tools` component provides `llvm-nm`/`llvm-objdump`, which
  handle Mach-O archives on Linux; GNU `nm` cannot.
- `cargo` emits deterministic `.a`s per target; lipo on the Mac merges
  arches — `cargo-lipo` is not needed.

## 4. Integration path if adopted

The ABI does not change — only the object files underneath it do:

1. **Build shape**: keep the crate in-tree (`native-core-rs/`), pinned by
   `rust-toolchain.toml`; a new Xcode run-script build phase per target runs
   `cargo build --release --target <...>` and exposes the `.a` + existing
   `native-core/include` headers to the link step. This preserves the
   no-CocoaPods setup — no Pods, no SPM, no CMake for Apple. `lipo` the two
   macOS arches (script provided); iOS needs device + `aarch64-apple-ios-sim`
   (+ `x86_64-apple-ios` only if Intel sims still matter).
2. **Migrate by module, C++ and Rust side by side** (they export the same
   symbols — link exactly one of them per target). Keep
   `spike/differential/run.sh` in CI until a module's C++ copy is deleted.
   Policy first (already done here), then boundary, recording, http.
3. **Android**: `cargo-ndk` produces the same C ABI for the NDK; or keep the
   C++ `.cpp` files on Android while Apple leads — the shims are unaffected.
4. **Headers stay as-is**; `cbindgen` is only worth adding once header drift
   becomes real (the interface is stable and tiny).

Cost: every dev machine and CI runner that builds Apple targets needs rustup
(one bootstrap line in `scripts/setup`); CI cache for the toolchain; Xcode
phase complexity is modest but real (per-target arch mapping, DerivedData
paths, first-build latency). Alternative shape — committing prebuilt `.a`s —
avoids dev toolchain but puts binaries in git; against repo hygiene.

## 5. Risks

- **Toolchain sprawl** (the big one): Rust becomes a hard requirement of the
  Apple build; version drift is mitigated by `rust-toolchain.toml`.
- **Panics across FFI**: mitigated structurally — `panic = "abort"`, no panic
  paths on any input, null/overflow battery in the differential harness.
- **Behavior drift during migration**: two implementations of one contract
  coexist per module; the differential harness holds them to byte-identical
  behavior until the C++ copy is retired.
- **Size**: `no_std` ≈ free (~8 KB over C++ baseline). If the core ever needs
  std (allocating deps), the app grows by a few hundred KB after dead-strip.
- **Semantics edges**: hostname folding is ASCII-only by design (matches the
  C++ in the C locale); non-ASCII hosts are outside the contract.
- **Min-OS floors**: keep iOS/macOS deployment targets above the pinned
  Rust's Apple support floor (non-issue at current versions).
- **Debuggability**: Rust frames in crash reports; `strip = "debuginfo"`
  keeps exported symbols; logic is small enough that the differential harness
  is the primary debug tool.

## 6. Recommendation

**Conditional go.** Technical feasibility is fully demonstrated: identical C
ABI, provable parity, ~zero size cost in `no_std`, Apple staticlibs cross-
build trivially. The `no_std` shape (pure logic, no allocation) fits this
code exactly. Whether to switch is a toolchain-ownership decision, not a
technical one: the C++ core is 615 lines, zero-dependency, already tested,
and builds with zero extra toolchain — so adopt Rust only if the team wants
the Rust direction anyway (the repo already ships a Rust GPUI simulator in
`tools/OmiSimulator/`), and take it module-by-module with the differential
harness as the safety net. If adopted, start with the policy module as
proven here; `omi_get_native_capabilities`, recording validators, and the
http planner/executor are mechanical follow-ups.
