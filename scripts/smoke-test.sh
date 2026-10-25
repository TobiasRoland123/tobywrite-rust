#!/usr/bin/env bash
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."
cargo build --locked
smoke_dir="$(mktemp -d -t tobywrite-smoke.XXXXXX)"
clipboard_type=''
restore_clipboard=0
if command -v wl-paste >/dev/null && command -v wl-copy >/dev/null; then
  clipboard_type="$(wl-paste --list-types 2>/dev/null | head -n 1 || true)"
  if [[ -n "$clipboard_type" ]]; then
    if wl-paste --no-newline --type "$clipboard_type" > "$smoke_dir/clipboard" 2>/dev/null; then restore_clipboard=1; fi
  else
    restore_clipboard=1
  fi
fi
cleanup() {
  if [[ "$restore_clipboard" == 1 ]]; then
    if [[ -n "$clipboard_type" ]]; then
      wl-copy --type "$clipboard_type" < "$smoke_dir/clipboard"
    else
      wl-copy --clear
    fi
  fi
  rm -rf -- "$smoke_dir"
}
trap cleanup EXIT
timeout 30s env TOBYWRITE_DATA_DIR="$smoke_dir/state" TOBYWRITE_NON_UNIQUE=1 TOBYWRITE_SMOKE_TEST=1 G_DEBUG=fatal-criticals \
  target/debug/tobywrite 2>&1 | tee "$smoke_dir/gtk.log"
grep -q 'TOBYWRITE_SMOKE_OK' "$smoke_dir/gtk.log"
target/debug/tobywrite --check assets/Welcome.md
target/debug/tobywrite --export-html assets/Welcome.md "$smoke_dir/welcome.html"
target/debug/tobywrite --export-pdf assets/Welcome.md "$smoke_dir/welcome.pdf"
if command -v pdftotext >/dev/null; then
  pdftotext "$smoke_dir/welcome.pdf" "$smoke_dir/welcome.txt"
  grep -q 'A little room to write' "$smoke_dir/welcome.txt"
  grep -q 'list of keyboard shortcuts' "$smoke_dir/welcome.txt"
fi
echo 'Native GTK and export smoke tests passed.'
