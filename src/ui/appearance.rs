use super::palette::Palette;
use crate::storage::Appearance;
use gtk::{glib, prelude::*};

pub fn system_dark(settings: &gtk::Settings) -> bool {
    if settings
        .find_property("gtk-interface-color-scheme")
        .is_some()
    {
        let value = settings.property_value("gtk-interface-color-scheme");
        if let Some((_, scheme)) = glib::EnumValue::from_value(&value) {
            match scheme.nick() {
                "dark" => return true,
                "light" => return false,
                _ => {}
            }
        }
    }
    settings.is_gtk_application_prefer_dark_theme()
        || settings
            .gtk_theme_name()
            .is_some_and(|name| name.to_lowercase().contains("dark"))
}

pub fn set_dark(settings: &gtk::Settings, dark: bool) {
    // The replacement property arrived in GTK 4.20. Discover it at runtime
    // to keep the binary compatible with GTK 4.10 without deprecated warnings.
    if let Some(property) = settings.find_property("gtk-interface-color-scheme")
        && let Some(class) = glib::EnumClass::with_type(property.value_type())
        && let Some(value) = class.to_value_by_nick(if dark { "dark" } else { "light" })
    {
        settings.set_property_from_value("gtk-interface-color-scheme", &value);
    } else {
        settings.set_gtk_application_prefer_dark_theme(dark);
    }
}

/// Clear only this process's overrides so GTK follows desktop settings again.
pub fn reset_dark(settings: &gtk::Settings) {
    if settings
        .find_property("gtk-interface-color-scheme")
        .is_some()
    {
        settings.reset_property("gtk-interface-color-scheme");
    }
    settings.reset_property("gtk-application-prefer-dark-theme");
}

pub fn palette(appearance: Appearance, theme: Option<&Palette>, system_dark: bool) -> Palette {
    let dark = match appearance {
        Appearance::System => theme.map_or(system_dark, |palette| palette.dark),
        Appearance::Light => false,
        Appearance::Dark => true,
    };
    theme
        .filter(|palette| palette.dark == dark)
        .cloned()
        .unwrap_or_else(|| Palette::fallback(dark))
}
