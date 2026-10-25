//! Theme integration checks use fixture files under the smoke data directory.
use super::{Writer, appearance, dialogs, editor::Editor, library, palette::Palette};
use crate::storage::Appearance;
use anyhow::{Context, Result, ensure};
use gtk::{gdk, glib, prelude::*};
use std::{cell::RefCell, path::PathBuf, rc::Rc, time::Duration};

const DARK: &str = include_str!("../../tests/fixtures/themes/retro-82.toml");
const LIGHT: &str = include_str!("../../tests/fixtures/themes/rose-pine.toml");
const SOURCE: &str = "# Theme preview\n\nWrite with **bold**, *emphasis*, and a [link](https://example.com).\n\n> A quote stays readable.\n\nInline `code` and a fenced block:\n\n```rust\nlet theme = \"Omarchy\";\n```\n\nFocus keeps the current paragraph clear.\n";

struct Check {
    state: Rc<Writer>,
    editor: Rc<Editor>,
    directory: PathBuf,
    source: String,
    selection: (i32, i32),
    checks: RefCell<Vec<&'static str>>,
    done: RefCell<Option<Completion>>,
}

type Completion = Box<dyn FnOnce(Result<Vec<&'static str>>)>;

pub(super) fn run(state: Rc<Writer>, done: impl FnOnce(Result<Vec<&'static str>>) + 'static) {
    let directory = state.store.data_dir.join("theme");
    // Never replace a real desktop theme, even when someone runs the smoke binary directly.
    if std::env::var_os("TOBYWRITE_THEME_DIR").map(PathBuf::from) != Some(directory.clone()) {
        done(Err(anyhow::anyhow!(
            "theme smoke test requires TOBYWRITE_THEME_DIR=$TOBYWRITE_DATA_DIR/data/theme"
        )));
        return;
    }
    state.update_preferences(|prefs| {
        prefs.appearance = Appearance::System;
        prefs.focus_mode = false;
        prefs.show_syntax = false;
    });
    // The earlier smoke checks are finished. Close only this isolated process's
    // windows so a tiling desktop gives the theme preview enough room to render.
    let documents = state.documents.borrow().clone();
    for editor in documents {
        editor.allow_close.set(true);
        editor.window.close();
    }
    let editor = state.new_document(SOURCE);
    editor.buffer.begin_user_action();
    editor.buffer.insert_at_cursor("Draft ");
    editor.buffer.end_user_action();
    editor.buffer.select_range(
        &editor.buffer.iter_at_offset(30),
        &editor.buffer.iter_at_offset(40),
    );
    let check = Rc::new(Check {
        source: editor.text(),
        editor,
        state,
        directory,
        selection: (30, 40),
        checks: RefCell::new(Vec::new()),
        done: RefCell::new(Some(Box::new(done))),
    });
    if let Err(error) = replace_theme(&check, DARK) {
        finish(&check, Err(error));
        return;
    }
    later(check, dark_loaded);
}

fn later(check: Rc<Check>, next: fn(&Rc<Check>) -> Result<()>) {
    glib::timeout_add_local_once(
        Duration::from_millis(1300),
        move || match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| next(&check))) {
            Ok(Ok(())) => {}
            Ok(Err(error)) => finish(&check, Err(error)),
            Err(_) => finish(&check, Err(anyhow::anyhow!("theme check panicked"))),
        },
    );
}

fn replace_theme(check: &Check, source: &str) -> Result<()> {
    let next = check.directory.with_file_name("next-theme");
    std::fs::create_dir_all(&next)?;
    std::fs::write(next.join("colors.toml"), source)?;
    if check.directory.exists() {
        std::fs::remove_dir_all(&check.directory)?;
    }
    std::fs::rename(next, &check.directory)?;
    Ok(())
}

