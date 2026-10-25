use super::{Writer, dialogs, editor::Editor};
use crate::storage::{Document, FileEntry, discover_files, search_files};
use gtk::{gdk, glib, prelude::*};
use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
};

#[derive(Clone, Debug)]
enum QuickOpenItem {
    File(FileEntry),
    Create { path: PathBuf, name: String },
}

pub fn show(state: &Rc<Writer>, parent: &Rc<Editor>) {
    let window = gtk::Window::builder()
        .title("Quick Open")
        .transient_for(&parent.window)
        .modal(true)
        .default_width(690)
        .default_height(540)
        .build();
    window.add_css_class("writer");

    let content = gtk::Box::new(gtk::Orientation::Vertical, 12);
    content.set_margin_top(18);
    content.set_margin_bottom(18);
    content.set_margin_start(18);
    content.set_margin_end(18);

    let search = gtk::SearchEntry::new();
    search.set_placeholder_text(Some("Search filenames and folders…"));
    content.append(&search);

    let list = gtk::ListBox::new();
    list.set_selection_mode(gtk::SelectionMode::Single);
    list.set_activate_on_single_click(false);
    let scroll = gtk::ScrolledWindow::builder()
        .child(&list)
        .vexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .build();
    content.append(&scroll);

    let footer = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let status = gtk::Label::new(Some("Searching linked folders…"));
    status.add_css_class("dim-label");
    status.set_hexpand(true);
    status.set_xalign(0.);
    let browse = gtk::Button::with_label("Browse…");
    let link = gtk::Button::with_label("Link Folder…");
    footer.append(&status);
    footer.append(&link);
    footer.append(&browse);
    content.append(&footer);
    window.set_child(Some(&content));

    let files = Rc::new(RefCell::new(Vec::<FileEntry>::new()));
    let visible = Rc::new(RefCell::new(Vec::<QuickOpenItem>::new()));
    let first_folder = state.preferences.borrow().linked_folders.first().cloned();
    let refresh: Rc<dyn Fn()> = {
        let files = files.clone();
        let visible = visible.clone();
        let list = list.clone();
        let search = search.clone();
        let status = status.clone();
        let first_folder = first_folder.clone();
        Rc::new(move || {
            while let Some(child) = list.first_child() {
                list.remove(&child);
            }

            let query = search.text().to_string();
            let mut items: Vec<_> = search_files(&files.borrow(), &query)
                .into_iter()
                .map(QuickOpenItem::File)
                .collect();
            let create = create_candidate(&query, first_folder.as_deref(), &files.borrow());
            if let Some((path, name)) = create {
                items.push(QuickOpenItem::Create { path, name });
            }

            for item in &items {
                let row = gtk::Box::new(gtk::Orientation::Vertical, 4);
                row.set_margin_top(10);
                row.set_margin_bottom(10);
                row.set_margin_start(12);
                row.set_margin_end(12);
                let (title, path) = match item {
                    QuickOpenItem::File(entry) => (
                        entry.title.clone(),
                        entry.path.to_string_lossy().into_owned(),
                    ),
                    QuickOpenItem::Create { path, name } => (
                        format!("Create {name}"),
                        path.parent().unwrap_or(path).to_string_lossy().into_owned(),
                    ),
                };
                let title = gtk::Label::new(Some(&title));
                title.set_xalign(0.);
                title.set_ellipsize(gtk::pango::EllipsizeMode::End);
                let subtitle = gtk::Label::new(Some(&path));
                subtitle.set_xalign(0.);
                subtitle.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
                subtitle.add_css_class("library-path");
                row.append(&title);
                row.append(&subtitle);
                row.set_tooltip_text(Some(&path));
                list.append(&row);
            }

            let message = if items.is_empty() {
                "No matches — browse or link a folder"
            } else if matches!(items.last(), Some(QuickOpenItem::Create { .. })) {
                "Enter opens or creates · Ctrl+N creates · Ctrl+D links a folder · Esc closes"
            } else {
                "Enter opens · Ctrl+N creates a named note · Ctrl+D links a folder · Esc closes"
            };
            status.set_text(message);
            *visible.borrow_mut() = items;
            if let Some(first) = list.row_at_index(0) {
                list.select_row(Some(&first));
            }
        })
    };

    let update = refresh.clone();
    search.connect_search_changed(move |_| update());

    let weak_state = Rc::downgrade(state);
    let items = visible.clone();
    let dialog = window.clone();
    list.connect_row_activated(move |_, row| {
        let action = items.borrow().get(row.index() as usize).cloned();
        if let (Some(state), Some(action)) = (weak_state.upgrade(), action) {
            activate_item(&state, &dialog, action);
        }
    });

    let weak_editor = Rc::downgrade(parent);
    let dialog = window.clone();
    browse.connect_clicked(move |_| {
        dialog.close();
        if let Some(editor) = weak_editor.upgrade() {
            editor.browse();
        }
    });

    let weak_state = Rc::downgrade(state);
    let main_window = parent.window.clone();
    let dialog = window.clone();
    link.connect_clicked(move |_| {
        dialog.close();
        if let Some(state) = weak_state.upgrade() {
            state.link_folder(&main_window);
        }
    });

    let cancelled = Arc::new(AtomicBool::new(false));
    let cancel_on_close = cancelled.clone();
    window.connect_close_request(move |_| {
        cancel_on_close.store(true, Ordering::Relaxed);
        glib::Propagation::Proceed
    });

    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let dialog = window.clone();
    let results = list.clone();
    let visible_items = visible.clone();
    let weak_state = Rc::downgrade(state);
    let folder_parent = parent.window.clone();
    let search_for_keys = search.clone();
    let files_for_keys = files.clone();
    let folder_for_keys = first_folder.clone();
    keys.connect_key_pressed(move |_, key, _, modifiers| {
        if key == gdk::Key::Escape {
            dialog.close();
            return glib::Propagation::Stop;
        }

        let control = modifiers.contains(gdk::ModifierType::CONTROL_MASK);
        if control && key == gdk::Key::n {
            if let (Some(state), Some((path, name))) = (
                weak_state.upgrade(),
                create_candidate(
                    &search_for_keys.text(),
                    folder_for_keys.as_deref(),
                    &files_for_keys.borrow(),
                ),
            ) {
                activate_item(&state, &dialog, QuickOpenItem::Create { path, name });
            }
            return glib::Propagation::Stop;
        }
        if control && key == gdk::Key::d {
            dialog.close();
            if let Some(state) = weak_state.upgrade() {
                state.link_folder(&folder_parent);
            }
            return glib::Propagation::Stop;
        }

        match key {
            gdk::Key::Down | gdk::Key::Up => {
                let count = visible_items.borrow().len() as i32;
                if count > 0 {
                    let current = results.selected_row().map(|row| row.index());
                    let next = match (key, current) {
                        (gdk::Key::Down, Some(index)) => (index + 1).min(count - 1),
                        (gdk::Key::Up, Some(index)) => (index - 1).max(0),
                        (gdk::Key::Down, None) => 0,
                        (_, None) => count - 1,
                        _ => 0,
                    };
                    if let Some(row) = results.row_at_index(next) {
                        results.select_row(Some(&row));
                    }
                }
                glib::Propagation::Stop
            }
            gdk::Key::Return | gdk::Key::KP_Enter => {
                if let Some(row) = results.selected_row() {
                    if let (Some(state), Some(action)) = (
                        weak_state.upgrade(),
                        visible_items.borrow().get(row.index() as usize).cloned(),
                    ) {
                        activate_item(&state, &dialog, action);
                    }
                    glib::Propagation::Stop
                } else {
                    glib::Propagation::Proceed
                }
            }
            _ => glib::Propagation::Proceed,
        }
    });
    window.add_controller(keys);

    let folders = state.preferences.borrow().linked_folders.clone();
    let recents = state.recent_paths();
    let (send, receive) = mpsc::sync_channel(1);
    let worker_cancelled = cancelled.clone();
    std::thread::spawn(move || {
        if worker_cancelled.load(Ordering::Relaxed) {
            return;
        }
        let found = discover_files(&folders, &recents);
        if !worker_cancelled.load(Ordering::Relaxed) {
            let _ = send.send(found);
        }
    });

    let weak_window = window.downgrade();
    glib::timeout_add_local(Duration::from_millis(50), move || {
        if weak_window
            .upgrade()
            .is_none_or(|window| !window.is_visible())
        {
            cancelled.store(true, Ordering::Relaxed);
            return glib::ControlFlow::Break;
        }
        match receive.try_recv() {
            Ok(found) => {
                *files.borrow_mut() = found;
                refresh();
                glib::ControlFlow::Break
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                status.set_text("Folder search did not finish. Try browsing for a file.");
                glib::ControlFlow::Break
            }
            Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
        }
    });

    window.present();
    search.grab_focus();
}

