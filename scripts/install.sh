#!/usr/bin/env bash
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."
prefix="${PREFIX:-$HOME/.local}"
data_dir="${XDG_DATA_HOME:-$HOME/.local/share}"
if [[ "${1:-}" != "--no-build" ]]; then ./scripts/build.sh; fi
install -Dm755 target/release/tobywrite "$prefix/bin/tobywrite"
install -Dm644 assets/tobywrite.png "$data_dir/icons/hicolor/256x256/apps/com.tobywrite.Writer.png"
# Quote the installed path using Desktop Entry escaping, including custom prefixes.
python3 - "$prefix/bin/tobywrite" "$data_dir/applications/com.tobywrite.Writer.desktop" <<'PY'
from pathlib import Path
import sys
binary = sys.argv[1].replace('\\', '\\\\\\\\').replace('"', '\\\\"').replace('`', '\\\\`').replace('$', '\\\\$').replace('%', '%%')
desktop = Path('assets/com.tobywrite.Writer.desktop').read_text()
desktop = desktop.replace('Exec=tobywrite %F', f'Exec="{binary}" %F')
target = Path(sys.argv[2])
target.parent.mkdir(parents=True, exist_ok=True)
target.write_text(desktop)
PY
install -Dm644 LICENSE "$data_dir/tobywrite/LICENSE"
install -Dm644 assets/fonts/OFL.txt "$data_dir/tobywrite/OFL.txt"
command -v update-desktop-database >/dev/null && update-desktop-database "$data_dir/applications" || true
command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -f -t "$data_dir/icons/hicolor" >/dev/null 2>&1 || true
echo "Installed Tobywrite to $prefix/bin/tobywrite"
echo 'Open Tobywrite from your app launcher, or run tobywrite notes.md.'
