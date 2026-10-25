use super::{Writer, dialogs, find::FindBar, library, style::Styler};
use crate::{
    editing,
    storage::{Document, remember_recent},
};
use gtk::{gdk, gio, glib, prelude::*};
use std::{
    cell::{Cell, RefCell},
    ops::Range,
    path::{Path, PathBuf},
    rc::{Rc, Weak},
    time::Duration,
};

pub(super) struct Editor {
    pub window: gtk::ApplicationWindow,
    pub view: gtk::TextView,
    pub buffer: gtk::TextBuffer,
    pub document: RefCell<Document>,
    pub find: Rc<FindBar>,
    writer: Weak<Writer>,
    title: gtk::Label,
    subtitle: gtk::Label,
    status: gtk::Label,
    focus_button: gtk::ToggleButton,
    styler: Styler,
    clean_text: RefCell<Option<String>>,
    resetting: Cell<bool>,
    refresh_pending: Cell<bool>,
    syntax_dirty: Cell<bool>,
    counts: Cell<(usize, usize)>,
    pub allow_close: Cell<bool>,
    close_pending: Cell<bool>,
    saving: Cell<bool>,
    preedit: Cell<bool>,
}

impl Editor {
    pub fn new(writer: &Rc<Writer>, document: Document) -> Rc<Self> {
        let window = gtk::ApplicationWindow::builder()
            .application(&writer.app)
            .title("Tobywrite")
            .default_width(1000)
            .default_height(780)
            .build();
        window.add_css_class("writer");
        window.set_icon_name(Some("com.tobywrite.Writer"));
        let header = gtk::HeaderBar::new();
        let new = gtk::Button::from_icon_name("document-new-symbolic");
        new.set_tooltip_text(Some("New document (Ctrl+N)"));
        new.set_action_name(Some("win.new"));
        let open = gtk::Button::from_icon_name("document-open-symbolic");
        open.set_tooltip_text(Some("Quick Open (Ctrl+O)"));
        open.set_action_name(Some("win.open"));
        header.pack_start(&new);
        header.pack_start(&open);
        let title_box = gtk::Box::new(gtk::Orientation::Vertical, 1);
        let title = gtk::Label::new(None);
        title.add_css_class("document-title");
        let subtitle = gtk::Label::new(None);
        subtitle.add_css_class("document-path");
        subtitle.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        subtitle.set_max_width_chars(55);
        title_box.append(&title);
        title_box.append(&subtitle);
        header.set_title_widget(Some(&title_box));
        let menu = gtk::MenuButton::builder()
            .icon_name("open-menu-symbolic")
            .tooltip_text("Document menu")
            .build();
        header.pack_end(&menu);
        let save = gtk::Button::from_icon_name("document-save-symbolic");
        save.set_tooltip_text(Some("Save (Ctrl+S)"));
        save.set_action_name(Some("win.save"));
        header.pack_end(&save);
        let focus_button = gtk::ToggleButton::new();
        focus_button.set_icon_name("view-reveal-symbolic");
        focus_button.set_tooltip_text(Some("Focus current paragraph (Ctrl+Shift+D)"));
        header.pack_end(&focus_button);
        window.set_titlebar(Some(&header));
        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let buffer = gtk::TextBuffer::new(None);
        buffer.set_enable_undo(true);
        buffer.set_max_undo_levels(500);
        buffer.set_text(&document.file.text);
        buffer.set_modified(document.dirty);
        let view = gtk::TextView::builder()
            .buffer(&buffer)
            .wrap_mode(gtk::WrapMode::WordChar)
            .monospace(true)
            .top_margin(72)
            .bottom_margin(160)
            .left_margin(48)
            .right_margin(48)
            .pixels_above_lines(3)
            .pixels_below_lines(5)
            .accepts_tab(true)
            .hexpand(true)
            .vexpand(true)
            .build();
        let find = FindBar::new(&view);
        content.append(&find.widget);
        let scroll = gtk::ScrolledWindow::builder()
            .child(&view)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .build();
        content.append(&scroll);
        let status_box = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        status_box.add_css_class("status");
        let status = gtk::Label::new(None);
        status.set_xalign(0.);
        status.set_hexpand(true);
        status.set_ellipsize(gtk::pango::EllipsizeMode::End);
        status_box.append(&status);
        let format = gtk::Label::new(Some("Markdown"));
        format.set_tooltip_text(Some(
            "Plain Markdown · your original file encoding and line endings are preserved",
        ));
        status_box.append(&format);
        content.append(&status_box);
        window.set_child(Some(&content));
        let clean_text = if document.dirty {
            None
        } else {
            Some(document.file.text.clone())
        };
        let counts = Cell::new((
            editing::word_count(&document.file.text),
            document.file.text.chars().count(),
        ));
        let styler = Styler::new(&buffer);
        let this = Rc::new(Self {
            window,
            view,
            buffer,
            document: RefCell::new(document),
            find,
            writer: Rc::downgrade(writer),
            title,
            subtitle,
            status,
            focus_button,
            styler,
            clean_text: RefCell::new(clean_text),
            resetting: Cell::new(false),
            refresh_pending: Cell::new(false),
            syntax_dirty: Cell::new(false),
            counts,
            allow_close: Cell::new(false),
            close_pending: Cell::new(false),
            saving: Cell::new(false),
            preedit: Cell::new(false),
        });
        this.actions(writer);
        menu.set_menu_model(Some(&this.menu(writer)));
        this.connect();
        this.refresh_header();
        this.refresh_style();
        this.update_status();
        this.update_toggles();
        this.resize_margins();
        this
    }