fn activate_item(state: &Rc<Writer>, dialog: &gtk::Window, item: QuickOpenItem) {
    match item {
        QuickOpenItem::File(entry) => {
            dialog.close();
            state.open(&entry.path);
        }
        QuickOpenItem::Create { path, .. } => {
            let mut document = Document::new();
            match document.save(&path, false) {
                Ok(()) => {
                    dialog.close();
                    state.open(&path);
                }
                Err(error) => dialogs::error(
                    Some(dialog),
                    "Could not create note",
                    &format!("{}\n\n{error:#}", path.display()),
                ),
            }
        }
    }
}

fn create_candidate(
    query: &str,
    first_folder: Option<&Path>,
    files: &[FileEntry],
) -> Option<(PathBuf, String)> {
    let folder = first_folder?;
    if !folder.is_dir() {
        return None;
    }
    let name = safe_note_name(query)?;
    let path = folder.join(&name);
    let leaf = Path::new(&name).file_name()?;
    if path.exists() || files.iter().any(|file| file.path.file_name() == Some(leaf)) {
        return None;
    }
    Some((path, name))
}

fn safe_note_name(query: &str) -> Option<String> {
    let name = query.trim();
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.starts_with('/')
        || name.starts_with('~')
        || name.contains('/')
        || name.contains('\\')
        || name.chars().any(char::is_control)
    {
        return None;
    }
    let extension = Path::new(name)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase);
    let supported = matches!(
        extension.as_deref(),
        Some("md" | "markdown" | "mdown" | "txt" | "text")
    );
    if supported || extension.is_none() {
        Some(if extension.is_none() {
            format!("{name}.md")
        } else {
            name.to_owned()
        })
    } else {
        Some(format!("{name}.md"))
    }
}

