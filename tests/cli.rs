use std::{fs, process::Command};

fn binary() -> Command {
    Command::new(env!("CARGO_BIN_EXE_tobywrite"))
}

#[test]
fn terminal_commands_work_without_a_display_and_do_not_touch_sources() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("note with spaces.md");
    let target = directory.path().join("note.html");
    let text = "# Hello, æøå 🌱\r\n\r\n**A thought.**\r\n";
    let mut bytes = vec![0xff, 0xfe];
    for unit in text.encode_utf16() {
        bytes.extend(unit.to_le_bytes());
    }
    fs::write(&source, &bytes).unwrap();
    let checked = binary()
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .arg("--check")
        .arg(&source)
        .output()
        .unwrap();
    assert!(
        checked.status.success(),
        "{}",
        String::from_utf8_lossy(&checked.stderr)
    );
    assert!(String::from_utf8_lossy(&checked.stdout).contains("UTF-16"));
    let exported = binary()
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .arg("--export-html")
        .arg(&source)
        .arg(&target)
        .output()
        .unwrap();
    assert!(
        exported.status.success(),
        "{}",
        String::from_utf8_lossy(&exported.stderr)
    );
    let html = fs::read_to_string(&target).unwrap();
    assert!(html.contains("<strong>A thought.</strong>"));
    assert!(html.contains("æøå 🌱"));
    let again = binary()
        .arg("--export-html")
        .arg(&source)
        .arg(&target)
        .output()
        .unwrap();
    assert!(!again.status.success());
    assert_eq!(fs::read(&source).unwrap(), bytes);
    assert_eq!(fs::read_to_string(&target).unwrap(), html);
}

#[test]
fn rejects_binary_input_and_incomplete_commands() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("binary.md");
    fs::write(&source, b"\x89PNG\0not text").unwrap();
    assert!(
        !binary()
            .arg("--check")
            .arg(source)
            .output()
            .unwrap()
            .status
            .success()
    );
    assert!(
        !binary()
            .arg("--export-html")
            .output()
            .unwrap()
            .status
            .success()
    );
    assert!(binary().arg("--help").output().unwrap().status.success());
}
