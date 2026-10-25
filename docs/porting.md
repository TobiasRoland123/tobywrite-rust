# Mac-to-Linux port notes

Tobywrite preserves the Mac reference app’s writing model: documents are Markdown source, formatting remains in the text, the writing surface stays quiet and centered, and files remain ordinary local text files. The Linux implementation uses Rust and GTK4 and follows Linux keyboard conventions.

## Behavior carried over

- UTF-8, UTF-8 BOM, and BOM-marked UTF-16 LE/BE documents retain their original encoding and detected line endings. Unsupported or binary content is rejected rather than decoded with replacement characters.
- Bold, italic, link, and inline-code formatting operate on the Markdown source. URL insertion accepts explicit HTTP(S) URLs with hosts and escapes label and destination delimiters. List continuation handles indentation, numbered items, task markers, empty-item exit, and fenced code. Selection offsets are Unicode-safe.
- Headings, emphasis, links, lists, quotes, code, strikethrough, and dividers receive visual styling while their source stays editable. Syntax markers fade outside the active paragraph unless syntax display is enabled. Focus mode dims other paragraphs.
- Documents can be opened in separate windows, have a recovery snapshot, and can be explicitly saved. Save conflicts are checked using a SHA-256 digest of the last successfully opened or saved bytes. Replacing an existing file is an explicit action.
- Saving over a read-only or multiply hard-linked source is refused, so Save As is required for those files.
- Linked-folder indexing is limited to Markdown and text extensions, skips hidden folders and symlinks, stops at 10 levels and 10,000 files, and ranks Quick Open results using names, paths, recency, and modified time.
- Formatted printing, offline HTML export, and PDF export are available. HTML export escapes source HTML and omits image previews, so opening an export does not fetch document images.

## Linux-specific choices

- GTK4 supplies the application windows, text selection, undo, input-method support, file dialogs, and print dialog. The app targets GTK 4.10 or newer and Wayland.
- App shortcuts use Ctrl in place of Command. For example, Ctrl+S saves and Ctrl+Shift+S opens Save As.
- The iA Writer Mono font files are registered with Fontconfig only for the Tobywrite process. Installing or launching the app does not add a system-wide font or alter desktop font settings.
- Preferences and recovery data use XDG directories, with private user-only settings and session files. Recovery snapshots are written about 750 ms after the first pending edit and at application shutdown; source documents are not autosaved to their original paths.

## Differences from the Mac app

- macOS iCloud document coordination, version history, native conflict dialogs, Finder integration, and app-icon switching have no direct Linux equivalent in this port. Tobywrite detects changed source files and offers an explicit overwrite path.
- Spell checking depends on platform text services in the Mac app. The GTK port does not currently connect to a Linux spell-check service.
- Horizontal rules keep their original Markdown characters in the editor and receive visual styling. The divider line is not replaced by a rendered object.
- HTML export and PDF export are Linux-specific additions to the command-line workflow. HTML is self-contained and offline; PDF export needs a graphical session.
- The app stores recovery snapshots and preferences locally. It does not sync documents or settings to a service.

## Reference and licenses

This port is an independent implementation based on the [Mac reference app](https://github.com/TobiasRoland123/tobywrite-mac) at commit `683011aa296539e5f1db44bc12a02badec64310d`. Its MIT license is retained in the repository. The bundled iA Writer Mono font is distributed under SIL Open Font License 1.1; its notice is in `assets/fonts/OFL.txt`.