#[cfg(test)]
mod tests {
    use super::{create_candidate, safe_note_name};
    use crate::storage::FileEntry;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn note_names_get_markdown_extension_without_allowing_paths() {
        assert_eq!(
            safe_note_name(" Meeting notes ").as_deref(),
            Some("Meeting notes.md")
        );
        assert_eq!(safe_note_name("ideas.txt").as_deref(), Some("ideas.txt"));
        assert_eq!(safe_note_name("ideas.pdf").as_deref(), Some("ideas.pdf.md"));
        for invalid in [
            "",
            " ",
            ".",
            "..",
            "../escape",
            "/tmp/escape",
            "~/escape",
            "a\\b",
            "line\nbreak",
        ] {
            assert!(safe_note_name(invalid).is_none(), "accepted {invalid:?}");
        }
    }

    #[test]
    fn new_note_targets_only_the_first_linked_folder_and_never_duplicates() {
        let dir = tempdir().unwrap();
        let first = dir.path().join("first");
        let second = dir.path().join("second");
        fs::create_dir_all(&first).unwrap();
        fs::create_dir_all(&second).unwrap();

        let candidate = create_candidate("New note", Some(&first), &[]).unwrap();
        assert_eq!(candidate.0, first.join("New note.md"));
        let indexed = FileEntry {
            path: second.join("Existing.md"),
            title: "Existing".into(),
            modified: 0,
        };
        assert!(create_candidate("Existing.md", Some(&first), &[indexed]).is_none());
        fs::write(first.join("New note.md"), b"").unwrap();
        assert!(create_candidate("New note", Some(&first), &[]).is_none());
        assert!(create_candidate("New note", None, &[]).is_none());
    }
}
