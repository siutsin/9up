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

/// Guards that a bare file name needs no directory created: it has no parent, so the
/// create step is skipped and the file lands in the working directory.
#[test]
fn a_write_to_a_bare_file_name_creates_it_in_place() {
    let name = format!("9up-sys-write-test-{}", std::process::id());
    let path = Path::new(&name);
    RealSys.write(path, b"data").unwrap();
    let read = std::fs::read(path);
    std::fs::remove_file(path).unwrap();
    assert_eq!(read.unwrap(), b"data");
}

/// Guards the git defences: the `-c` pairs lead the arguments in order, the inherited `GIT_*`
/// variables are removed rather than passed through, and the locale stays pinned.
#[test]
fn the_git_command_leads_with_overrides_and_scrubs_the_environment() {
    let command = git_command(Path::new("/repo"), &["fetch", "--porcelain"]);
    let args: Vec<String> = command
        .get_args()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        args,
        [
            "-c",
            "protocol.ext.allow=never",
            "-c",
            "core.sshCommand=ssh",
            "-c",
            "core.fsmonitor=",
            "-c",
            "core.hooksPath=/dev/null",
            "fetch",
            "--porcelain",
        ]
        .map(str::to_owned)
    );
    let envs: Vec<(String, Option<String>)> = command
        .get_envs()
        .map(|(key, value)| {
            (
                key.to_string_lossy().into_owned(),
                value.map(|value| value.to_string_lossy().into_owned()),
            )
        })
        .collect();
    for name in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_SSH_COMMAND",
        "GIT_SSH",
        "GIT_ASKPASS",
        "GIT_CONFIG",
        "GIT_CONFIG_COUNT",
        "GIT_CONFIG_PARAMETERS",
        "GIT_CONFIG_SYSTEM",
        "GIT_CONFIG_GLOBAL",
    ] {
        assert!(
            envs.iter()
                .any(|(key, value)| key == name && value.is_none()),
            "{name} is not removed from the environment"
        );
    }
    assert!(
        envs.iter()
            .any(|(key, value)| key == "LC_ALL" && value.as_deref() == Some("C")),
        "LC_ALL is not pinned to C"
    );
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
