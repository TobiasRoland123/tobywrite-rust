# Tobywrite

Tobywrite is a quiet, native Markdown writing app for Linux and Wayland. It is a Rust and GTK4 port of [Tobywrite for Mac](https://github.com/TobiasRoland123/tobywrite-mac), with a centered source editor, the bundled iA Writer Mono typeface, and plain text files on disk.

## Requirements

- Rust 1.92 or newer
- GTK 4.10 or newer
- Fontconfig
- A graphical Linux session (Wayland is the target desktop environment)

On Arch Linux or Omarchy, install the build and runtime dependencies with:

```sh
sudo pacman -S --needed rust base-devel gtk4 fontconfig pkgconf python
```

## Build and run

Build a release binary with:

```sh
./scripts/build.sh
```

Use `./scripts/build.sh debug` for a faster development build. Open a file with `cargo run -- path/to/notes.md`, or start with a blank document using `cargo run`.

Install the release binary and desktop launcher for your user:

```sh
./scripts/install.sh
```

The installer builds the release binary first. To install a release you already built, pass `--no-build`. The default install location is `~/.local`; set `PREFIX` to use a different binary prefix. Desktop files, the icon, and license notices go under `$XDG_DATA_HOME` (normally `~/.local/share`). The install script does not change system-wide configuration.

## Writing

- Open multiple documents in their own windows, create a blank draft, or use the file picker and Quick Open to open a Markdown or text file.
- Save and Save As use native GTK dialogs. Existing files require replacement confirmation. Saving checks whether the open file changed on disk and reports a conflict unless you confirm the overwrite.
- The editor keeps Markdown as editable source and styles headings, emphasis, links, lists, quotes, code, strikethrough, and thematic dividers. The source characters remain in the document. Syntax can be shown or faded outside the paragraph being edited.
- Format selected text as bold, italic, inline code, or a link. Return continues ordered, unordered, and task lists, and exits an empty list item. Find, replace, next match, and previous match are available from the keyboard.
- Paragraph focus mode dims surrounding text. The footer reports document statistics, including word and character counts and an estimated reading time.
- Choose system, light, or dark appearance; adjust editor text size; toggle syntax display and focus mode. Settings, linked writing folders, and recent paths are stored for the current user.
- Defaults are system appearance, 20 px text, focus mode off, syntax markers faded outside the active paragraph, and no linked folders. Text size can be set from 12 to 36 px.
- Text input, selection, and undo are provided by GTK. The editor accepts international text input through the active desktop input method. It does not currently integrate with a platform spell-check service.
- Print through the GTK print dialog, export an offline HTML file, or export a PDF from a graphical session.

### Omarchy themes

System appearance follows Omarchy's active `colors.toml`, including changes made while Tobywrite is open. The editor, Markdown colors, selection, menus, and app dialogs share the palette. Colors used for text are adjusted when needed for contrast. For example, switching from Retro 82 to Rose Pine changes an open document from dark blue to light paper without changing its text, selection, or undo history.

Light and Dark keep the brightness you choose. Tobywrite uses the Omarchy palette when it matches that choice and its built-in palette otherwise. Returning to System follows the active theme again. Native file pickers and print dialogs provided by the desktop follow the desktop's own styling.

Tobywrite reads `~/.local/state/omarchy/current/theme/colors.toml`, with support for XDG state paths and the older `~/.config/omarchy/current/theme` location. It checks for updates once per second and keeps the last valid palette while files are missing or incomplete during a switch. It also accepts legacy `bg`/`fg` and ANSI color keys. Without an Omarchy palette, System follows GTK's light/dark setting. No desktop configuration is changed.

### Shortcuts

| Action | Shortcut |
| --- | --- |
| New / open with Quick Open | Ctrl+N / Ctrl+O |
| Browse for a file / link a folder | Ctrl+Shift+O / Ctrl+D |
| Save / Save As | Ctrl+S / Ctrl+Shift+S |
| Close window | Ctrl+W |
| Find / replace | Ctrl+F / Ctrl+H |
| Next / previous match | Ctrl+G / Ctrl+Shift+G |
| Bold / italic / link / inline code | Ctrl+B / Ctrl+I / Ctrl+K / Ctrl+Shift+K |
| Focus mode / show Markdown syntax | Ctrl+Shift+D / Ctrl+Shift+M |
| Larger / smaller / actual text size | Ctrl++ / Ctrl+- / Ctrl+0 |
| Print / settings | Ctrl+P / Ctrl+, |

Shortcuts use Ctrl, following common Linux desktop conventions.

## File handling and recovery

Tobywrite reads UTF-8, UTF-8 with a byte-order mark, and BOM-marked UTF-16 little- or big-endian files. It rejects unsupported encodings and binary control characters instead of replacing unreadable bytes. Newline style (LF, CRLF, or CR) and encoding are kept when a document is saved. Documents larger than 32 MiB are rejected.

Saving writes a temporary file beside the destination and atomically replaces the target after flushing it. Existing file permissions are retained. Opening a symlink resolves it to the target, so saving updates the target and keeps the symlink in place. Files are local; Tobywrite has no account, sync, or remote API.

Open document contents and their dirty state are snapshotted to a recovery session about 750 milliseconds after the first pending edit and when the application exits. This supports recovery of unsaved drafts after a crash. Source documents are only written to their original path after you explicitly save. If the recovery session cannot be read, Tobywrite preserves it and disables automatic session writes for that run so the damaged recovery file is not silently replaced.

Saving over a read-only or multiply hard-linked file is refused; use Save As to choose another path. This avoids replacing a file that cannot be written under its permissions or breaking another hard-link name.

On Linux, preferences are stored under `$XDG_CONFIG_HOME/tobywrite` (normally `~/.config/tobywrite`) and recovery data under `$XDG_DATA_HOME/tobywrite` (normally `~/.local/share/tobywrite`). Settings and session files are private to the current user. Quick Open indexes Markdown and text files in linked folders, skips hidden folders and symbolic links, and limits scans to 10,000 files and 10 directory levels. Search uses file names and paths; it does not index document contents.

To remove a default user install while keeping preferences and recovered drafts, remove the launcher, binary, icon, and copied license notices:

```sh
rm -f "$HOME/.local/bin/tobywrite"
data_dir="${XDG_DATA_HOME:-$HOME/.local/share}"
rm -f "$data_dir/applications/com.tobywrite.Writer.desktop"
rm -f "$data_dir/icons/hicolor/256x256/apps/com.tobywrite.Writer.png"
rm -f "$data_dir/tobywrite/LICENSE" "$data_dir/tobywrite/OFL.txt"
```

If you installed with a custom `PREFIX`, remove the binary from that prefix instead of `~/.local/bin`. These commands leave the Tobywrite settings and recovery data directories intact.

## Command line

```sh
tobywrite path/to/notes.md
tobywrite --check path/to/notes.md
tobywrite --export-html path/to/notes.md /tmp/notes.html
tobywrite --export-pdf path/to/notes.md /tmp/notes.pdf
```

`--check` validates the file encoding and reports its encoding, newline style, word count, and character count. HTML export is offline: it escapes embedded HTML, drops image previews, and only emits HTTP(S) links. PDF export needs a graphical session. Exports refuse to replace an existing destination.

## Development

Run formatting, lint, and the test suite with:

```sh
./scripts/test.sh
```

From a graphical session, run `./scripts/smoke-test.sh` to exercise the native editor, clipboard, undo, save and close dialogs, recovery, live theme changes, and PDF export. It uses temporary documents, settings, and theme fixtures. On Wayland, it restores the clipboard after the checks when `wl-clipboard` is available. Set `TOBYWRITE_THEME_CAPTURE_DIR` to a directory to save native widget screenshots during the theme checks. `TOBYWRITE_THEME_DIR` can point a development instance at a separate theme directory; the smoke script sets this automatically.

The library code is organized into `editing` for Markdown operations, `text_file` for encoding and line endings, `storage` for documents/settings/recovery and folder indexing, and `export`/`printing` for output. GTK application and editor code lives in `src/ui`.

## Credits and license

Tobywrite is an independent implementation based on the [Mac reference app](https://github.com/TobiasRoland123/tobywrite-mac), whose upstream behavior was taken from commit `683011aa296539e5f1db44bc12a02badec64310d`. The application is MIT licensed; the bundled iA Writer Mono font is licensed under SIL Open Font License 1.1. See [LICENSE](LICENSE) and [assets/fonts/OFL.txt](assets/fonts/OFL.txt).
