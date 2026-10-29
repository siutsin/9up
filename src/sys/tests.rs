use super::*;

/// A stub that overrides nothing: every call fails, so a test
/// only ever sees the failures it scripts itself.
struct Bare;

impl Sys for Bare {}

#[test]
fn a_write_under_a_file_fails() {
    let dir = tempfile::tempdir().unwrap();
    let blocker = dir.path().join("blocker");
    std::fs::write(&blocker, b"x").unwrap();
    assert!(RealSys.write(&blocker.join("child.toml"), b"x").is_err());
}

#[test]
fn a_bare_stub_fails_every_call() {
    let path = Path::new("/nowhere");
    assert!(Bare.git(path, &["status"]).is_err());
    assert!(Bare.exec(path, &["true".to_owned()]).is_err());
    assert!(Bare.read(path).is_err());
    assert!(Bare.write(path, b"x").is_err());
}
