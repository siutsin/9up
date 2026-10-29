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
    let command = git_command(
        Path::new("/repo"),
        &["fetch", "--porcelain"],
        vec![std::ffi::OsString::from("PATH")],
    );
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
        "GIT_PROXY_COMMAND",
        "SSH_ASKPASS",
        "GIT_CONFIG",
        "GIT_CONFIG_COUNT",
        "GIT_CONFIG_PARAMETERS",
        "GIT_CONFIG_SYSTEM",
        "GIT_CONFIG_GLOBAL",
        "GIT_EXEC_PATH",
        "GIT_NAMESPACE",
        "GIT_COMMON_DIR",
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

/// Guards the passed-name sweep: only `GIT_CONFIG_KEY_*`/`GIT_CONFIG_VALUE_*` names are removed.
#[test]
fn the_git_command_scrubs_passed_config_key_and_value_names() {
    let names = vec![
        std::ffi::OsString::from("GIT_CONFIG_KEY_0"),
        std::ffi::OsString::from("GIT_CONFIG_VALUE_0"),
        std::ffi::OsString::from("PATH"),
    ];
    let command = git_command(Path::new("/repo"), &["status"], names);
    let envs: Vec<(String, Option<String>)> = command
        .get_envs()
        .map(|(key, value)| {
            (
                key.to_string_lossy().into_owned(),
                value.map(|value| value.to_string_lossy().into_owned()),
            )
        })
        .collect();
    for name in ["GIT_CONFIG_KEY_0", "GIT_CONFIG_VALUE_0"] {
        assert!(
            envs.iter()
                .any(|(key, value)| key == name && value.is_none()),
            "{name} is not removed from the environment"
        );
    }
    assert!(
        !envs.iter().any(|(key, _)| key == "PATH"),
        "PATH must not be removed from the environment"
    );
}

/// Guards the atomic write: a successful write leaves only the target, no temp sibling.
#[test]
fn a_write_leaves_no_temp_sibling() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("config.toml");
    RealSys.write(&target, b"data").unwrap();
    assert_eq!(std::fs::read(&target).unwrap(), b"data");
    let entries: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].file_name(), "config.toml");
}

/// Guards the cap on both streams: output past it is an error naming the limit,
/// not unbounded memory.
#[cfg(unix)]
#[test]
fn capped_output_past_the_limit_fails() {
    for script in ["exec yes", "exec yes >&2"] {
        let mut command = std::process::Command::new("sh");
        command.args(["-c", script]);
        let error = run_capped(&mut command, 1024).unwrap_err();
        assert!(
            error.to_string().contains("output limit of 1024 bytes"),
            "error does not name the limit: {error}"
        );
    }
}

/// Guards the cap: output under it returns intact, with its status.
#[cfg(unix)]
#[test]
fn capped_output_under_the_limit_returns_output_and_status() {
    let mut command = std::process::Command::new("sh");
    command.args(["-c", "printf hello"]);
    let output = run_capped(&mut command, 1024).unwrap();
    assert!(output.status.success());
    assert_eq!(output.stdout, b"hello");
}

/// Guards that a non-zero exit is a status, not an error, and stderr stays intact.
#[cfg(unix)]
#[test]
fn capped_output_keeps_a_non_zero_status_and_stderr() {
    let mut command = std::process::Command::new("sh");
    command.args(["-c", "printf oops >&2; exit 3"]);
    let output = run_capped(&mut command, 1024).unwrap();
    assert_eq!(output.status.code(), Some(3));
    assert_eq!(output.stderr, b"oops");
}

/// A reader whose every read fails, the one state a real pipe cannot be put in.
#[cfg(unix)]
struct FailingRead;

#[cfg(unix)]
impl std::io::Read for FailingRead {
    /// Always fail, the way a broken pipe would surface to the reader thread.
    fn read(&mut self, _buffer: &mut [u8]) -> io::Result<usize> {
        Err(io::Error::other("read broke"))
    }
}

/// Guards the capped read's error path; a broken stream ends the read, not the run.
#[cfg(unix)]
#[test]
fn a_read_error_ends_the_capture() {
    let (buffer, over) = read_capped(Box::new(FailingRead), 1024);
    assert!(buffer.is_empty());
    assert!(!over);
}

/// Guards that a replacement file keeps the permissions of the file it replaces.
#[cfg(unix)]
#[test]
fn a_write_keeps_the_replaced_files_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    RealSys.write(&path, b"one").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    RealSys.write(&path, b"two").unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"two");
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

/// Guards the write's cleanup on a path with no file name
/// (`..`) fails its rename, and no temp file is left in the working directory.
#[test]
fn a_write_to_a_path_with_no_file_name_fails_and_cleans_up() {
    assert!(RealSys.write(Path::new(".."), b"x").is_err());
    let leftovers: Vec<_> = std::fs::read_dir(".")
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("9up.tmp-"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

/// Guards the capture setup: a child spawned without piped streams is an error
/// naming the missing stream, for stdout and stderr alike.
#[cfg(unix)]
#[test]
fn capture_reports_a_child_without_piped_streams() {
    let child = std::process::Command::new("true").spawn().unwrap();
    let error = capture(child, 1024).unwrap_err();
    assert!(
        error.to_string().contains("child stdout was not piped"),
        "error does not name the stream: {error}"
    );
    let child = std::process::Command::new("true")
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let error = capture(child, 1024).unwrap_err();
    assert!(
        error.to_string().contains("child stderr was not piped"),
        "error does not name the stream: {error}"
    );
}

/// Guards the cap against a pipe-holding descendant: killing the flooding child
/// must end the run even though a background grandchild holds the other pipe.
#[cfg(unix)]
#[test]
fn a_capped_run_ends_despite_a_pipe_holding_descendant() {
    let mut command = std::process::Command::new("sh");
    command.args(["-c", "sleep 10 >&2 & exec yes"]);
    let started = std::time::Instant::now();
    let error = run_capped(&mut command, 1024).unwrap_err();
    assert!(
        error.to_string().contains("output limit of 1024 bytes"),
        "error does not name the limit: {error}"
    );
    assert!(
        started.elapsed() < std::time::Duration::from_secs(8),
        "run waited on the descendant: {:?}",
        started.elapsed()
    );
}
