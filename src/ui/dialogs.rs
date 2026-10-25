use super::Writer;
use crate::storage::Appearance;
use gtk::{gio, prelude::*};
use std::rc::Rc;

pub fn error(parent: Option<&impl IsA<gtk::Window>>, message: &str, detail: &str) {
    gtk::AlertDialog::builder()
        .message(message)
        .detail(detail)
        .modal(true)
        .build()
        .show(parent);
}

pub fn choose(
    parent: &impl IsA<gtk::Window>,
    message: &str,
    detail: &str,
    buttons: &[&str],
    default: i32,
    callback: impl FnOnce(i32) + 'static,
) {
    let dialog = gtk::AlertDialog::builder()
        .message(message)
        .detail(detail)
        .modal(true)
        .cancel_button(0)
        .default_button(default)
        .build();
    dialog.set_buttons(buttons);
    dialog.choose(Some(parent), None::<&gio::Cancellable>, move |result| {
        callback(result.unwrap_or(0))
    });
}

pub fn settings(state: &Rc<Writer>, parent: &gtk::ApplicationWindow) {
    let window = gtk::Window::builder()
        .title("Tobywrite Settings")
        .transient_for(parent)
        .modal(true)
        .default_width(500)
        .resizable(false)
        .build();
    let content = gtk::Box::new(gtk::Orientation::Vertical, 18);
    for set in [
        gtk::Widget::set_margin_top,
        gtk::Widget::set_margin_bottom,
        gtk::Widget::set_margin_start,
        gtk::Widget::set_margin_end,
    ] {
        set(content.upcast_ref(), 24);
    }
    let heading = gtk::Label::new(Some("Writing"));
    heading.add_css_class("title-3");
    heading.set_xalign(0.);
    content.append(&heading);
    let prefs = state.preferences.borrow().clone();
    let appearance = gtk::DropDown::from_strings(&["System", "Light", "Dark"]);
    appearance.set_selected(match prefs.appearance {
        Appearance::System => 0,
        Appearance::Light => 1,
        Appearance::Dark => 2,
    });
    row(&content, "Appearance", &appearance);
    let size = gtk::SpinButton::with_range(12., 36., 1.);
    size.set_value(prefs.font_size as f64);
    row(&content, "Text size", &size);
    let focus = gtk::Switch::builder()
        .active(prefs.focus_mode)
        .valign(gtk::Align::Center)
        .build();
    row(&content, "Focus current paragraph", &focus);
    let syntax = gtk::Switch::builder()
        .active(prefs.show_syntax)
        .valign(gtk::Align::Center)
        .build();
    row(&content, "Show Markdown syntax", &syntax);
    let separator = gtk::Separator::new(gtk::Orientation::Horizontal);
    content.append(&separator);
    let heading = gtk::Label::new(Some("Writing folders"));
    heading.add_css_class("title-3");
    heading.set_xalign(0.);
    content.append(&heading);
    let detail = gtk::Label::new(Some(
        "Quick Open searches Markdown and text files in these folders. Linking a folder keeps its files in place.",
    ));
    detail.set_wrap(true);
    detail.set_xalign(0.);
    detail.add_css_class("dim-label");
    content.append(&detail);
    let list = gtk::Box::new(gtk::Orientation::Vertical, 6);
    if prefs.linked_folders.is_empty() {
        let label = gtk::Label::new(Some("No linked folders"));
        label.set_xalign(0.);
        list.append(&label);
    }
    for path in prefs.linked_folders {
        let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let label = gtk::Label::new(Some(&path.to_string_lossy()));
        label.set_xalign(0.);
        label.set_hexpand(true);
        label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        label.set_tooltip_text(Some(&path.to_string_lossy()));
        let remove = gtk::Button::from_icon_name("list-remove-symbolic");
        remove.set_tooltip_text(Some("Unlink folder"));
        line.append(&label);
        line.append(&remove);
        list.append(&line);
        let weak = Rc::downgrade(state);
        let list = list.clone();
        remove.connect_clicked(move |_| {
            if let Some(state) = weak.upgrade() {
                state.update_preferences(|prefs| prefs.linked_folders.retain(|item| item != &path));
                list.remove(&line);
            }
        });
    }
    content.append(&list);
    let link = gtk::Button::with_label("Link Folder…");
    link.set_halign(gtk::Align::Start);
    content.append(&link);
    let done = gtk::Button::with_label("Done");
    done.add_css_class("suggested-action");
    done.set_halign(gtk::Align::End);
    content.append(&done);
    window.set_child(Some(&content));
    let weak = Rc::downgrade(state);
    appearance.connect_selected_notify(move |widget| {
        if let Some(state) = weak.upgrade() {
            state.update_preferences(|prefs| {
                prefs.appearance = match widget.selected() {
                    1 => Appearance::Light,
                    2 => Appearance::Dark,
                    _ => Appearance::System,
                }
            });
        }
    });
    let weak = Rc::downgrade(state);
    size.connect_value_changed(move |widget| {
        if let Some(state) = weak.upgrade() {
            state.update_preferences(|prefs| prefs.font_size = widget.value_as_int());
        }
    });
    let weak = Rc::downgrade(state);
    focus.connect_active_notify(move |widget| {
        if let Some(state) = weak.upgrade() {
            state.update_preferences(|prefs| prefs.focus_mode = widget.is_active());
        }
    });
    let weak = Rc::downgrade(state);
    syntax.connect_active_notify(move |widget| {
        if let Some(state) = weak.upgrade() {
            state.update_preferences(|prefs| prefs.show_syntax = widget.is_active());
        }
    });
    let weak = Rc::downgrade(state);
    let parent = parent.clone();
    let settings_window = window.clone();
    link.connect_clicked(move |_| {
        if let Some(state) = weak.upgrade() {
            state.link_folder(&parent);
            settings_window.close();
        }
    });
    let settings_window = window.clone();
    done.connect_clicked(move |_| settings_window.close());
    window.present();
}

