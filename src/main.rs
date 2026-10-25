mod fonts;

use anyhow::{Context, Result, bail};
use std::{ffi::OsString, path::Path, process::ExitCode};

fn main() -> ExitCode {
    match entry() {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            eprintln!("Tobywrite: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn entry() -> Result<u8> {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    if args.first().is_some_and(|a| a == "--version") {
        println!("Tobywrite {}", env!("CARGO_PKG_VERSION"));
        return Ok(0);
    }
    if args.first().is_some_and(|a| a == "--help" || a == "-h") {
        println!(
            "Tobywrite, a native Markdown editor for Linux\n\nUsage:\n  tobywrite [FILE ...]                  Open documents\n  tobywrite --check FILE                Check text encoding and word count\n  tobywrite --export-html INPUT OUTPUT  Export formatted, offline HTML\n  tobywrite --export-pdf INPUT OUTPUT   Export formatted PDF (needs a display)\n  tobywrite --version\n\nExports refuse to replace an existing file.\n\nEditor shortcuts: Ctrl+O Quick Open, Ctrl+Shift+O Browse, Ctrl+S Save,\nCtrl+Shift+S Save As, Ctrl+F Find, Ctrl+H Replace, Ctrl+P Print, Ctrl+, Settings."
        );
        return Ok(0);
    }
    if args.first().is_some_and(|a| a == "--check") {
        if args.len() != 2 {
            bail!("Usage: tobywrite --check FILE");
        }
        let file = load(Path::new(&args[1]))?;
        println!(
            "{} · {} · {} words · {} characters",
            file.encoding,
            file.line_ending,
            tobywrite::editing::word_count(&file.text),
            file.text.chars().count()
        );
        return Ok(0);
    }
    let _fonts = fonts::register()
        .map_err(|error| eprintln!("Tobywrite: bundled font unavailable: {error:#}"));
    if args
        .first()
        .is_some_and(|a| a == "--export-html" || a == "--export-pdf")
    {
        if args.len() != 3 {
            bail!("Usage: tobywrite --export-html/--export-pdf INPUT OUTPUT");
        }
        let input = Path::new(&args[1]);
        let output = Path::new(&args[2]);
        let file = load(input)?;
        let title = input.file_stem().unwrap_or_default().to_string_lossy();
        if args[0] == "--export-html" {
            tobywrite::export::save_html(&file.text, &title, output, false)?;
        } else {
            gtk::init().context("PDF export needs a running graphical session")?;
            tobywrite::printing::export_pdf(&file.text, &title, output)?;
        }
        println!("Exported {}", output.display());
        return Ok(0);
    }
    Ok(u8::from(tobywrite::ui::run()))
}

fn load(path: &Path) -> Result<tobywrite::text_file::TextFile> {
    let doc = tobywrite::storage::Document::open(path)?;
    Ok(doc.file)
}
