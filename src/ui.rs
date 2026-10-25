//! Native GTK document windows. All document changes pass through the same buffer.
mod appearance;
mod dialogs;
mod editor;
mod find;
mod library;
mod palette;
mod smoke;
mod smoke_theme;
mod style;

use crate::storage::{Appearance, Document, Preferences, Store, remember_recent};
use editor::Editor;
use gtk::{gio, glib, prelude::*};
use palette::{Palette, ThemeSource};
use std::{
    cell::{Cell, RefCell},
    path::{Path, PathBuf},
    rc::Rc,
    time::Duration,
};

/// Keep each committed command separate from a previously redone compound edit.
pub(super) fn begin_edit(buffer: &gtk::TextBuffer) {
    // GTK 4.24 can leave a restored action group at the undo tail after redo.
    // An empty pair seals that group before the actual edit begins. Only do this
    // when committing a new edit: doing it in a redo hook would erase redo history.
    buffer.begin_user_action();
    buffer.end_user_action();
    buffer.begin_user_action();
}

pub(super) struct Writer {
    app: gtk::Application,
    store: Store,
    preferences: RefCell<Preferences>,
    documents: RefCell<Vec<Rc<Editor>>>,
    restored: Cell<bool>,
    session_safe: Cell<bool>,
    snapshot_pending: Cell<bool>,
    snapshot_error_shown: Cell<bool>,
    system_dark: Cell<bool>,
    palette: RefCell<Palette>,
    omarchy_palette: RefCell<Option<Palette>>,
    theme_source: RefCell<ThemeSource>,
    applied_appearance: Cell<Appearance>,
    gtk_override: Cell<Option<bool>>,
    appearance_updating: Cell<bool>,
    theme_poll: RefCell<Option<glib::SourceId>>,
    appearance_signals: RefCell<Vec<(gtk::Settings, glib::SignalHandlerId)>>,
    css: gtk::CssProvider,
    startup_error: RefCell<Option<String>>,
}

