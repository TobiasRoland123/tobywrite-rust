//! Opt-in native integration checks, confined to an isolated data folder.
use super::{Writer, editor::Editor};
use crate::{
    storage::{Document, Draft},
    text_file::TextFile,
};
use anyhow::{Context, Result, ensure};
use gtk::{gdk, glib, prelude::*};
use std::{cell::RefCell, path::PathBuf, rc::Rc, time::Duration};

struct Smoke {
    state: Rc<Writer>,
    editor: Rc<Editor>,
    path: PathBuf,
    checks: RefCell<Vec<&'static str>>,
    recovered: RefCell<Option<Draft>>,
}

pub(super) fn run(state: Rc<Writer>) {
    if std::env::var_os("TOBYWRITE_DATA_DIR").is_none() {
        eprintln!("TOBYWRITE_SMOKE_FAILED: set TOBYWRITE_DATA_DIR to an isolated directory");
        state.app.quit();
        return;
    }
    let path = state.store.data_dir.join("smoke-note.md");
    let editor = state.new_document("");
    schedule(
        Rc::new(Smoke {
            state,
            editor,
            path,
            checks: RefCell::new(Vec::new()),
            recovered: RefCell::new(None),
        }),
        editing_and_paste,
    );
}

fn schedule(smoke: Rc<Smoke>, work: fn(&Rc<Smoke>) -> Result<()>) {
    glib::timeout_add_local_once(
        Duration::from_millis(200),
        move || match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(&smoke))) {
            Ok(Ok(())) => {}
            Ok(Err(error)) => finish(&smoke, Some(format!("{error:#}"))),
            Err(_) => finish(&smoke, Some("native UI assertion panicked".into())),
        },
    );
}

fn editing_and_paste(smoke: &Rc<Smoke>) -> Result<()> {
    let editor = &smoke.editor;
    editor.buffer.begin_user_action();
    editor.buffer.insert_at_cursor("Héllo κόσμε 😀\n\n- first");
    editor.buffer.end_user_action();
    ensure!(
        editor.document.borrow().dirty,
        "native insertion dirties document"
    );
    editor.buffer.select_range(
        &editor.buffer.iter_at_offset(6),
        &editor.buffer.iter_at_offset(11),
    );
    editor
        .window
        .lookup_action("bold")
        .context("bold action")?
        .activate(None);
    ensure!(
        editor.text() == "Héllo **κόσμε** 😀\n\n- first",
        "Unicode bold selection"
    );
    editor.buffer.undo();
    ensure!(editor.text() == "Héllo κόσμε 😀\n\n- first", "native undo");
    editor.buffer.redo();
    ensure!(editor.text().contains("**κόσμε**"), "native redo");
    smoke
        .checks
        .borrow_mut()
        .push("Unicode formatting, selection, native undo and redo");
    editor.find.search.set_text("κόσμε");
    editor.find.next(false);
    let (start, end) = editor
        .buffer
        .selection_bounds()
        .context("find selected match")?;
    ensure!(
        editor.buffer.text(&start, &end, true) == "κόσμε",
        "Unicode find"
    );
    editor.find.replacement.set_text("world");
    editor.find.replace_all();
    ensure!(editor.text().contains("**world**"), "replace all");
    editor.buffer.undo();
    ensure!(
        editor.text().contains("**κόσμε**"),
        "replace undo group: actual {:?}",
        editor.text()
    );
    editor.buffer.undo();
    ensure!(
        editor.text() == "Héllo κόσμε 😀\n\n- first",
        "second undo preserves previous command boundary: actual {:?}",
        editor.text()
    );
    editor.buffer.redo();
    ensure!(
        editor.text().contains("**κόσμε**"),
        "first redo restores formatting"
    );
    editor.buffer.redo();
    ensure!(
        editor.text().contains("**world**"),
        "second redo retains replacement history"
    );
    smoke
        .checks
        .borrow_mut()
        .push("Unicode find, replace, and grouped undo");
    editor.buffer.place_cursor(&editor.buffer.end_iter());
    ensure!(
        !editor.try_continue_list(gdk::ModifierType::SHIFT_MASK),
        "Shift+Return remains native"
    );
    editor.view.emit_preedit_changed("かな");
    ensure!(
        !editor.try_continue_list(gdk::ModifierType::empty()),
        "IME composition remains native"
    );
    editor.view.emit_preedit_changed("");
    ensure!(
        editor.try_continue_list(gdk::ModifierType::empty()) && editor.text().ends_with("\n- "),
        "list continuation"
    );
    ensure!(
        editor.try_continue_list(gdk::ModifierType::empty()) && !editor.text().ends_with("- "),
        "empty list exit"
    );
    smoke
        .checks
        .borrow_mut()
        .push("List continuation, empty-item exit, Shift and IME guards");
    editor
        .view
        .clipboard()
        .set_text("https://example.com/notes");
    editor.buffer.select_range(
        &editor.buffer.iter_at_offset(0),
        &editor.buffer.iter_at_offset(5),
    );
    editor.view.emit_paste_clipboard();
    schedule(smoke.clone(), save_and_conflict);
    Ok(())
}