fn row(content: &gtk::Box, title: &str, control: &impl IsA<gtk::Widget>) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 20);
    let label = gtk::Label::new(Some(title));
    label.set_xalign(0.);
    label.set_hexpand(true);
    row.append(&label);
    row.append(control);
    content.append(&row);
}

pub fn shortcuts(parent: &gtk::ApplicationWindow) {
    let window = gtk::Window::builder()
        .title("Keyboard Shortcuts")
        .transient_for(parent)
        .default_width(520)
        .default_height(560)
        .build();
    let content = gtk::Box::new(gtk::Orientation::Vertical, 14);
    content.set_margin_top(24);
    content.set_margin_bottom(24);
    content.set_margin_start(28);
    content.set_margin_end(28);
    let heading = gtk::Label::new(Some("Write without leaving the keyboard"));
    heading.add_css_class("title-3");
    heading.set_xalign(0.);
    content.append(&heading);
    for (title, shortcut) in [
        ("New document / Quick Open", "Ctrl+N / Ctrl+O"),
        ("Browse for a file", "Ctrl+Shift+O"),
        ("Save / Save As", "Ctrl+S / Ctrl+Shift+S"),
        ("Close window", "Ctrl+W"),
        ("Find / Replace", "Ctrl+F / Ctrl+H"),
        ("Next / previous match", "Ctrl+G / Ctrl+Shift+G"),
        ("Bold / Italic", "Ctrl+B / Ctrl+I"),
        ("Link / Inline code", "Ctrl+K / Ctrl+Shift+K"),
        ("Focus current paragraph", "Ctrl+Shift+D"),
        ("Show Markdown syntax", "Ctrl+Shift+M"),
        ("Larger / smaller / reset text", "Ctrl++ / Ctrl+− / Ctrl+0"),
        ("Print / Settings", "Ctrl+P / Ctrl+,"),
        ("Link writing folder", "Ctrl+D"),
        ("Undo / Redo", "Ctrl+Z / Ctrl+Shift+Z"),
        ("Plain newline in a list", "Shift+Enter"),
    ] {
        let value = gtk::Label::new(Some(shortcut));
        value.add_css_class("dim-label");
        row(&content, title, &value);
    }
    let note = gtk::Label::new(Some(
        "Paste a web address over selected text to turn it into a Markdown link. Press Return in a list to continue it, or Return on an empty item to leave the list. Your files remain plain Markdown.",
    ));
    note.set_wrap(true);
    note.set_xalign(0.);
    note.add_css_class("dim-label");
    content.append(&note);
    let scroll = gtk::ScrolledWindow::builder()
        .child(&content)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .build();
    window.set_child(Some(&scroll));
    window.present();
}
