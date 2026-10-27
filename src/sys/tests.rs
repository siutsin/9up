use super::*;

/// A stub that overrides nothing: every call fails, so a test
/// only ever sees the failures it scripts itself.
struct Bare;

impl Sys for Bare {}

#[test]
fn a_bare_stub_fails_every_call() {
    let path = Path::new("/nowhere");
    assert!(Bare.git(path, &["status"]).is_err());
    assert!(Bare.exec(path, &["true".to_owned()]).is_err());
    assert!(Bare.read(path).is_err());
    assert!(Bare.write(path, b"x").is_err());
}
