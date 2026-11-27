#!/usr/bin/env bash
# Prints the CHANGELOG.md section of a version: scripts/release-notes.sh 0.1.0
set -euo pipefail
v="${1#v}"
awk -v v="$v" '
  $0 ~ "^## \\[" v "\\]" { on = 1; next }
  on && /^## \[/ { exit }
  on { print }
' "$(dirname "$0")/../CHANGELOG.md"