fn assert_document(check: &Check) -> Result<()> {
    ensure!(
        check.editor.text() == check.source,
        "theme preserves draft text"
    );
    ensure!(
        check.editor.document.borrow().dirty,
        "theme preserves dirty state"
    );
    ensure!(
        check.editor.buffer.can_undo(),
        "theme preserves undo history"
    );
    let (start, end) = check
        .editor
        .buffer
        .selection_bounds()
        .context("selection retained")?;
    ensure!(
        (start.offset(), end.offset()) == check.selection,
        "theme preserves selection"
    );
    Ok(())
}

fn assert_palette(check: &Check, expected: &Palette) -> Result<()> {
    ensure!(
        *check.state.palette.borrow() == *expected,
        "effective palette matches fixture"
    );
    assert_document(check)?;
    for (name, color) in [
        ("md-code", &expected.foreground),
        ("md-link", &expected.accent),
        ("md-quote", &expected.quote),
        ("md-marker", &expected.focus),
        ("md-focus", &expected.focus),
    ] {
        let tag = check
            .editor
            .buffer
            .tag_table()
            .lookup(name)
            .context("Markdown tag")?;
        ensure!(
            tag.foreground_rgba() == Some(gdk::RGBA::parse(color)?),
            "{name} recolored"
        );
    }
    ensure!(
        check.editor.view.color() == gdk::RGBA::parse(&expected.foreground)?,
        "editor CSS foreground"
    );
    Ok(())
}

fn dark_loaded(check: &Rc<Check>) -> Result<()> {
    assert_palette(check, &Palette::from_toml(DARK, false).unwrap())?;
    capture(&check.editor.window, "dark-editor")?;
    check
        .checks
        .borrow_mut()
        .push("Live Omarchy dark palette, Markdown tags, selection and undo preservation");
    let menu = find_widget::<gtk::MenuButton>(check.editor.window.upcast_ref(), &|menu| {
        menu.tooltip_text().as_deref() == Some("Document menu")
    })
    .context("document menu")?;
    menu.popup();
    later(check.clone(), dark_menu);
    Ok(())
}

fn dark_menu(check: &Rc<Check>) -> Result<()> {
    let menu = find_widget::<gtk::MenuButton>(check.editor.window.upcast_ref(), &|menu| {
        menu.tooltip_text().as_deref() == Some("Document menu")
    })
    .context("document menu")?;
    let popover = menu.popover().context("open menu popover")?;
    ensure!(popover.is_visible(), "document menu opens");
    capture(&popover, "dark-menu")?;
    menu.popdown();
    dialogs::settings(&check.state, &check.editor.window);
    later(check.clone(), dark_settings);
    Ok(())
}

fn dark_settings(check: &Rc<Check>) -> Result<()> {
    let window = named_window("Tobywrite Settings")?;
    capture(&window, "dark-settings")?;
    // Leave the settings window open across the theme replacement.
    replace_theme(check, LIGHT)?;
    later(check.clone(), light_loaded);
    Ok(())
}

fn light_loaded(check: &Rc<Check>) -> Result<()> {
    assert_palette(check, &Palette::from_toml(LIGHT, false).unwrap())?;
    let window = named_window("Tobywrite Settings")?;
    ensure!(
        window.color() == gdk::RGBA::parse(&check.state.palette.borrow().foreground)?,
        "open settings recolored"
    );
    capture(&window, "light-settings")?;
    window.close();
    check.editor.view.grab_focus();
    check
        .checks
        .borrow_mut()
        .push("Directory replacement reloads light theme and open settings");
    later(check.clone(), light_editor);
    Ok(())
}

fn light_editor(check: &Rc<Check>) -> Result<()> {
    capture(&check.editor.window, "light-editor")?;
    library::show(&check.state, &check.editor);
    later(check.clone(), light_library);
    Ok(())
}

