#!/usr/bin/env bash
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."
case "${1:-release}" in
  release) cargo build --locked --release ;;
  debug) cargo build --locked ;;
  *) echo "Usage: $0 [release|debug]" >&2; exit 2 ;;
esac