    fn connect(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.buffer.connect_changed(move |_| {
            if let Some(editor) = weak.upgrade() {
                editor.changed();
            }
        });
        let weak = Rc::downgrade(self);
        self.buffer.connect_mark_set(move |_, _, mark| {
            if mark.name().as_deref() == Some("insert")
                && let Some(editor) = weak.upgrade()
            {
                editor.schedule_refresh();
            }
        });
        let weak = Rc::downgrade(self);
        let previous_width = Cell::new(0);
        self.view.add_tick_callback(move |view, _| {
            if previous_width.replace(view.width()) != view.width()
                && let Some(editor) = weak.upgrade()
            {
                editor.resize_margins();
            }
            glib::ControlFlow::Continue
        });
        let weak = Rc::downgrade(self);
        self.view.connect_preedit_changed(move |_, text| {
            if let Some(editor) = weak.upgrade() {
                editor.preedit.set(!text.is_empty());
            }
        });
        let weak = Rc::downgrade(self);
        self.window.connect_close_request(move |_| {
            let Some(editor) = weak.upgrade() else {
                return glib::Propagation::Proceed;
            };
            if editor.allow_close.get() || !editor.document.borrow().dirty {
                if let Some(writer) = editor.writer.upgrade() {
                    writer.remove_document(&editor);
                }
                return glib::Propagation::Proceed;
            }
            if editor.close_pending.replace(true) {
                return glib::Propagation::Stop;
            }
            let title = editor.document.borrow().title();
            let weak = Rc::downgrade(&editor);
            dialogs::choose(
                &editor.window,
                &format!("Save changes to “{title}”?"),
                "Save your changes before closing this document.",
                &["Cancel", "Discard Changes", "Save"],
                2,
                move |choice| {
                    let Some(editor) = weak.upgrade() else {
                        return;
                    };
                    match choice {
                        1 => {
                            editor.allow_close.set(true);
                            editor.window.close();
                        }
                        2 => editor.save(false, true),
                        _ => editor.close_pending.set(false),
                    }
                },
            );
            glib::Propagation::Stop
        });
        let weak = Rc::downgrade(self);
        self.focus_button.connect_toggled(move |button| {
            if let Some(editor) = weak.upgrade()
                && let Some(writer) = editor.writer.upgrade()
                && writer.preferences.borrow().focus_mode != button.is_active()
            {
                writer.update_preferences(|prefs| prefs.focus_mode = button.is_active());
            }
        });
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(self);
        keys.connect_key_pressed(move |_, key, _, modifiers| {
            let Some(editor) = weak.upgrade() else {
                return glib::Propagation::Proceed;
            };
            if (key == gdk::Key::Return || key == gdk::Key::KP_Enter)
                && editor.try_continue_list(modifiers)
            {
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        self.view.add_controller(keys);
        self.connect_paste();
    }

    fn actions(self: &Rc<Self>, writer: &Rc<Writer>) {
        self.action("new", |editor| {
            if let Some(writer) = editor.writer.upgrade() {
                writer.new_document("");
            }
        });
        self.action("open", |editor| {
            if let Some(writer) = editor.writer.upgrade() {
                library::show(&writer, editor);
            }
        });
        self.action("browse", |editor| editor.browse());
        self.action("save", |editor| editor.save(false, false));
        self.action("save-as", |editor| editor.save(true, false));
        self.action("close", |editor| editor.window.close());
        self.action("bold", |editor| {
            editor.apply_edit(editing::wrap(&editor.text(), editor.selection(), "**"))
        });
        self.action("italic", |editor| {
            editor.apply_edit(editing::wrap(&editor.text(), editor.selection(), "*"))
        });
        self.action("code", |editor| {
            editor.apply_edit(editing::wrap(&editor.text(), editor.selection(), "`"))
        });
        self.action("link", |editor| editor.insert_link());
        self.action("find", |editor| editor.find.show(false));
        self.action("replace", |editor| editor.find.show(true));
        self.action("next", |editor| editor.find.next(false));
        self.action("previous", |editor| editor.find.next(true));
        self.action("focus", |editor| {
            if let Some(writer) = editor.writer.upgrade() {
                writer.update_preferences(|prefs| prefs.focus_mode = !prefs.focus_mode);
            }
        });
        self.action("syntax", |editor| {
            if let Some(writer) = editor.writer.upgrade() {
                writer.update_preferences(|prefs| prefs.show_syntax = !prefs.show_syntax);
            }
        });
        self.action("settings", |editor| {
            if let Some(writer) = editor.writer.upgrade() {
                dialogs::settings(&writer, &editor.window);
            }
        });
        self.action("link-folder", |editor| {
            if let Some(writer) = editor.writer.upgrade() {
                writer.link_folder(&editor.window);
            }
        });
        self.action("shortcuts", |editor| dialogs::shortcuts(&editor.window));
        self.action("welcome", |editor| {
            if let Some(writer) = editor.writer.upgrade() {
                writer.new_document(include_str!("../../assets/Welcome.md"));
            }
        });
        self.action("larger", |editor| editor.font_size(1));
        self.action("smaller", |editor| editor.font_size(-1));
        self.action("actual-size", |editor| editor.font_size(0));
        self.action("print", |editor| {
            let title = editor.document.borrow().title();
            let text = editor.text();
            if let Err(error) = crate::printing::print(&editor.window, &text, &title) {
                dialogs::error(
                    Some(&editor.window),
                    "Could not print document",
                    &format!("{error:#}"),
                );
            }
        });
        self.action("export-html", |editor| editor.export(false));
        self.action("export-pdf", |editor| editor.export(true));
        let open_recent = gio::SimpleAction::new("open-recent", Some(glib::VariantTy::STRING));
        let weak = Rc::downgrade(writer);
        open_recent.connect_activate(move |_, parameter| {
            if let (Some(writer), Some(path)) = (weak.upgrade(), parameter.and_then(|p| p.str())) {
                writer.open(Path::new(path));
            }
        });
        self.window.add_action(&open_recent);
    }

    fn action(self: &Rc<Self>, name: &str, callback: impl Fn(&Rc<Self>) + 'static) {
        let action = gio::SimpleAction::new(name, None);
        let weak = Rc::downgrade(self);
        action.connect_activate(move |_, _| {
            if let Some(editor) = weak.upgrade() {
                callback(&editor);
            }
        });
        self.window.add_action(&action);
    }

    fn menu(&self, writer: &Writer) -> gio::Menu {
        let menu = gio::Menu::new();
        let file = gio::Menu::new();
        for (label, action) in [
            ("New Document", "win.new"),
            ("Quick Open…", "win.open"),
            ("Browse for a File…", "win.browse"),
            ("Link Folder…", "win.link-folder"),
        ] {
            file.append(Some(label), Some(action));
        }
        let recent = gio::Menu::new();
        for path in writer.recent_paths() {
            let title = path
                .file_name()
                .unwrap_or(path.as_os_str())
                .to_string_lossy();
            let item = gio::MenuItem::new(Some(&title), None);
            item.set_action_and_target_value(
                Some("win.open-recent"),
                Some(&path.to_string_lossy().to_string().to_variant()),
            );
            recent.append_item(&item);
        }
        file.append_submenu(Some("Open Recent"), &recent);
        menu.append_section(None, &file);
        let save = gio::Menu::new();
        for (label, action) in [
            ("Save", "win.save"),
            ("Save As…", "win.save-as"),
            ("Export HTML…", "win.export-html"),
            ("Export PDF…", "win.export-pdf"),
            ("Print…", "win.print"),
        ] {
            save.append(Some(label), Some(action));
        }
        menu.append_section(None, &save);
        let format = gio::Menu::new();
        for (label, action) in [
            ("Bold", "win.bold"),
            ("Italic", "win.italic"),
            ("Insert Link", "win.link"),
            ("Inline Code", "win.code"),
        ] {
            format.append(Some(label), Some(action));
        }
        menu.append_submenu(Some("Format"), &format);
        let view = gio::Menu::new();
        for (label, action) in [
            ("Focus Current Paragraph", "win.focus"),
            ("Show Markdown Syntax", "win.syntax"),
            ("Larger Text", "win.larger"),
            ("Smaller Text", "win.smaller"),
            ("Actual Text Size", "win.actual-size"),
        ] {
            view.append(Some(label), Some(action));
        }
        menu.append_submenu(Some("View"), &view);
        let last = gio::Menu::new();
        for (label, action) in [
            ("Find…", "win.find"),
            ("Find and Replace…", "win.replace"),
            ("Settings…", "win.settings"),
            ("Welcome to Tobywrite", "win.welcome"),
            ("Keyboard Shortcuts", "win.shortcuts"),
            ("Close Document", "win.close"),
        ] {
            last.append(Some(label), Some(action));
        }
        menu.append_section(None, &last);
        menu
    }

    pub fn text(&self) -> String {
        self.buffer
            .text(&self.buffer.start_iter(), &self.buffer.end_iter(), true)
            .to_string()
    }

    pub fn selection(&self) -> Range<usize> {
        if let Some((start, end)) = self.buffer.selection_bounds() {
            start.offset() as usize..end.offset() as usize
        } else {
            let at = self.buffer.cursor_position() as usize;
            at..at
        }
    }

    pub fn apply_edit(&self, edit: editing::Edit) {
        super::begin_edit(&self.buffer);
        let mut start = self.buffer.iter_at_offset(edit.range.start as i32);
        let mut end = self.buffer.iter_at_offset(edit.range.end as i32);
        self.buffer.delete(&mut start, &mut end);
        self.buffer.insert(&mut start, &edit.replacement);
        self.buffer.select_range(
            &self.buffer.iter_at_offset(edit.selection.start as i32),
            &self.buffer.iter_at_offset(edit.selection.end as i32),
        );
        self.buffer.end_user_action();
        self.view.grab_focus();
        self.ensure_caret();
    }

    pub(super) fn try_continue_list(&self, modifiers: gdk::ModifierType) -> bool {
        if modifiers.intersects(
            gdk::ModifierType::SHIFT_MASK
                | gdk::ModifierType::CONTROL_MASK
                | gdk::ModifierType::ALT_MASK
                | gdk::ModifierType::SUPER_MASK,
        ) || self.preedit.get()
        {
            return false;
        }
        if let Some(edit) = editing::continue_list(&self.text(), self.selection()) {
            self.apply_edit(edit);
            true
        } else {
            false
        }
    }

    fn changed(self: &Rc<Self>) {
        if self.resetting.get() {
            return;
        }
        let text = self.text();
        let dirty = self
            .clean_text
            .borrow()
            .as_ref()
            .is_none_or(|clean| *clean != text);
        {
            let mut document = self.document.borrow_mut();
            document.set_text(text);
            document.dirty = dirty;
        }
        self.buffer.set_modified(dirty);
        self.refresh_header();
        self.schedule_refresh();
        self.syntax_dirty.set(true);
        if let Some(writer) = self.writer.upgrade() {
            writer.schedule_snapshot();
        }
    }

    fn schedule_refresh(self: &Rc<Self>) {
        if self.refresh_pending.replace(true) {
            return;
        }
        let weak = Rc::downgrade(self);
        glib::timeout_add_local_once(Duration::from_millis(120), move || {
            if let Some(editor) = weak.upgrade() {
                editor.refresh_pending.set(false);
                if editor.syntax_dirty.replace(false) {
                    editor.refresh_style();
                    editor.refresh_counts();
                    if editor.find.widget.is_visible() {
                        editor.find.count();
                    }
                } else if let Some(writer) = editor.writer.upgrade() {
                    let prefs = writer.preferences.borrow();
                    editor
                        .styler
                        .caret(&editor.buffer, prefs.show_syntax, prefs.focus_mode);
                }
                editor.update_status();
            }
        });
    }

    pub fn refresh_style(&self) {
        let Some(writer) = self.writer.upgrade() else {
            return;
        };
        let prefs = writer.preferences.borrow();
        self.styler.parse(
            &self.buffer,
            &self.document.borrow().file.text,
            writer.dark(),
            prefs.show_syntax,
            prefs.focus_mode,
        );
        let mut tabs = gtk::pango::TabArray::new(1, true);
        tabs.set_tab(0, gtk::pango::TabAlign::Left, prefs.font_size * 4 * 3 / 5);
        self.view.set_tabs(&tabs);
    }

    fn resize_margins(&self) {
        let width = self.view.width();
        let font_size = self
            .writer
            .upgrade()
            .map(|writer| writer.preferences.borrow().font_size)
            .unwrap_or(20);
        let writing_width = font_size * 38;
        let margin = ((width - writing_width) / 2).max(28);
        self.view.set_left_margin(margin);
        self.view.set_right_margin(margin);
    }

    fn ensure_caret(&self) {
        let mut cursor = self.buffer.iter_at_offset(self.buffer.cursor_position());
        self.view.scroll_to_iter(&mut cursor, 0.12, false, 0., 0.);
    }

    fn refresh_header(&self) {
        let document = self.document.borrow();
        let title = document.title();
        let marker = if document.dirty { " •" } else { "" };
        self.window
            .set_title(Some(&format!("{title}{marker} — Tobywrite")));
        self.title.set_text(&format!("{title}{marker}"));
        let path = document
            .path
            .as_ref()
            .and_then(|path| path.parent())
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_else(|| "New document".to_owned());
        self.subtitle.set_text(&path);
        self.subtitle.set_tooltip_text(Some(&path));
    }

    fn update_status(&self) {
        let document = self.document.borrow();
        let (words, chars) = self.counts.get();
        let cursor = self.buffer.iter_at_offset(self.buffer.cursor_position());
        let state = if document.dirty {
            " · Unsaved changes"
        } else {
            ""
        };
        let selection = self
            .buffer
            .selection_bounds()
            .map(|(start, end)| {
                let text = self.buffer.text(&start, &end, true);
                format!(
                    " · {} words / {} characters selected",
                    editing::word_count(&text),
                    text.chars().count()
                )
            })
            .unwrap_or_default();
        self.status.set_text(&format!("{words} words · {chars} characters{selection} · {} min read     Ln {}, Col {} · {} · {}{state}", words.div_ceil(200).max(1), cursor.line() + 1, cursor.line_offset() + 1, document.file.encoding, document.file.line_ending));
    }

    fn refresh_counts(&self) {
        let document = self.document.borrow();
        self.counts.set((
            editing::word_count(&document.file.text),
            document.file.text.chars().count(),
        ));
    }

    pub fn update_toggles(&self) {
        if let Some(writer) = self.writer.upgrade() {
            self.focus_button
                .set_active(writer.preferences.borrow().focus_mode);
        }
        self.resize_margins();
    }

    fn font_size(&self, delta: i32) {
        if let Some(writer) = self.writer.upgrade() {
            writer.update_preferences(|prefs| {
                prefs.font_size = if delta == 0 {
                    20
                } else {
                    (prefs.font_size + delta).clamp(12, 36)
                }
            });
        }
    }

    pub fn browse(self: &Rc<Self>) {
        let dialog = gtk::FileDialog::builder()
            .title("Open a Markdown or text document")
            .build();
        let weak = self.writer.clone();
        let parent = self.window.clone();
        dialog.open_multiple(
            Some(&self.window),
            None::<&gio::Cancellable>,
            move |result| match result {
                Ok(files) => {
                    if let Some(writer) = weak.upgrade() {
                        for index in 0..files.n_items() {
                            if let Some(path) = files
                                .item(index)
                                .and_downcast::<gio::File>()
                                .and_then(|file| file.path())
                            {
                                writer.open(&path);
                            }
                        }
                    }
                }
                Err(error)
                    if error.matches(gtk::DialogError::Dismissed)
                        || error.matches(gtk::DialogError::Cancelled) => {}
                Err(error) => dialogs::error(
                    Some(&parent),
                    "Could not open the file picker",
                    &error.to_string(),
                ),
            },
        );
    }

    pub fn save(self: &Rc<Self>, save_as: bool, close_after: bool) {
        if self.saving.replace(true) {
            return;
        }
        let path = self.document.borrow().path.clone();
        if !save_as && let Some(path) = path {
            self.save_target(path, false, close_after);
            return;
        }
        let dialog = gtk::FileDialog::builder()
            .title("Save Markdown document")
            .initial_name(self.document.borrow().title())
            .build();
        let folder = self
            .document
            .borrow()
            .path
            .as_ref()
            .and_then(|path| path.parent().map(Path::to_path_buf))
            .or_else(|| {
                self.writer
                    .upgrade()
                    .and_then(|writer| writer.preferences.borrow().linked_folders.first().cloned())
            });
        if let Some(folder) = folder {
            dialog.set_initial_folder(Some(&gio::File::for_path(folder)));
        }
        if self.document.borrow().path.is_none() {
            dialog.set_initial_name(Some("Untitled.md"));
        }
        let weak = Rc::downgrade(self);
        dialog.save(
            Some(&self.window),
            None::<&gio::Cancellable>,
            move |result| {
                if let Some(editor) = weak.upgrade() {
                    match result {
                        Ok(file) => {
                            if let Some(path) = file.path() {
                                editor.save_target(path, false, close_after);
                            } else {
                                editor.save_failed("Choose a local file to save this document.");
                            }
                        }
                        Err(error)
                            if error.matches(gtk::DialogError::Dismissed)
                                || error.matches(gtk::DialogError::Cancelled) =>
                        {
                            editor.saving.set(false);
                            editor.close_pending.set(false);
                        }
                        Err(error) => editor.save_failed(&error.to_string()),
                    }
                }
            },
        );
    }

    pub(super) fn save_target(self: &Rc<Self>, path: PathBuf, overwrite: bool, close_after: bool) {
        if self.other_document_at(&path) {
            self.save_failed("This file is already open in another Tobywrite window. Save to a different filename or return to that document.");
            return;
        }
        let same = self.document.borrow().path.as_ref() == Some(&path);
        if !overwrite {
            if same {
                let changed = self.document.borrow().disk_changed();
                match changed {
                    Ok(true) => {
                        self.conflict(path, close_after);
                        return;
                    }
                    Err(error) => {
                        self.save_failed(&format!("{error:#}"));
                        return;
                    }
                    _ => {}
                }
            } else if path.exists() {
                let weak = Rc::downgrade(self);
                dialogs::choose(
                    &self.window,
                    "Replace this file?",
                    &format!(
                        "{} already exists. Its contents will be replaced with this document.",
                        path.display()
                    ),
                    &["Cancel", "Replace"],
                    0,
                    move |choice| {
                        if let Some(editor) = weak.upgrade() {
                            if choice == 1 {
                                editor.save_target(path, true, close_after);
                            } else {
                                editor.saving.set(false);
                                editor.close_pending.set(false);
                            }
                        }
                    },
                );
                return;
            }
        }
        let result = self.document.borrow_mut().save(&path, overwrite);
        match result {
            Ok(()) => {
                *self.clean_text.borrow_mut() = Some(self.text());
                self.buffer.set_modified(false);
                self.saving.set(false);
                self.close_pending.set(false);
                self.refresh_header();
                self.refresh_counts();
                self.update_status();
                if let Some(writer) = self.writer.upgrade() {
                    remember_recent(&mut writer.preferences.borrow_mut(), &path);
                    writer.save_preferences();
                    writer.snapshot();
                }
                if close_after {
                    self.allow_close.set(true);
                    self.window.close();
                }
            }
            Err(error) => self.save_failed(&format!("{error:#}")),
        }
    }

    fn conflict(self: &Rc<Self>, path: PathBuf, close_after: bool) {
        let weak = Rc::downgrade(self);
        dialogs::choose(
            &self.window,
            "This document changed on disk",
            "Another program changed or removed this file after Tobywrite opened it. Keep your draft with Save As, reload the disk version, or replace the disk version with your changes.",
            &["Cancel", "Reload from Disk", "Save As…", "Save Anyway"],
            2,
            move |choice| {
                let Some(editor) = weak.upgrade() else {
                    return;
                };
                match choice {
                    1 => match Document::open(&path) {
                        Ok(document) => {
                            editor.replace_document(document);
                            editor.saving.set(false);
                            editor.close_pending.set(false);
                            if close_after {
                                editor.allow_close.set(true);
                                editor.window.close();
                            }
                        }
                        Err(error) => editor.save_failed(&format!("{error:#}")),
                    },
                    2 => {
                        editor.saving.set(false);
                        editor.save(true, close_after);
                    }
                    3 => editor.save_target(path, true, close_after),
                    _ => {
                        editor.saving.set(false);
                        editor.close_pending.set(false);
                    }
                }
            },
        );
    }

    fn save_failed(&self, message: &str) {
        self.saving.set(false);
        self.close_pending.set(false);
        dialogs::error(Some(&self.window), "Could not save document", message);
        if let Some(writer) = self.writer.upgrade() {
            writer.snapshot();
        }
    }

    fn other_document_at(&self, path: &Path) -> bool {
        let Some(writer) = self.writer.upgrade() else {
            return false;
        };
        let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        writer.documents.borrow().iter().any(|editor| {
            editor.window != self.window && editor.document.borrow().path.as_ref() == Some(&path)
        })
    }

    pub fn replace_document(&self, document: Document) {
        self.resetting.set(true);
        self.buffer.set_enable_undo(false);
        self.buffer.set_text(&document.file.text);
        self.buffer.set_enable_undo(true);
        self.buffer.set_modified(document.dirty);
        *self.clean_text.borrow_mut() = if document.dirty {
            None
        } else {
            Some(document.file.text.clone())
        };
        *self.document.borrow_mut() = document;
        self.resetting.set(false);
        self.refresh_header();
        self.refresh_style();
        self.refresh_counts();
        self.update_status();
        if let Some(writer) = self.writer.upgrade() {
            writer.snapshot();
        }
    }

    fn connect_paste(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.view.connect_paste_clipboard(move |view| {
            let Some(editor) = weak.upgrade() else {
                return;
            };
            let (start, end) = editor.buffer.selection_bounds().unwrap_or_else(|| {
                let cursor = editor
                    .buffer
                    .iter_at_offset(editor.buffer.cursor_position());
                (cursor, cursor)
            });
            let had_selection = start != end;
            let selection = editor.buffer.text(&start, &end, true).to_string();
            let start = editor.buffer.create_mark(None, &start, true);
            let end = editor.buffer.create_mark(None, &end, false);
            view.stop_signal_emission_by_name("paste-clipboard");
            let weak = Rc::downgrade(&editor);
            view.clipboard()
                .read_text_async(None::<&gio::Cancellable>, move |result| {
                    let Some(editor) = weak.upgrade() else {
                        return;
                    };
                    let first = editor.buffer.iter_at_mark(&start);
                    let last = editor.buffer.iter_at_mark(&end);
                    editor.buffer.delete_mark(&start);
                    editor.buffer.delete_mark(&end);
                    if let Ok(Some(text)) = result {
                        // A delayed clipboard response must never replace newly edited text.
                        if editor.buffer.text(&first, &last, true) != selection {
                            return;
                        }
                        let range = first.offset() as usize..last.offset() as usize;
                        if editor.selection() != range {
                            return;
                        }
                        let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
                        let url = if had_selection {
                            editing::accepted_url(&normalized)
                        } else {
                            None
                        };
                        let edit = if let Some(url) = url {
                            editing::insert_link(&editor.text(), range, Some(url))
                        } else {
                            let at = range.start + normalized.chars().count();
                            editing::Edit {
                                range,
                                replacement: normalized,
                                selection: at..at,
                            }
                        };
                        editor.apply_edit(edit);
                    }
                });
        });
    }

    fn insert_link(self: &Rc<Self>) {
        let original = self.text();
        let selection = self.selection();
        let weak = Rc::downgrade(self);
        self.view
            .clipboard()
            .read_text_async(None::<&gio::Cancellable>, move |result| {
                if let Some(editor) = weak.upgrade() {
                    if editor.text() != original || editor.selection() != selection {
                        return;
                    }
                    let clipboard = result.ok().flatten();
                    let url = clipboard.as_deref().and_then(editing::accepted_url);
                    editor.apply_edit(editing::insert_link(&original, selection, url));
                }
            });
    }

    fn export(self: &Rc<Self>, pdf: bool) {
        let title = self.document.borrow().title();
        let stem = Path::new(&title)
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy();
        let name = format!("{stem}.{}", if pdf { "pdf" } else { "html" });
        let dialog = gtk::FileDialog::builder()
            .title(if pdf { "Export PDF" } else { "Export HTML" })
            .initial_name(name)
            .build();
        let weak = Rc::downgrade(self);
        dialog.save(
            Some(&self.window),
            None::<&gio::Cancellable>,
            move |result| {
                if let (Some(editor), Ok(file)) = (weak.upgrade(), result)
                    && let Some(path) = file.path()
                {
                    editor.export_to(path, pdf, false);
                }
            },
        );
    }

    fn export_to(self: &Rc<Self>, path: PathBuf, pdf: bool, overwrite: bool) {
        let canonical = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
        if self.document.borrow().path.as_ref() == Some(&canonical)
            || self.other_document_at(&canonical)
        {
            dialogs::error(
                Some(&self.window),
                "Choose a different export filename",
                "An export cannot replace a document that is open in Tobywrite.",
            );
            return;
        }
        if path.exists() && !overwrite {
            let weak = Rc::downgrade(self);
            dialogs::choose(
                &self.window,
                "Replace this export?",
                &format!("{} already exists.", path.display()),
                &["Cancel", "Replace"],
                0,
                move |choice| {
                    if choice == 1
                        && let Some(editor) = weak.upgrade()
                    {
                        editor.export_to(path, pdf, true);
                    }
                },
            );
            return;
        }
        let title = self.document.borrow().title();
        let result = if pdf {
            crate::printing::export_pdf_to(&self.text(), &title, &path, overwrite)
        } else {
            crate::export::save_html(&self.text(), &title, &path, overwrite)
        };
        if let Err(error) = result {
            dialogs::error(
                Some(&self.window),
                "Could not export document",
                &format!("{error:#}"),
            );
        }
    }
}