fn save_and_conflict(smoke: &Rc<Smoke>) -> Result<()> {
    ensure!(
        smoke
            .editor
            .text()
            .starts_with("[Héllo](https://example.com/notes)"),
        "clipboard URL paste over selection"
    );
    smoke
        .checks
        .borrow_mut()
        .push("Asynchronous native clipboard URL paste");
    std::fs::create_dir_all(&smoke.state.store.data_dir)?;
    if smoke.path.exists() {
        std::fs::remove_file(&smoke.path)?;
    }
    smoke.editor.save_target(smoke.path.clone(), false, false);
    ensure!(
        !smoke.editor.document.borrow().dirty,
        "UI save clears dirty state"
    );
    ensure!(
        TextFile::decode(&std::fs::read(&smoke.path)?)?.text == smoke.editor.text(),
        "UI save retains exact source"
    );
    smoke
        .checks
        .borrow_mut()
        .push("Atomic UI save and clean document state");
    std::fs::write(&smoke.path, "External change\n")?;
    smoke
        .editor
        .buffer
        .place_cursor(&smoke.editor.buffer.end_iter());
    smoke.editor.buffer.insert_at_cursor("Local change\n");
    ensure!(
        smoke.editor.document.borrow().disk_changed()?,
        "external modification detected"
    );
    smoke.editor.save(false, false);
    schedule(smoke.clone(), cancel_conflict);
    Ok(())
}

fn cancel_conflict(smoke: &Rc<Smoke>) -> Result<()> {
    click_dialog("Cancel")?;
    schedule(smoke.clone(), retry_conflict);
    Ok(())
}
fn retry_conflict(smoke: &Rc<Smoke>) -> Result<()> {
    ensure!(
        std::fs::read_to_string(&smoke.path)? == "External change\n",
        "cancel conflict preserves disk contents"
    );
    ensure!(
        smoke.editor.document.borrow().dirty && smoke.editor.text().contains("Local change"),
        "cancel conflict keeps draft"
    );
    smoke.editor.save(false, false);
    schedule(smoke.clone(), force_conflict);
    Ok(())
}
fn force_conflict(smoke: &Rc<Smoke>) -> Result<()> {
    click_dialog("Save Anyway")?;
    schedule(smoke.clone(), recovery_and_encoding);
    Ok(())
}