fn light_library(check: &Rc<Check>) -> Result<()> {
    let window = named_window("Quick Open")?;
    let list = find_widget::<gtk::ListBox>(window.upcast_ref(), &|_| true)
        .context("Quick Open file list")?;
    let row = list.selected_row().context("selected recent file")?;
    let path = find_widget::<gtk::Label>(row.upcast_ref(), &|label| {
        label.has_css_class("library-path")
    })
    .context("selected file path")?;
    ensure!(
        path.color() == gdk::RGBA::parse(&check.state.palette.borrow().selection_foreground)?,
        "selected Quick Open path uses readable selection foreground"
    );
    capture(&window, "light-quick-open")?;
    window.close();
    std::fs::write(check.directory.join("colors.toml"), "background = invalid")?;
    later(check.clone(), malformed_theme);
    Ok(())
}

fn malformed_theme(check: &Rc<Check>) -> Result<()> {
    assert_palette(check, &Palette::from_toml(LIGHT, false).unwrap())?;
    std::fs::remove_dir_all(&check.directory)?;
    later(check.clone(), missing_theme);
    Ok(())
}

fn missing_theme(check: &Rc<Check>) -> Result<()> {
    assert_palette(check, &Palette::from_toml(LIGHT, false).unwrap())?;
    check
        .checks
        .borrow_mut()
        .push("Incomplete and missing theme files keep the last readable palette");
    replace_theme(check, DARK)?;
    check
        .state
        .update_preferences(|prefs| prefs.appearance = Appearance::Light);
    later(check.clone(), explicit_light);
    Ok(())
}

fn explicit_light(check: &Rc<Check>) -> Result<()> {
    assert_palette(check, &Palette::fallback(false))?;
    check
        .state
        .update_preferences(|prefs| prefs.appearance = Appearance::Dark);
    replace_theme(check, LIGHT)?;
    later(check.clone(), explicit_dark);
    Ok(())
}

fn explicit_dark(check: &Rc<Check>) -> Result<()> {
    assert_palette(check, &Palette::fallback(true))?;
    check
        .checks
        .borrow_mut()
        .push("Explicit Light and Dark remain fixed across opposite desktop themes");
    check.state.update_preferences(|prefs| {
        prefs.appearance = Appearance::System;
        prefs.focus_mode = true;
    });
    later(check.clone(), focus_and_dialog);
    Ok(())
}

fn focus_and_dialog(check: &Rc<Check>) -> Result<()> {
    assert_palette(check, &Palette::from_toml(LIGHT, false).unwrap())?;
    capture(&check.editor.window, "light-focus")?;
    let tag = check
        .editor
        .buffer
        .tag_table()
        .lookup("md-focus")
        .context("focus tag")?;
    ensure!(
        check.editor.buffer.end_iter().backward_char(),
        "document has text"
    );
    ensure!(
        check
            .editor
            .buffer
            .iter_at_offset(check.editor.buffer.char_count() - 3)
            .has_tag(&tag),
        "focus dimming applied outside selected paragraph: enabled={}, cursor={}",
        check.state.preferences.borrow().focus_mode,
        check.editor.buffer.cursor_position()
    );
    dialogs::choose(
        &check.editor.window,
        "Theme check",
        "The draft and its undo history stay intact.",
        &["Close"],
        0,
        |_| {},
    );
    later(check.clone(), dialog_visible);
    Ok(())
}

fn dialog_visible(check: &Rc<Check>) -> Result<()> {
    let dialog = gtk::Window::list_toplevels()
        .into_iter()
        .filter_map(|widget| widget.downcast::<gtk::Window>().ok())
        .find(|window| {
            window.is_visible()
                && find_widget::<gtk::Button>(window.upcast_ref(), &|button| {
                    button.label().as_deref() == Some("Close")
                })
                .is_some()
        })
        .context("native alert dialog")?;
    capture(&dialog, "light-dialog")?;
    find_widget::<gtk::Button>(dialog.upcast_ref(), &|button| {
        button.label().as_deref() == Some("Close")
    })
    .unwrap()
    .emit_clicked();
    check.editor.buffer.undo();
    ensure!(
        check.editor.text() == SOURCE,
        "theme changes retain original undo operation"
    );
    check.editor.buffer.redo();
    ensure!(
        check.editor.text() == check.source,
        "theme changes retain redo operation"
    );
    check
        .checks
        .borrow_mut()
        .push("System restoration, focus mode, native menus and dialogs, retained undo/redo");
    // Emulate GTK notifications within this test process, without touching
    // desktop settings. Remove the fixture to exercise a non-Omarchy desktop.
    std::fs::remove_dir_all(&check.directory)?;
    check.state.omarchy_palette.borrow_mut().take();
    check.state.apply_preferences();
    let settings = gtk::Settings::default().context("GTK settings")?;
    appearance::set_dark(&settings, false);
    later(check.clone(), native_system_light);
    Ok(())
}