pub fn run() -> glib::ExitCode {
    if let Err(error) = gtk::init() {
        eprintln!("Cannot open the desktop display: {error}");
        return glib::ExitCode::FAILURE;
    }
    let store = match std::env::var_os("TOBYWRITE_DATA_DIR") {
        Some(path) => Store::at(Path::new(&path)),
        None => Store::new(),
    };
    let store = match store {
        Ok(store) => store,
        Err(error) => {
            eprintln!("Cannot start Tobywrite: {error:#}");
            return glib::ExitCode::FAILURE;
        }
    };
    let (preferences, startup_error) = match store.load_preferences() {
        Ok(preferences) => (preferences, None),
        Err(error) => (
            Preferences::default(),
            Some(format!(
                "Settings could not be read. Default settings are in use.\n\n{error:#}"
            )),
        ),
    };
    let mut flags = gio::ApplicationFlags::HANDLES_OPEN;
    if std::env::var_os("TOBYWRITE_NON_UNIQUE").is_some() {
        flags |= gio::ApplicationFlags::NON_UNIQUE;
    }
    let app = gtk::Application::builder()
        .application_id("com.tobywrite.Writer")
        .flags(flags)
        .build();
    let system_dark = gtk::Settings::default()
        .as_ref()
        .is_some_and(appearance::system_dark);
    let mut theme_source = ThemeSource::new(palette::theme_paths());
    let omarchy_palette = theme_source.read();
    let palette = appearance::palette(
        preferences.appearance,
        omarchy_palette.as_ref(),
        system_dark,
    );
    let writer = Rc::new(Writer {
        app: app.clone(),
        store,
        preferences: RefCell::new(preferences),
        documents: RefCell::new(Vec::new()),
        restored: Cell::new(false),
        session_safe: Cell::new(true),
        snapshot_pending: Cell::new(false),
        snapshot_error_shown: Cell::new(false),
        system_dark: Cell::new(system_dark),
        palette: RefCell::new(palette),
        omarchy_palette: RefCell::new(omarchy_palette),
        theme_source: RefCell::new(theme_source),
        applied_appearance: Cell::new(Appearance::System),
        gtk_override: Cell::new(None),
        appearance_updating: Cell::new(false),
        theme_poll: RefCell::new(None),
        appearance_signals: RefCell::new(Vec::new()),
        css: gtk::CssProvider::new(),
        startup_error: RefCell::new(startup_error),
    });
    let state = writer.clone();
    app.connect_startup(move |_| {
        if let Some(display) = gtk::gdk::Display::default() {
            gtk::style_context_add_provider_for_display(
                &display,
                &state.css,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }
        state.apply_preferences();
        state.watch_appearance();
        for (action, keys) in [
            ("win.new", &["<Primary>n"][..]),
            ("win.open", &["<Primary>o"]),
            ("win.browse", &["<Primary><Shift>o"]),
            ("win.save", &["<Primary>s"]),
            ("win.save-as", &["<Primary><Shift>s"]),
            ("win.close", &["<Primary>w"]),
            ("win.find", &["<Primary>f"]),
            ("win.replace", &["<Primary>h"]),
            ("win.next", &["<Primary>g"]),
            ("win.previous", &["<Primary><Shift>g"]),
            ("win.bold", &["<Primary>b"]),
            ("win.italic", &["<Primary>i"]),
            ("win.link", &["<Primary>k"]),
            ("win.code", &["<Primary><Shift>k"]),
            ("win.focus", &["<Primary><Shift>d"]),
            ("win.syntax", &["<Primary><Shift>m"]),
            ("win.settings", &["<Primary>comma"]),
            ("win.print", &["<Primary>p"]),
            ("win.larger", &["<Primary>plus", "<Primary>equal"]),
            ("win.smaller", &["<Primary>minus"]),
            ("win.actual-size", &["<Primary>0"]),
            ("win.link-folder", &["<Primary>d"]),
            ("win.shortcuts", &["<Primary><Shift>question"]),
        ] {
            state.app.set_accels_for_action(action, keys);
        }
    });
    let state = writer.clone();
    app.connect_activate(move |_| {
        state.restore(true);
        if state.documents.borrow().is_empty() {
            state.new_document("");
        }
        if let Some(editor) = state.documents.borrow().last() {
            editor.window.present();
        }
        state.show_startup_error();
        if std::env::var_os("TOBYWRITE_SMOKE_TEST").is_some() {
            smoke::run(state.clone());
        }
    });
    let state = writer.clone();
    app.connect_open(move |_, files, _| {
        state.restore(false);
        for file in files {
            if let Some(path) = file.path() {
                state.open(&path);
            }
        }
        if state.documents.borrow().is_empty() {
            state.new_document("");
        }
        state.show_startup_error();
    });
    let state = writer.clone();
    app.connect_shutdown(move |_| {
        state.stop_appearance_watch();
        state.snapshot();
    });
    app.run()
}

impl Writer {
    fn restore(self: &Rc<Self>, clean_documents: bool) {
        if self.restored.replace(true) {
            return;
        }
        match self.store.load_session() {
            Ok(drafts) => {
                for draft in drafts {
                    if draft.dirty {
                        self.add_document(Document::from_draft(draft));
                    } else if clean_documents
                        && let Some(path) = draft.path
                        && path.is_file()
                    {
                        self.open(&path);
                    }
                }
            }
            Err(error) => {
                // Never replace an unreadable recovery file with an empty session.
                self.session_safe.set(false);
                *self.startup_error.borrow_mut() = Some(format!(
                    "Your recovery session could not be read. Its original file has been kept at {}. Automatic draft recovery is disabled for this run; save your work to a file.\n\n{error:#}",
                    self.store.data_dir.join("session.json").display()
                ));
            }
        }
    }

    fn show_startup_error(&self) {
        if let Some(message) = self.startup_error.borrow_mut().take() {
            dialogs::error(
                self.app.active_window().as_ref(),
                "Tobywrite needs your attention",
                &message,
            );
        }
    }

    fn add_document(self: &Rc<Self>, document: Document) -> Rc<Editor> {
        let editor = Editor::new(self, document);
        self.documents.borrow_mut().push(editor.clone());
        editor.window.present();
        editor.view.grab_focus();
        self.schedule_snapshot();
        editor
    }

    fn new_document(self: &Rc<Self>, text: &str) -> Rc<Editor> {
        let mut document = Document::new();
        if !text.is_empty() {
            document.set_text(text.to_owned());
        }
        self.add_document(document)
    }

    fn open(self: &Rc<Self>, path: &Path) {
        let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        if let Some(editor) = self
            .documents
            .borrow()
            .iter()
            .find(|editor| editor.document.borrow().path.as_ref() == Some(&canonical))
        {
            editor.window.present();
            return;
        }
        match Document::open(path) {
            Ok(document) => {
                if let Some(path) = &document.path {
                    remember_recent(&mut self.preferences.borrow_mut(), path);
                }
                self.save_preferences();
                self.add_document(document);
            }
            Err(error) => dialogs::error(
                self.app.active_window().as_ref(),
                "Could not open document",
                &format!("{error:#}"),
            ),
        }
    }

    fn remove_document(&self, editor: &Editor) {
        self.documents
            .borrow_mut()
            .retain(|item| item.window != editor.window);
        self.snapshot();
    }

    fn schedule_snapshot(self: &Rc<Self>) {
        if self.snapshot_pending.replace(true) {
            return;
        }
        let weak = Rc::downgrade(self);
        glib::timeout_add_local_once(Duration::from_millis(750), move || {
            if let Some(state) = weak.upgrade() {
                state.snapshot_pending.set(false);
                state.snapshot();
            }
        });
    }

    fn snapshot(&self) {
        if !self.session_safe.get() {
            return;
        }
        let drafts = self
            .documents
            .borrow()
            .iter()
            .map(|editor| editor.document.borrow().draft())
            .collect::<Vec<_>>();
        if let Err(error) = self.store.save_session(&drafts) {
            if !self.snapshot_error_shown.replace(true) {
                dialogs::error(
                    self.app.active_window().as_ref(),
                    "Draft recovery could not be saved",
                    &format!("Please save your work to a file.\n\n{error:#}"),
                );
            }
        } else {
            self.snapshot_error_shown.set(false);
        }
    }

    fn save_preferences(&self) {
        if let Err(error) = self.store.save_preferences(&self.preferences.borrow()) {
            dialogs::error(
                self.app.active_window().as_ref(),
                "Could not save settings",
                &format!("{error:#}"),
            );
        }
    }

    fn watch_appearance(self: &Rc<Self>) {
        if let Some(settings) = gtk::Settings::default() {
            for property in [
                "gtk-interface-color-scheme",
                "gtk-application-prefer-dark-theme",
                "gtk-theme-name",
            ] {
                if settings.find_property(property).is_none() {
                    continue;
                }
                let weak = Rc::downgrade(self);
                let id = settings.connect_notify_local(Some(property), move |settings, _| {
                    if let Some(state) = weak.upgrade()
                        && !state.appearance_updating.get()
                        && state.gtk_override.get().is_none()
                    {
                        state.system_dark.set(appearance::system_dark(settings));
                        state.refresh_appearance();
                    }
                });
                self.appearance_signals
                    .borrow_mut()
                    .push((settings.clone(), id));
            }
        }
        let weak = Rc::downgrade(self);
        let source = glib::timeout_add_local(Duration::from_secs(1), move || {
            let Some(state) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            state.refresh_theme();
            glib::ControlFlow::Continue
        });
        *self.theme_poll.borrow_mut() = Some(source);
    }

    fn stop_appearance_watch(&self) {
        if let Some(source) = self.theme_poll.borrow_mut().take() {
            source.remove();
        }
        for (settings, id) in self.appearance_signals.borrow_mut().drain(..) {
            settings.disconnect(id);
        }
    }

    fn refresh_theme(&self) {
        // Omarchy replaces its current theme directory while applying a theme.
        // Keep the last valid palette through missing or partly written files.
        if let Some(palette) = self.theme_source.borrow_mut().read()
            && self.omarchy_palette.borrow().as_ref() != Some(&palette)
        {
            *self.omarchy_palette.borrow_mut() = Some(palette);
            self.refresh_appearance();
        }
    }

    fn sync_native_appearance(&self) {
        let mode = self.preferences.borrow().appearance;
        let returning_to_system = self.applied_appearance.replace(mode) != Appearance::System
            && mode == Appearance::System;
        let target = match mode {
            Appearance::System => self
                .omarchy_palette
                .borrow()
                .as_ref()
                .map(|palette| palette.dark),
            Appearance::Light => Some(false),
            Appearance::Dark => Some(true),
        };
        if self.gtk_override.get() == target && !returning_to_system {
            return;
        }
        if let Some(settings) = gtk::Settings::default() {
            // GtkSettings emits notify synchronously for our local overrides.
            // Never mistake these notifications for a desktop preference change.
            self.appearance_updating.set(true);
            if returning_to_system || target.is_none() {
                appearance::reset_dark(&settings);
                self.system_dark.set(appearance::system_dark(&settings));
                self.gtk_override.set(None);
            }
            if let Some(dark) = target {
                appearance::set_dark(&settings, dark);
                self.gtk_override.set(Some(dark));
            }
            self.appearance_updating.set(false);
        }
    }

    fn selected_palette(&self) -> Palette {
        appearance::palette(
            self.preferences.borrow().appearance,
            self.omarchy_palette.borrow().as_ref(),
            self.system_dark.get(),
        )
    }

    fn refresh_appearance(&self) {
        self.sync_native_appearance();
        let palette = self.selected_palette();
        if *self.palette.borrow() == palette {
            return;
        }
        *self.palette.borrow_mut() = palette;
        self.apply_editor_preferences();
    }

    fn apply_preferences(&self) {
        self.sync_native_appearance();
        *self.palette.borrow_mut() = self.selected_palette();
        self.apply_editor_preferences();
    }

    fn apply_editor_preferences(&self) {
        self.css.load_from_data(&style::css(
            &self.palette.borrow(),
            self.preferences.borrow().font_size,
        ));
        for editor in self.documents.borrow().iter() {
            editor.refresh_style();
            editor.update_toggles();
        }
    }

    fn update_preferences(&self, update: impl FnOnce(&mut Preferences)) {
        update(&mut self.preferences.borrow_mut());
        self.save_preferences();
        self.apply_preferences();
    }

    fn link_folder(self: &Rc<Self>, parent: &gtk::ApplicationWindow) {
        let dialog = gtk::FileDialog::builder()
            .title("Link a writing folder")
            .build();
        let weak = Rc::downgrade(self);
        dialog.select_folder(Some(parent), None::<&gio::Cancellable>, move |result| {
            if let (Some(state), Ok(file)) = (weak.upgrade(), result)
                && let Some(path) = file.path()
            {
                state.update_preferences(|prefs| {
                    if !prefs.linked_folders.contains(&path) {
                        prefs.linked_folders.push(path);
                    }
                });
            }
        });
    }

    fn recent_paths(&self) -> Vec<PathBuf> {
        self.preferences.borrow().recent_files.clone()
    }
}
