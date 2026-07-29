#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "$0")" && pwd)"
script_path="$script_dir/../scripts/audit-desktop-bundle-deps.sh"

grep -Fq 'for rpath in "${candidate_rpaths[@]+"${candidate_rpaths[@]}"}"; do' "$script_path"