fn native_system_light(check: &Rc<Check>) -> Result<()> {
    ensure!(
        *check.state.palette.borrow() == Palette::fallback(false),
        "GTK light notification followed without Omarchy"
    );
    let settings = gtk::Settings::default().context("GTK settings")?;
    appearance::set_dark(&settings, true);
    later(check.clone(), native_system_dark);
    Ok(())
}

fn native_system_dark(check: &Rc<Check>) -> Result<()> {
    ensure!(
        *check.state.palette.borrow() == Palette::fallback(true),
        "GTK dark notification followed without Omarchy"
    );
    check
        .state
        .update_preferences(|prefs| prefs.appearance = Appearance::Light);
    ensure!(
        !check.state.palette.borrow().dark,
        "explicit Light without Omarchy"
    );
    check
        .state
        .update_preferences(|prefs| prefs.appearance = Appearance::System);
    let settings = gtk::Settings::default().context("GTK settings")?;
    ensure!(
        check.state.gtk_override.get().is_none(),
        "returning to System releases local override"
    );
    ensure!(
        check.state.palette.borrow().dark == appearance::system_dark(&settings),
        "returning to System reads current GTK preference"
    );
    check
        .checks
        .borrow_mut()
        .push("Live GTK System appearance and override reset without Omarchy");
    finish(check, Ok(()));
    Ok(())
}

fn named_window(title: &str) -> Result<gtk::Window> {
    gtk::Window::list_toplevels()
        .into_iter()
        .filter_map(|widget| widget.downcast::<gtk::Window>().ok())
        .find(|window| window.is_visible() && window.title().as_deref() == Some(title))
        .with_context(|| format!("visible {title} window"))
}

fn find_widget<T: IsA<gtk::Widget> + glib::object::IsClass>(
    widget: &gtk::Widget,
    matches: &impl Fn(&T) -> bool,
) -> Option<T> {
    if let Some(candidate) = widget.downcast_ref::<T>()
        && matches(candidate)
    {
        return Some(candidate.clone());
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        if let Some(found) = find_widget(&widget, matches) {
            return Some(found);
        }
        child = widget.next_sibling();
    }
    None
}

fn capture(widget: &impl IsA<gtk::Widget>, name: &str) -> Result<()> {
    let Some(directory) = std::env::var_os("TOBYWRITE_THEME_CAPTURE_DIR").map(PathBuf::from) else {
        return Ok(());
    };
    std::fs::create_dir_all(&directory)?;
    let widget = widget.as_ref();
    let snapshot = gtk::Snapshot::new();
    gtk::WidgetPaintable::new(Some(widget)).snapshot(
        &snapshot,
        widget.width() as f64,
        widget.height() as f64,
    );
    let node = snapshot.to_node().context("widget render node")?;
    let native = widget.native().context("widget native")?;
    let renderer = native.renderer().context("native renderer")?;
    renderer
        .render_texture(&node, None)
        .save_to_png(directory.join(format!("{name}.png")))?;
    Ok(())
}

fn finish(check: &Check, result: Result<()>) {
    if let Some(done) = check.done.borrow_mut().take() {
        done(result.map(|()| check.checks.take()));
    }
}
