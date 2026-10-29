use super::*;

/// A stub overriding nothing: every call fails, so a test sees only failures it scripts.
struct Bare;

impl Sys for Bare {}

/// Guards that a failed `create_dir_all` is reported, not ignored, when the parent is a file.
#[test]
fn a_write_under_a_file_fails() {
    let dir = tempfile::tempdir().unwrap();
    let blocker = dir.path().join("blocker");
    std::fs::write(&blocker, b"x").unwrap();
    assert!(RealSys.write(&blocker.join("child.toml"), b"x").is_err());
}

/// Guards the trait defaults; a stub fails any call it does not script, never passing silently.
#[test]
fn a_bare_stub_fails_every_call() {
    let path = Path::new("/nowhere");
    assert!(Bare.git(path, &["status"]).is_err());
    assert!(Bare.exec(path, &["true".to_owned()]).is_err());
    assert!(Bare.read(path).is_err());
    assert!(Bare.write(path, b"x").is_err());
}