fn recovery_and_encoding(smoke: &Rc<Smoke>) -> Result<()> {
    ensure!(
        std::fs::read_to_string(&smoke.path)? == smoke.editor.text()
            && !smoke.editor.document.borrow().dirty,
        "confirmed conflict overwrite"
    );
    smoke
        .checks
        .borrow_mut()
        .push("Real conflict dialogs: Cancel and Save Anyway");
    smoke
        .editor
        .buffer
        .insert_at_cursor("Recover this unsaved sentence.\n");
    smoke.state.snapshot();
    let draft = smoke
        .state
        .store
        .load_session()?
        .into_iter()
        .find(|draft| draft.path.as_ref() == Some(&smoke.path))
        .context("saved recovery draft")?;
    ensure!(
        draft.dirty && draft.file.text == smoke.editor.text(),
        "session captures unsaved source"
    );
    let restored = Document::from_draft(draft.clone());
    ensure!(
        restored.dirty && restored.file.text == smoke.editor.text(),
        "draft restoration"
    );
    *smoke.recovered.borrow_mut() = Some(draft);
    smoke
        .checks
        .borrow_mut()
        .push("Session snapshot and dirty draft restoration");
    let source = smoke.editor.text();
    smoke.state.update_preferences(|prefs| {
        prefs.focus_mode = true;
        prefs.show_syntax = false;
    });
    smoke.editor.refresh_style();
    smoke.state.update_preferences(|prefs| {
        prefs.focus_mode = false;
        prefs.show_syntax = true;
    });
    ensure!(
        smoke.editor.text() == source,
        "styling never changes source"
    );
    smoke
        .checks
        .borrow_mut()
        .push("Markdown styling and focus preserve source");
    let utf16 = smoke.state.store.data_dir.join("smoke-utf16.md");
    let mut bytes = vec![0xff, 0xfe];
    for unit in "# Héllo\r\n世界\r\n".encode_utf16() {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    std::fs::write(&utf16, bytes)?;
    let encoded = smoke.state.add_document(Document::open(&utf16)?);
    encoded.buffer.place_cursor(&encoded.buffer.end_iter());
    encoded.buffer.insert_at_cursor("Another line\n");
    encoded.save_target(utf16.clone(), false, false);
    let bytes = std::fs::read(&utf16)?;
    ensure!(bytes.starts_with(&[0xff, 0xfe]), "UTF-16 BOM preserved");
    let units = bytes[2..]
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect::<Vec<_>>();
    ensure!(
        String::from_utf16(&units)? == "# Héllo\r\n世界\r\nAnother line\r\n",
        "UTF-16 and CRLF preserved"
    );
    smoke
        .checks
        .borrow_mut()
        .push("Native editing preserves UTF-16 BOM and CRLF");
    smoke.editor.window.close();
    schedule(smoke.clone(), cancel_close);
    Ok(())
}

fn cancel_close(smoke: &Rc<Smoke>) -> Result<()> {
    click_dialog("Cancel")?;
    schedule(smoke.clone(), retry_close);
    Ok(())
}
fn retry_close(smoke: &Rc<Smoke>) -> Result<()> {
    ensure!(
        smoke.editor.window.is_visible() && smoke.editor.document.borrow().dirty,
        "cancel close keeps unsaved document open"
    );
    smoke.editor.window.close();
    schedule(smoke.clone(), discard_close);
    Ok(())
}
fn discard_close(smoke: &Rc<Smoke>) -> Result<()> {
    click_dialog("Discard Changes")?;
    schedule(smoke.clone(), complete);
    Ok(())
}
fn complete(smoke: &Rc<Smoke>) -> Result<()> {
    ensure!(
        !smoke
            .state
            .documents
            .borrow()
            .iter()
            .any(|editor| editor.window == smoke.editor.window),
        "discard closes document"
    );
    ensure!(
        !smoke
            .state
            .store
            .load_session()?
            .iter()
            .any(|draft| draft.path.as_ref() == Some(&smoke.path)),
        "discarded draft removed from recovery"
    );
    smoke
        .checks
        .borrow_mut()
        .push("Real unsaved-close dialogs: Cancel and Discard Changes");
    let draft = smoke
        .recovered
        .borrow_mut()
        .take()
        .context("recovery fixture")?;
    smoke.state.add_document(Document::from_draft(draft));
    smoke.state.snapshot();
    let smoke = smoke.clone();
    super::smoke_theme::run(smoke.state.clone(), move |result| match result {
        Ok(checks) => {
            smoke.checks.borrow_mut().extend(checks);
            finish(&smoke, None);
        }
        Err(error) => finish(&smoke, Some(format!("{error:#}"))),
    });
    Ok(())
}

fn click_dialog(label: &str) -> Result<()> {
    fn find(widget: &gtk::Widget, label: &str) -> Option<gtk::Button> {
        if let Some(button) = widget.downcast_ref::<gtk::Button>()
            && button.label().as_deref() == Some(label)
        {
            return Some(button.clone());
        }
        let mut child = widget.first_child();
        while let Some(widget) = child {
            if let Some(found) = find(&widget, label) {
                return Some(found);
            }
            child = widget.next_sibling();
        }
        None
    }
    let button = gtk::Window::list_toplevels()
        .iter()
        .filter(|window| window.is_visible())
        .find_map(|window| find(window, label))
        .with_context(|| format!("native dialog button {label:?} not shown"))?;
    button.emit_clicked();
    Ok(())
}

fn finish(smoke: &Smoke, error: Option<String>) {
    let report = serde_json::json!({ "ok": error.is_none(), "checks": *smoke.checks.borrow(), "error": error });
    let path = std::env::var_os("TOBYWRITE_SMOKE_REPORT")
        .map(PathBuf::from)
        .unwrap_or_else(|| smoke.state.store.data_dir.join("smoke-report.json"));
    if let Err(error) = std::fs::write(
        &path,
        serde_json::to_vec_pretty(&report).unwrap_or_default(),
    ) {
        eprintln!("Could not write smoke report: {error}");
    }
    if let Some(error) = error {
        eprintln!("TOBYWRITE_SMOKE_FAILED: {error}");
    } else {
        eprintln!(
            "TOBYWRITE_SMOKE_OK: {} native checks passed",
            smoke.checks.borrow().len()
        );
    }
    smoke.state.app.quit();
}
