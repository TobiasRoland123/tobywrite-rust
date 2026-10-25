//! Register bundled fonts for this process only, without changing desktop fonts.
use anyhow::{Result, ensure};
use std::{
    ffi::{CString, c_int, c_void},
    os::unix::ffi::OsStrExt,
};

#[link(name = "fontconfig")]
unsafe extern "C" {
    fn FcConfigGetCurrent() -> *mut c_void;
    fn FcConfigAppFontAddFile(config: *mut c_void, file: *const u8) -> c_int;
    fn FcConfigBuildFonts(config: *mut c_void) -> c_int;
}

pub fn register() -> Result<tempfile::TempDir> {
    let directory = tempfile::tempdir()?;
    let fonts: [(&str, &[u8]); 4] = [
        (
            "Regular.ttf",
            include_bytes!("../assets/fonts/iAWriterMonoS-Regular.ttf"),
        ),
        (
            "Bold.ttf",
            include_bytes!("../assets/fonts/iAWriterMonoS-Bold.ttf"),
        ),
        (
            "Italic.ttf",
            include_bytes!("../assets/fonts/iAWriterMonoS-Italic.ttf"),
        ),
        (
            "BoldItalic.ttf",
            include_bytes!("../assets/fonts/iAWriterMonoS-BoldItalic.ttf"),
        ),
    ];
    // SAFETY: Fontconfig owns the current configuration. Registration happens
    // before GTK/Pango starts, all paths are NUL-terminated and live for each call.
    let config = unsafe { FcConfigGetCurrent() };
    ensure!(!config.is_null(), "Fontconfig has no configuration");
    for (name, data) in fonts {
        let path = directory.path().join(name);
        std::fs::write(&path, data)?;
        let path_c = CString::new(path.as_os_str().as_bytes())?;
        ensure!(
            unsafe { FcConfigAppFontAddFile(config, path_c.as_ptr().cast()) } != 0,
            "Cannot register {name}"
        );
    }
    ensure!(
        unsafe { FcConfigBuildFonts(config) } != 0,
        "Cannot refresh application fonts"
    );
    Ok(directory)
}
