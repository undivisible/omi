#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
MACOS_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

tmpdir="$(mktemp -d "${TMPDIR:-/tmp}/omi-prepare-native-deps-test.XXXXXX")"
cleanup() {
  rm -rf "$tmpdir"
}
trap cleanup EXIT

fakebin="$tmpdir/bin"
app_bundle="$tmpdir/Omi Test.app"
main_binary="$app_bundle/Contents/MacOS/Omi Computer"
mkdir -p "$fakebin" "$app_bundle/Contents/MacOS" "$app_bundle/Contents/Resources"

cat > "$fakebin/file" <<'EOF'
#!/usr/bin/env bash
for arg in "$@"; do
  case "$arg" in
    *"/Contents/MacOS/Omi Computer")
      echo "$arg: Mach-O universal binary with 2 architectures: [x86_64] [arm64]"
      ;;
    *)
      echo "$arg: ASCII text"
      ;;
  esac
done
EOF
chmod +x "$fakebin/file"

cat > "$fakebin/strip" <<'EOF'
#!/usr/bin/env bash
target="${@: -1}"
before="$(wc -c < "$target" | tr -d ' ')"
after=$((before - 256))
if [ "$after" -lt 1 ]; then
  after=1
fi
perl -e 'truncate $ARGV[0], $ARGV[1] or die "truncate: $!"' "$target" "$after"
EOF
chmod +x "$fakebin/strip"

cat > "$fakebin/dyld_info" <<'EOF'
#!/usr/bin/env bash
if [[ "${OMI_TEST_INVALID_STRIP:-0}" == "1" ]]; then
  exit 1
fi
EOF
chmod +x "$fakebin/dyld_info"

head -c 4096 /dev/zero > "$main_binary"
chmod +x "$main_binary"

before="$(wc -c < "$main_binary" | tr -d ' ')"
output="$(PATH="$fakebin:$PATH" "$MACOS_DIR/scripts/prepare-desktop-bundle-native-deps.sh" "$app_bundle")"
after="$(wc -c < "$main_binary" | tr -d ' ')"

if [ "$after" -ge "$before" ]; then
  fail "main executable was not stripped: before=$before after=$after"
fi
if ! grep -q "Stripped main app executable:" <<< "$output"; then
  fail "prepare script did not report main executable strip"
fi
if ! grep -q "Prepared desktop bundle native dependencies" <<< "$output"; then
  fail "prepare script did not complete successfully"
fi

head -c 4096 /dev/zero > "$main_binary"
before="$(wc -c < "$main_binary" | tr -d ' ')"
output="$(PATH="$fakebin:$PATH" OMI_TEST_INVALID_STRIP=1 "$MACOS_DIR/scripts/prepare-desktop-bundle-native-deps.sh" "$app_bundle")"
after="$(wc -c < "$main_binary" | tr -d ' ')"
if [ "$after" -ne "$before" ]; then
  fail "invalid strip output was not restored: before=$before after=$after"
fi
if [ ! -x "$main_binary" ]; then
  fail "restored executable lost its execute permission"
fi
if ! grep -q "Skipping main app executable strip: invalid Mach-O output" <<< "$output"; then
  fail "prepare script did not report invalid Mach-O strip output"
fi

echo "prepare-desktop-bundle-native-deps tests passed"
