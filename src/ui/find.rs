use gtk::{glib, prelude::*};
use std::rc::Rc;

pub struct FindBar {
    pub widget: gtk::Box,
    pub search: gtk::SearchEntry,
    pub(super) replacement: gtk::Entry,
    replace_row: gtk::Box,
    case_sensitive: gtk::CheckButton,
    result: gtk::Label,
    buffer: gtk::TextBuffer,
    view: gtk::TextView,
}

impl FindBar {
    pub fn new(view: &gtk::TextView) -> Rc<Self> {
        let widget = gtk::Box::new(gtk::Orientation::Vertical, 6);
        widget.add_css_class("find-bar");
        widget.set_visible(false);
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let search = gtk::SearchEntry::new();
        search.set_placeholder_text(Some("Find in document"));
        search.set_hexpand(true);
        let previous = gtk::Button::from_icon_name("go-up-symbolic");
        previous.set_tooltip_text(Some("Previous match (Ctrl+Shift+G)"));
        let next = gtk::Button::from_icon_name("go-down-symbolic");
        next.set_tooltip_text(Some("Next match (Ctrl+G)"));
        let close = gtk::Button::from_icon_name("window-close-symbolic");
        close.set_tooltip_text(Some("Close find (Escape)"));
        let result = gtk::Label::new(None);
        let case_sensitive = gtk::CheckButton::with_label("Match case");
        for child in [
            search.upcast_ref::<gtk::Widget>(),
            previous.upcast_ref(),
            next.upcast_ref(),
            result.upcast_ref(),
            case_sensitive.upcast_ref(),
            close.upcast_ref(),
        ] {
            row.append(child);
        }
        widget.append(&row);
        let replace_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let replacement = gtk::Entry::new();
        replacement.set_placeholder_text(Some("Replace with"));
        replacement.set_hexpand(true);
        let replace = gtk::Button::with_label("Replace");
        let replace_all = gtk::Button::with_label("Replace All");
        replace_row.append(&replacement);
        replace_row.append(&replace);
        replace_row.append(&replace_all);
        widget.append(&replace_row);
        let this = Rc::new(Self {
            widget,
            search,
            replacement,
            replace_row,
            result,
            case_sensitive,
            buffer: view.buffer(),
            view: view.clone(),
        });
        let weak = Rc::downgrade(&this);
        next.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade() {
                this.next(false);
            }
        });
        let weak = Rc::downgrade(&this);
        previous.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade() {
                this.next(true);
            }
        });
        let weak = Rc::downgrade(&this);
        this.search.connect_activate(move |_| {
            if let Some(this) = weak.upgrade() {
                this.next(false);
            }
        });
        let weak = Rc::downgrade(&this);
        this.search.connect_search_changed(move |_| {
            if let Some(this) = weak.upgrade() {
                this.count();
            }
        });
        let weak = Rc::downgrade(&this);
        this.case_sensitive.connect_toggled(move |_| {
            if let Some(this) = weak.upgrade() {
                this.count();
            }
        });
        let weak = Rc::downgrade(&this);
        close.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade() {
                this.hide();
            }
        });
        let weak = Rc::downgrade(&this);
        replace.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade() {
                this.replace_one();
            }
        });
        let weak = Rc::downgrade(&this);
        replace_all.connect_clicked(move |_| {
            if let Some(this) = weak.upgrade() {
                this.replace_all();
            }
        });
        let keys = gtk::EventControllerKey::new();
        let weak = Rc::downgrade(&this);
        keys.connect_key_pressed(move |_, key, _, _| {
            if key == gtk::gdk::Key::Escape {
                if let Some(this) = weak.upgrade() {
                    this.hide();
                }
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        this.widget.add_controller(keys);
        this
    }

    pub fn show(&self, replace: bool) {
        if let Some((start, end)) = self.buffer.selection_bounds() {
            let text = self.buffer.text(&start, &end, true);
            if !text.contains('\n') {
                self.search.set_text(&text);
            }
        }
        self.replace_row.set_visible(replace);
        self.widget.set_visible(true);
        self.search.grab_focus();
        self.count();
    }

    fn hide(&self) {
        self.widget.set_visible(false);
        self.view.grab_focus();
    }

    fn flags(&self) -> gtk::TextSearchFlags {
        let mut flags = gtk::TextSearchFlags::TEXT_ONLY;
        if !self.case_sensitive.is_active() {
            flags |= gtk::TextSearchFlags::CASE_INSENSITIVE;
        }
        flags
    }

    pub fn next(&self, backwards: bool) {
        let query = self.search.text();
        if query.is_empty() {
            self.show(false);
            return;
        }
        let (start, end) = self.buffer.selection_bounds().unwrap_or_else(|| {
            let at = self.buffer.iter_at_offset(self.buffer.cursor_position());
            (at, at)
        });
        let found = if backwards {
            start
                .backward_search(&query, self.flags(), None)
                .or_else(|| {
                    self.buffer
                        .end_iter()
                        .backward_search(&query, self.flags(), None)
                })
        } else {
            end.forward_search(&query, self.flags(), None).or_else(|| {
                self.buffer
                    .start_iter()
                    .forward_search(&query, self.flags(), None)
            })
        };
        if let Some((mut start, end)) = found {
            self.buffer.select_range(&start, &end);
            self.view.scroll_to_iter(&mut start, 0.15, false, 0., 0.);
            self.result.set_text("");
        } else {
            self.result.set_text("No matches");
        }
    }

    fn matches(&self) -> Vec<(i32, i32)> {
        let query = self.search.text();
        if query.is_empty() {
            return Vec::new();
        }
        let mut at = self.buffer.start_iter();
        let mut matches = Vec::new();
        while let Some((start, end)) = at.forward_search(&query, self.flags(), None) {
            if start == end {
                break;
            }
            matches.push((start.offset(), end.offset()));
            at = end;
            if matches.len() >= 100_000 {
                break;
            }
        }
        matches
    }

    pub fn count(&self) {
        if self.search.text().is_empty() {
            self.result.set_text("");
            return;
        }
        let count = self.matches().len();
        self.result.set_text(&format!(
            "{count} {}",
            if count == 1 { "match" } else { "matches" }
        ));
    }

    fn replace_one(&self) {
        if let Some((mut start, mut end)) = self.buffer.selection_bounds() {
            let query = self.search.text();
            let matches = !query.is_empty()
                && start
                    .forward_search(&query, self.flags(), Some(&end))
                    .is_some_and(|(first, last)| first == start && last == end);
            if matches {
                super::begin_edit(&self.buffer);
                self.buffer.delete(&mut start, &mut end);
                self.buffer.insert(&mut start, &self.replacement.text());
                self.buffer.end_user_action();
            }
        }
        self.next(false);
        self.count();
    }

    pub fn replace_all(&self) {
        let matches = self.matches();
        if matches.is_empty() {
            return;
        }
        super::begin_edit(&self.buffer);
        for (start, end) in matches.iter().rev() {
            let mut start = self.buffer.iter_at_offset(*start);
            let mut end = self.buffer.iter_at_offset(*end);
            self.buffer.delete(&mut start, &mut end);
            self.buffer.insert(&mut start, &self.replacement.text());
        }
        self.buffer.end_user_action();
        self.count();
    }
}
