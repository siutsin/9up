use super::*;
use std::fs;
use std::io::Write;
use std::path::Path;

use crate::render::tests::FailingWriter;

/// A system that serves one in-memory config file and answers every git call with empty
/// success. Its write can be set to fail, so a command's save can be made to fail.
struct MemorySys {
    config: Vec<u8>,
    write_fails: bool,
}

impl crate::sys::Sys for MemorySys {
    /// Succeed with no output, so every repository reads as having no remotes.
    fn git(&self, _dir: &Path, _argv: &[&str]) -> io::Result<std::process::Output> {
        Ok(std::process::Output {
            status: std::os::unix::process::ExitStatusExt::from_raw(0),
            stdout: Vec::new(),
            stderr: Vec::new(),
        })
    }

    /// Serve the in-memory config, so each test sets its own without disk.
    fn read(&self, _path: &Path) -> io::Result<Vec<u8>> {
        Ok(self.config.clone())
    }

    /// Accept the write unless the test set `write_fails`, so a save fails there instead.
    fn write(&self, _path: &Path, _contents: &[u8]) -> io::Result<()> {
        if self.write_fails {
            Err(io::Error::other("write refused"))
        } else {
            Ok(())
        }
    }
}

/// Build an `App` around `MemorySys`, so a test needs only a config and a writer.
fn test_app<'a>(config: Vec<u8>, cwd: &Path, out: &'a mut (dyn Write + 'a)) -> App<'a> {
    app_with(
        Box::new(MemorySys {
            config,
            write_fails: false,
        }),
        cwd,
        out,
    )
}

/// Create a fake repository under `dir` and return the config bytes that name it.
fn repo_config(dir: &Path) -> Vec<u8> {
    let repo = dir.join("repo");
    fs::create_dir_all(&repo).unwrap();
    crate::discover::tests::fake_repo(&repo);
    format!("repositories = [\"{}\"]\n", repo.display()).into_bytes()
}

/// Guards the empty config hint, so a new user learns how to add a repository.
#[test]
fn update_with_no_configured_repositories_prints_the_hint() {
    let dir = tempfile::tempdir().unwrap();
    let mut out: Vec<u8> = Vec::new();
    let mut app = test_app(Vec::new(), dir.path(), &mut out);
    assert!(update(&mut app, &UpdateArgs::default()).is_ok());
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("No repositories configured."), "{text}");
}

/// Guards that every command returns its output write error, so `main` can decide the
/// exit code. Each row is one command meeting a failing writer at one of its writes.
#[test]
fn commands_report_a_failed_write() {
    let dir = tempfile::tempdir().unwrap();
    let repo = repo_config(dir.path());
    let kept = dir.path().join("kept");
    fs::create_dir_all(&kept).unwrap();
    let kept_config = format!("repositories = [\"{}\"]\n", kept.display()).into_bytes();
    let missing: Vec<u8> = b"repositories = [\"/nonexistent-9up-test\"]\n".to_vec();
    let stored: Vec<u8> = b"repositories = [\"/x\"]\n".to_vec();
    let empty: Vec<u8> = b"repositories = []\n".to_vec();
    type Call = fn(&mut App<'_>) -> io::Result<ExitCode>;
    let cases: Vec<(&str, Vec<u8>, Call)> = vec![
        ("update hint", Vec::new(), |app| {
            update(app, &UpdateArgs::default())
        }),
        ("update header", repo.clone(), |app| {
            update(app, &UpdateArgs::default())
        }),
        ("exec missing path", missing.clone(), |app| {
            exec(app, &[], 3, &["true".to_owned()])
        }),
        (
            "exec comment",
            b"repositories = [\"# note\"]\n".to_vec(),
            |app| exec(app, &[], 3, &["true".to_owned()]),
        ),
        ("exec header", repo, |app| {
            exec(app, &[], 3, &["true".to_owned()])
        }),
        ("add", empty.clone(), |app| add(app, &["/x".to_owned()])),
        ("add stored", stored.clone(), |app| {
            add(app, &["/x".to_owned()])
        }),
        ("delete", stored.clone(), |app| {
            delete(app, &["/x".to_owned()])
        }),
        ("delete missing", empty.clone(), |app| {
            delete(app, &["/x".to_owned()])
        }),
        ("list", stored.clone(), |app| list(app)),
        ("list empty", empty, |app| list(app)),
        ("clean removal", missing.clone(), |app| clean(app)),
        ("clean empty", b"repositories = []\n".to_vec(), |app| {
            clean(app)
        }),
        ("clean nothing", kept_config, |app| clean(app)),
    ];
    for (name, config, call) in cases {
        let mut out = FailingWriter;
        let mut app = app_with(
            Box::new(MemorySys {
                config,
                write_fails: false,
            }),
            dir.path(),
            &mut out,
        );
        assert_eq!(
            call(&mut app).unwrap_err().kind(),
            io::ErrorKind::BrokenPipe,
            "{name}"
        );
    }
}

/// Guards the whole `update` path from config to output through the `App`.
#[test]
fn update_a_repository_through_the_app() {
    let dir = tempfile::tempdir().unwrap();
    let config = repo_config(dir.path());
    let mut out: Vec<u8> = Vec::new();
    let mut app = test_app(config, dir.path(), &mut out);
    assert!(update(&mut app, &UpdateArgs::default()).is_ok());
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("(1 repository)"), "{text}");
    assert!(text.contains("No remotes."), "{text}");
}

#[test]
/// Guards that `exec` reports a configured path that is gone, so the user can fix the config.
fn exec_prints_a_missing_path_through_the_app() {
    let dir = tempfile::tempdir().unwrap();
    let config = b"repositories = [\"/nonexistent-9up-test\"]\n".to_vec();
    let mut out: Vec<u8> = Vec::new();
    let mut app = test_app(config, dir.path(), &mut out);
    assert!(exec(&mut app, &[], 3, &["true".to_owned()]).is_ok());
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("doesn't exist."), "{text}");
}

/// Guards that an empty `#` entry prints nothing, so a blank comment adds no noise.
#[test]
fn exec_prints_nothing_for_an_empty_comment() {
    let dir = tempfile::tempdir().unwrap();
    let config = b"repositories = [\"#\"]\n".to_vec();
    let mut out: Vec<u8> = Vec::new();
    let mut app = test_app(config, dir.path(), &mut out);
    assert!(exec(&mut app, &[], 3, &["true".to_owned()]).is_ok());
    assert!(out.is_empty(), "{}", String::from_utf8_lossy(&out));
}

/// Guards that `update` prints comment entries and missing paths beside real repositories.
#[test]
fn update_prints_comments_and_missing_paths() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    crate::discover::tests::fake_repo(&repo);
    let config = format!(
        "repositories = [\"# note\", \"/nonexistent-9up-test\", \"{}\"]\n",
        repo.display()
    )
    .into_bytes();
    let mut out: Vec<u8> = Vec::new();
    let mut app = test_app(config, dir.path(), &mut out);
    assert!(update(&mut app, &UpdateArgs::default()).is_ok());
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("note"), "{text}");
    assert!(text.contains("doesn't exist."), "{text}");
}

/// Guards that a path argument replaces the config, so one repository can be updated alone.
#[test]
fn update_runs_on_an_explicitly_named_path() {
    let dir = tempfile::tempdir().unwrap();
    let config = repo_config(dir.path());
    let repo = dir.path().join("repo");
    let mut out: Vec<u8> = Vec::new();
    let mut app = test_app(config, dir.path(), &mut out);
    let args = UpdateArgs {
        paths: vec![repo.display().to_string()],
        ..Default::default()
    };
    assert!(update(&mut app, &args).is_ok());
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("(1 repository)"), "{text}");
}

/// A system that reads a repository and runs the command, to drive `exec` end to end via `App`.
struct RepoExecSys;

impl crate::sys::Sys for RepoExecSys {
    /// Answer the `rev-parse` calls `exec_repo` makes, so the fake repository reads as real.
    fn git(&self, _dir: &Path, argv: &[&str]) -> io::Result<std::process::Output> {
        let stdout = match argv {
            ["rev-parse", "--is-bare-repository"] => "false\n",
            ["rev-parse", "--show-toplevel"] => "/repo\n",
            _ => "",
        };
        Ok(std::process::Output {
            status: std::os::unix::process::ExitStatusExt::from_raw(0),
            stdout: stdout.as_bytes().to_vec(),
            stderr: Vec::new(),
        })
    }

    /// Print fixed output, so the test can see the command's output reach the writer.
    fn exec(&self, _dir: &Path, _argv: &[String]) -> io::Result<std::process::Output> {
        Ok(std::process::Output {
            status: std::os::unix::process::ExitStatusExt::from_raw(0),
            stdout: b"hello\n".to_vec(),
            stderr: Vec::new(),
        })
    }
}

/// Guards that `exec` runs in a named repository and prints the command's output.
#[test]
fn exec_runs_in_an_explicitly_named_repository() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    crate::discover::tests::fake_repo(&repo);
    let mut out: Vec<u8> = Vec::new();
    let mut app = App {
        sys: Box::new(RepoExecSys),
        cwd: dir.path().to_path_buf(),
        home: None,
        config_file: dir.path().join("config.toml"),
        colour: false,
        out: &mut out,
    };
    let paths = vec![repo.display().to_string()];
    assert!(exec(&mut app, &paths, 3, &["ls".to_owned()]).is_ok());
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("hello"), "{text}");
}

/// Guards that a broken config fails `exec` rather than running on nothing.
#[test]
fn exec_reports_an_unparseable_config() {
    let dir = tempfile::tempdir().unwrap();
    let mut out: Vec<u8> = Vec::new();
    let mut app = test_app(b"repositories = [\n".to_vec(), dir.path(), &mut out);
    assert!(exec(&mut app, &[], 3, &["true".to_owned()]).is_err());
}

/// A system that fails every call, so `exec_repo` can only report an unreadable repository.
struct FailingSys;

impl crate::sys::Sys for FailingSys {}

/// Build an `App` around a given system, for tests that need a stub `test_app` cannot make.
fn app_with<'a>(
    sys: Box<dyn crate::sys::Sys>,
    cwd: &Path,
    out: &'a mut (dyn Write + 'a),
) -> App<'a> {
    App {
        sys,
        cwd: cwd.to_path_buf(),
        home: None,
        config_file: cwd.join("config.toml"),
        colour: false,
        out,
    }
}

/// A writer that accepts a fixed number of bytes, then fails: a test can let the group
/// header through and fail the write that follows it.
struct BudgetWriter {
    remaining: usize,
}

impl Write for BudgetWriter {
    /// Take bytes while the budget lasts; the first write past it fails.
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if buf.len() > self.remaining {
            return Err(io::Error::other("write budget spent"));
        }
        self.remaining -= buf.len();
        Ok(buf.len())
    }

    /// Always succeeds: accepted writes are only counted, never buffered.
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Guards that `add` stores a new path and says so.
#[test]
fn add_stores_a_new_path() {
    let dir = tempfile::tempdir().unwrap();
    let mut out: Vec<u8> = Vec::new();
    let mut app = test_app(b"repositories = []\n".to_vec(), dir.path(), &mut out);
    let code = add(&mut app, &["/x".to_owned()]).unwrap();
    assert_eq!(code, ExitCode::SUCCESS);
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("Added /x."), "{text}");
}

/// Guards that `add` reports a stored path instead of storing it twice.
#[test]
fn add_reports_a_path_that_is_already_stored() {
    let dir = tempfile::tempdir().unwrap();
    let mut out: Vec<u8> = Vec::new();
    let mut app = test_app(b"repositories = [\"/x\"]\n".to_vec(), dir.path(), &mut out);
    assert!(add(&mut app, &["/x".to_owned()]).is_ok());
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("already in the config file."), "{text}");
}

/// Guards that `add` fails on a broken config rather than starting from nothing.
#[test]
fn add_reports_an_unparseable_config() {
    let dir = tempfile::tempdir().unwrap();
    let mut out: Vec<u8> = Vec::new();
    let mut app = test_app(b"repositories = [\n".to_vec(), dir.path(), &mut out);
    assert!(add(&mut app, &["/x".to_owned()]).is_err());
}

/// Guards that `add` saves before it reports: a failed save errors, names the config
/// file, and prints no success line.
#[test]
fn add_reports_a_failed_save() {
    let dir = tempfile::tempdir().unwrap();
    let mut out: Vec<u8> = Vec::new();
    let sys = MemorySys {
        config: b"repositories = []\n".to_vec(),
        write_fails: true,
    };
    let mut app = app_with(Box::new(sys), dir.path(), &mut out);
    let err = add(&mut app, &["/x".to_owned()]).unwrap_err();
    assert!(err.to_string().contains("config.toml"), "{err}");
    let text = String::from_utf8(out).unwrap();
    assert!(!text.contains("Added /x."), "{text}");
}

/// Guards that `delete` reports a path that was never stored and changes nothing.
#[test]
fn delete_reports_a_path_that_is_not_stored() {
    let dir = tempfile::tempdir().unwrap();
    let mut out: Vec<u8> = Vec::new();
    let mut app = test_app(b"repositories = [\"/x\"]\n".to_vec(), dir.path(), &mut out);
    assert!(delete(&mut app, &["/y".to_owned()]).is_ok());
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("not in the config file."), "{text}");
}

/// Guards that `delete` removes a stored path and saves.
#[test]
fn delete_removes_a_stored_path() {
    let dir = tempfile::tempdir().unwrap();
    let mut out: Vec<u8> = Vec::new();
    let mut app = test_app(b"repositories = [\"/x\"]\n".to_vec(), dir.path(), &mut out);
    assert!(delete(&mut app, &["/x".to_owned()]).is_ok());
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("Deleted /x."), "{text}");
}

/// Guards that `delete` saves before it reports: a failed save errors, names the config
/// file, and prints no success line.
#[test]
fn delete_reports_a_failed_save() {
    let dir = tempfile::tempdir().unwrap();
    let mut out: Vec<u8> = Vec::new();
    let sys = MemorySys {
        config: b"repositories = [\"/x\"]\n".to_vec(),
        write_fails: true,
    };
    let mut app = app_with(Box::new(sys), dir.path(), &mut out);
    let err = delete(&mut app, &["/x".to_owned()]).unwrap_err();
    assert!(err.to_string().contains("config.toml"), "{err}");
    let text = String::from_utf8(out).unwrap();
    assert!(!text.contains("Deleted /x."), "{text}");
}

/// Guards that `delete` fails on a broken config rather than guessing at its entries.
#[test]
fn delete_reports_an_unparseable_config() {
    let dir = tempfile::tempdir().unwrap();
    let mut out: Vec<u8> = Vec::new();
    let mut app = test_app(b"repositories = [\n".to_vec(), dir.path(), &mut out);
    assert!(delete(&mut app, &["/x".to_owned()]).is_err());
}

/// Guards that `list` says an empty config is empty, so blank output is never ambiguous.
#[test]
fn list_prints_the_empty_message() {
    let dir = tempfile::tempdir().unwrap();
    let mut out: Vec<u8> = Vec::new();
    let mut app = test_app(b"repositories = []\n".to_vec(), dir.path(), &mut out);
    assert!(list(&mut app).is_ok());
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("No repositories configured."), "{text}");
}

/// Guards that `list` prints a stored entry with its control characters replaced,
/// so an entry cannot inject escapes or forge a line.
#[test]
fn list_cleans_control_characters_in_an_entry() {
    let dir = tempfile::tempdir().unwrap();
    let mut out: Vec<u8> = Vec::new();
    let config = b"repositories = [\"/a\\u001B[31m\\nforged\"]\n".to_vec();
    let mut app = test_app(config, dir.path(), &mut out);
    assert!(list(&mut app).is_ok());
    let text = String::from_utf8(out).unwrap();
    assert_eq!(text, "/a [31m forged\n");
}

/// Guards that `list` fails on a broken config rather than printing nothing.
#[test]
fn list_reports_an_unparseable_config() {
    let dir = tempfile::tempdir().unwrap();
    let mut out: Vec<u8> = Vec::new();
    let mut app = test_app(b"repositories = [\n".to_vec(), dir.path(), &mut out);
    assert!(list(&mut app).is_err());
}

/// Guards that `update` with no paths fails on a broken config, not on an empty list.
#[test]
fn update_reports_an_unparseable_config() {
    let dir = tempfile::tempdir().unwrap();
    let mut out: Vec<u8> = Vec::new();
    let mut app = test_app(b"repositories = [\n".to_vec(), dir.path(), &mut out);
    assert!(update(&mut app, &UpdateArgs::default()).is_err());
}

/// Guards that `clean` says an empty config is empty.
#[test]
fn clean_prints_the_empty_message() {
    let dir = tempfile::tempdir().unwrap();
    let mut out: Vec<u8> = Vec::new();
    let mut app = test_app(b"repositories = []\n".to_vec(), dir.path(), &mut out);
    assert!(clean(&mut app).is_ok());
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("No repositories configured."), "{text}");
}

/// Guards that `clean` fails on a broken config rather than pruning blind.
#[test]
fn clean_reports_an_unparseable_config() {
    let dir = tempfile::tempdir().unwrap();
    let mut out: Vec<u8> = Vec::new();
    let mut app = test_app(b"repositories = [\n".to_vec(), dir.path(), &mut out);
    assert!(clean(&mut app).is_err());
}

/// Guards that `clean` keeps a comment entry; it is not a path the liveness test can judge.
#[test]
fn clean_keeps_a_comment_entry() {
    let dir = tempfile::tempdir().unwrap();
    let mut out: Vec<u8> = Vec::new();
    let mut app = test_app(
        b"repositories = [\"# note\"]\n".to_vec(),
        dir.path(),
        &mut out,
    );
    assert!(clean(&mut app).is_ok());
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("Nothing to clean."), "{text}");
}

/// Guards that `clean` keeps an entry whose path still exists and saves nothing.
#[test]
fn clean_keeps_a_live_entry() {
    let dir = tempfile::tempdir().unwrap();
    let kept = dir.path().join("kept");
    fs::create_dir_all(&kept).unwrap();
    let config = format!("repositories = [\"{}\"]\n", kept.display()).into_bytes();
    let mut out: Vec<u8> = Vec::new();
    let mut app = test_app(config, dir.path(), &mut out);
    assert!(clean(&mut app).is_ok());
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("Nothing to clean."), "{text}");
}

/// Guards that `clean` saves before it reports: a failed save errors, names the config
/// file, and prints no removal line.
#[test]
fn clean_reports_a_failed_save() {
    let dir = tempfile::tempdir().unwrap();
    let mut out: Vec<u8> = Vec::new();
    let sys = MemorySys {
        config: b"repositories = [\"/nonexistent-9up-test\"]\n".to_vec(),
        write_fails: true,
    };
    let mut app = app_with(Box::new(sys), dir.path(), &mut out);
    let err = clean(&mut app).unwrap_err();
    assert!(err.to_string().contains("config.toml"), "{err}");
    let text = String::from_utf8(out).unwrap();
    assert!(!text.contains("Removed /nonexistent-9up-test."), "{text}");
}

/// Guards that `exec` prints a comment entry from the config.
#[test]
fn exec_prints_a_comment_entry() {
    let dir = tempfile::tempdir().unwrap();
    let config = b"repositories = [\"# note\"]\n".to_vec();
    let mut out: Vec<u8> = Vec::new();
    let mut app = test_app(config, dir.path(), &mut out);
    assert!(exec(&mut app, &[], 3, &["true".to_owned()]).is_ok());
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("note"), "{text}");
}

/// Guards that a write failing after the group header, on the repository name line,
/// stops `exec` with an error.
#[test]
fn exec_reports_a_failed_repository_name_write() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    crate::discover::tests::fake_repo(&repo);
    // The budget ends exactly with the group header, so the name line is the failing write.
    let header = format!("{} (1 repository)\n", repo.display());
    let mut out = BudgetWriter {
        remaining: header.len(),
    };
    let mut app = app_with(Box::new(RepoExecSys), dir.path(), &mut out);
    let paths = vec![repo.display().to_string()];
    assert!(exec(&mut app, &paths, 3, &["ls".to_owned()]).is_err());
}

/// Guards that a write failing on a command output line stops `exec` with an error.
#[test]
fn exec_reports_a_failed_output_line_write() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    crate::discover::tests::fake_repo(&repo);
    // The budget ends with the header and the name line, so the output line fails.
    let header = format!("{} (1 repository)\n", repo.display());
    let mut out = BudgetWriter {
        remaining: header.len() + "  repo\n".len(),
    };
    let mut app = app_with(Box::new(RepoExecSys), dir.path(), &mut out);
    let paths = vec![repo.display().to_string()];
    assert!(exec(&mut app, &paths, 3, &["ls".to_owned()]).is_err());
}

/// Guards that `exec` prints the error line for a repository it cannot read.
#[test]
fn exec_prints_a_repository_error() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    crate::discover::tests::fake_repo(&repo);
    let mut out: Vec<u8> = Vec::new();
    let mut app = app_with(Box::new(FailingSys), dir.path(), &mut out);
    let paths = vec![repo.display().to_string()];
    assert!(exec(&mut app, &paths, 3, &["ls".to_owned()]).is_ok());
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("Could not read the repository."), "{text}");
}

// Concurrency tests: a latch stub holds each repository's start call until the
// test releases it, so overlap, ordering and the bound are observed from the
// latch's records, never from elapsed time.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};

/// How long any latch or writer wait may block before the test fails as
/// deadlocked. A valve against a hang, not a timing assertion.
const WAIT_LIMIT: std::time::Duration = std::time::Duration::from_secs(60);

/// Which call of a repository's work the latch treats as its start and finish.
#[derive(Clone, Copy, PartialEq, Eq)]
enum LatchMode {
    /// `update`: the `config --get-regexp` call starts a repository; the
    /// `for-each-ref` call, the last in its script, finishes it.
    Update,
    /// `exec`: the `exec` call starts and finishes a repository; the
    /// `rev-parse` calls before it stay outside the latch.
    Exec,
}

/// Where a scripted worker panic fires.
#[derive(Clone, Copy, PartialEq, Eq)]
enum PanicAt {
    /// Inside the start call, after it has registered.
    Start,
    /// On the repository's fetch call (`update` mode only).
    Fetch,
}

/// The latch's records: which repositories started and finished, how many were
/// ever in flight at once, and how many config reads happened.
struct Latch {
    state: Mutex<LatchState>,
    cond: Condvar,
}

struct LatchState {
    started: HashSet<PathBuf>,
    finished: HashSet<PathBuf>,
    in_flight: usize,
    max_in_flight: usize,
    released: HashSet<PathBuf>,
    release_all: bool,
    reads: usize,
}

impl Latch {
    /// Start empty: nothing started or finished, no reads counted.
    fn new() -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(LatchState {
                started: HashSet::new(),
                finished: HashSet::new(),
                in_flight: 0,
                max_in_flight: 0,
                released: HashSet::new(),
                release_all: false,
                reads: 0,
            }),
            cond: Condvar::new(),
        })
    }

    /// Wait until `done` holds on the state; the wait limit turns a deadlock
    /// into a test failure that says what it was waiting for.
    fn wait_until(&self, what: &str, done: impl Fn(&LatchState) -> bool) {
        let mut state = self.state.lock().unwrap();
        while !done(&state) {
            let (guard, timeout) = self.cond.wait_timeout(state, WAIT_LIMIT).unwrap();
            state = guard;
            if timeout.timed_out() && !done(&state) {
                panic!("timed out waiting for {what}");
            }
        }
    }

    /// Register a repository's start. In blocking mode the worker then waits
    /// here until the test releases it, so several workers can be inside their
    /// start calls at once.
    fn begin(&self, dir: &Path, blocking: bool) {
        let mut state = self.state.lock().unwrap();
        state.started.insert(dir.to_path_buf());
        state.in_flight += 1;
        state.max_in_flight = state.max_in_flight.max(state.in_flight);
        self.cond.notify_all();
        if blocking {
            while !state.release_all && !state.released.contains(dir) {
                let (guard, timeout) = self.cond.wait_timeout(state, WAIT_LIMIT).unwrap();
                state = guard;
                if timeout.timed_out() && !state.release_all && !state.released.contains(dir) {
                    panic!("latch timed out holding {}", dir.display());
                }
            }
        }
    }

    /// Mark a repository finished and free its in-flight slot, waking any waiter.
    fn finish(&self, dir: &Path) {
        let mut state = self.state.lock().unwrap();
        state.finished.insert(dir.to_path_buf());
        state.in_flight -= 1;
        self.cond.notify_all();
    }

    /// Release one held repository, leaving the others held.
    fn release(&self, dir: &Path) {
        self.state
            .lock()
            .unwrap()
            .released
            .insert(dir.to_path_buf());
        self.cond.notify_all();
    }

    /// Release every held repository, and any that start later.
    fn release_all(&self) {
        self.state.lock().unwrap().release_all = true;
        self.cond.notify_all();
    }

    /// Wait until at least `count` repositories have started.
    fn wait_for_started(&self, count: usize) {
        self.wait_until("more repositories to start", |state| {
            state.started.len() >= count
        });
    }

    /// Wait until `dir` has finished.
    fn wait_for_finished(&self, dir: &Path) {
        let dir = dir.to_path_buf();
        self.wait_until("a repository to finish", |state| {
            state.finished.contains(&dir)
        });
    }

    /// A snapshot of the repositories that have started.
    fn started(&self) -> HashSet<PathBuf> {
        self.state.lock().unwrap().started.clone()
    }

    /// A snapshot of the repositories that have finished.
    fn finished(&self) -> HashSet<PathBuf> {
        self.state.lock().unwrap().finished.clone()
    }

    /// The peak number in flight at once, so a test can check the batch bound held.
    fn max_in_flight(&self) -> usize {
        self.state.lock().unwrap().max_in_flight
    }

    /// The config reads counted so far.
    fn reads(&self) -> usize {
        self.state.lock().unwrap().reads
    }

    /// Count one config read.
    fn note_read(&self) {
        self.state.lock().unwrap().reads += 1;
    }
}

/// A successful output carrying `stdout` and an empty stderr.
fn latch_output(stdout: &str) -> std::process::Output {
    std::process::Output {
        status: std::os::unix::process::ExitStatusExt::from_raw(0),
        stdout: stdout.as_bytes().to_vec(),
        stderr: Vec::new(),
    }
}

/// A `Sys` that holds each repository's start call on the latch. Keyed by
/// directory: `--show-toplevel` answers with the directory the call received,
/// so each repository's work directory is the repository itself.
struct LatchSys {
    latch: Arc<Latch>,
    mode: LatchMode,
    blocking: bool,
    config: Vec<u8>,
    panics: HashMap<PathBuf, PanicAt>,
}

impl LatchSys {
    /// Serve `mode`'s script over `latch`; starts hold only when `blocking` is set.
    fn new(latch: &Arc<Latch>, mode: LatchMode, blocking: bool) -> Self {
        Self {
            latch: latch.clone(),
            mode,
            blocking,
            config: Vec::new(),
            panics: HashMap::new(),
        }
    }

    /// Panic when `dir`'s script reaches `at`, as a worker bug would.
    fn panic_if(&self, dir: &Path, at: PanicAt) {
        if self.panics.get(dir) == Some(&at) {
            panic!("scripted panic in {}", dir.display());
        }
    }
}

impl crate::sys::Sys for LatchSys {
    /// Answer from the script: in update mode the config call latches the start
    /// and `for-each-ref` records the finish.
    fn git(&self, dir: &Path, argv: &[&str]) -> io::Result<std::process::Output> {
        match argv {
            ["config", "--get-regexp", ..] => {
                if self.mode == LatchMode::Update {
                    self.latch.begin(dir, self.blocking);
                    self.panic_if(dir, PanicAt::Start);
                }
                Ok(latch_output(
                    "remote.origin.url /srv/git.git\n\
                     remote.origin.fetch +refs/heads/*:refs/remotes/origin/*\n",
                ))
            }
            ["fetch", ..] => {
                self.panic_if(dir, PanicAt::Fetch);
                Ok(latch_output(""))
            }
            ["rev-parse", "--is-bare-repository"] => Ok(latch_output("false\n")),
            ["rev-parse", "--show-toplevel"] => Ok(latch_output(&format!("{}\n", dir.display()))),
            ["symbolic-ref", "-q", "HEAD"] => Ok(latch_output("refs/heads/main\n")),
            ["for-each-ref", ..] => {
                if self.mode == LatchMode::Update {
                    self.latch.finish(dir);
                }
                Ok(latch_output(""))
            }
            _ => Ok(latch_output("")),
        }
    }

    /// The `exec` call is the latched call in exec mode: entry is the start,
    /// exit is the finish.
    fn exec(&self, dir: &Path, _argv: &[String]) -> io::Result<std::process::Output> {
        if self.mode == LatchMode::Exec {
            self.latch.begin(dir, self.blocking);
            self.panic_if(dir, PanicAt::Start);
            self.latch.finish(dir);
        }
        Ok(latch_output("hello\n"))
    }

    /// Serve the fixture's config bytes and count the read, so a test can
    /// assert the configured entries load exactly once, or never.
    fn read(&self, _path: &Path) -> io::Result<Vec<u8>> {
        self.latch.note_read();
        Ok(self.config.clone())
    }
}

/// A writer shared by the run and test threads: it records every write attempt,
/// can fail the write that completes a needle, and can hold one write open —
/// running a hook first — until the test releases it.
#[derive(Clone)]
struct TestWriter {
    inner: Arc<WriterInner>,
}

struct WriterInner {
    state: Mutex<WriterState>,
    cond: Condvar,
}

struct WriterState {
    attempts: Vec<u8>,
    text: Vec<u8>,
    fail_needle: Option<String>,
    failed: bool,
    block_needle: Option<String>,
    block_hook: Option<Box<dyn FnOnce() + Send>>,
    blocked: bool,
    block_released: bool,
}

impl TestWriter {
    /// A writer that records its writes and neither fails nor blocks.
    fn new() -> Self {
        Self {
            inner: Arc::new(WriterInner {
                state: Mutex::new(WriterState {
                    attempts: Vec::new(),
                    text: Vec::new(),
                    fail_needle: None,
                    failed: false,
                    block_needle: None,
                    block_hook: None,
                    blocked: false,
                    block_released: false,
                }),
                cond: Condvar::new(),
            }),
        }
    }

    /// Fail the first write that completes `needle`, with a fixed error kind.
    fn failing_on(needle: &str) -> Self {
        let writer = Self::new();
        writer.inner.state.lock().unwrap().fail_needle = Some(needle.to_owned());
        writer
    }

    /// Hold the write that completes `needle` open; `hook` runs first, while
    /// the render is in flight.
    fn blocking_on(needle: &str, hook: impl FnOnce() + Send + 'static) -> Self {
        let writer = Self::new();
        {
            let mut state = writer.inner.state.lock().unwrap();
            state.block_needle = Some(needle.to_owned());
            state.block_hook = Some(Box::new(hook));
        }
        writer
    }

    /// The output that was written successfully.
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.inner.state.lock().unwrap().text).into_owned()
    }

    /// Whether any write, successful or failed, carried `needle`.
    fn attempted(&self, needle: &str) -> bool {
        String::from_utf8_lossy(&self.inner.state.lock().unwrap().attempts).contains(needle)
    }

    /// Wait until a write is being held open, so the test can act mid-render.
    fn wait_blocked(&self) {
        let mut state = self.inner.state.lock().unwrap();
        while !state.blocked {
            let (guard, timeout) = self.inner.cond.wait_timeout(state, WAIT_LIMIT).unwrap();
            state = guard;
            if timeout.timed_out() && !state.blocked {
                panic!("timed out waiting for the writer to block");
            }
        }
    }

    /// Let the held write complete.
    fn release_block(&self) {
        self.inner.state.lock().unwrap().block_released = true;
        self.inner.cond.notify_all();
    }
}

impl Write for TestWriter {
    /// Record the write, failing or holding it when it completes the armed needle.
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let mut state = self.inner.state.lock().unwrap();
        state.attempts.extend_from_slice(buf);
        if let Some(needle) = state.fail_needle.clone()
            && !state.failed
        {
            let mut probe = state.text.clone();
            probe.extend_from_slice(buf);
            if String::from_utf8_lossy(&probe).contains(&needle) {
                state.failed = true;
                return Err(io::Error::other("scripted write failure"));
            }
        }
        state.text.extend_from_slice(buf);
        if let Some(needle) = state.block_needle.clone()
            && !state.block_released
            && String::from_utf8_lossy(&state.text).contains(&needle)
        {
            state.blocked = true;
            if let Some(hook) = state.block_hook.take() {
                hook();
            }
            self.inner.cond.notify_all();
            while !state.block_released {
                let (guard, timeout) = self.inner.cond.wait_timeout(state, WAIT_LIMIT).unwrap();
                state = guard;
                if timeout.timed_out() && !state.block_released {
                    panic!("writer block timed out");
                }
            }
            state.blocked = false;
        }
        Ok(buf.len())
    }

    /// Always succeeds: writes are recorded as they arrive, so nothing waits to flush.
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Create `count` fake repositories named `<prefix>01`.. under `base`,
/// returning their paths in name order.
fn make_repos(base: &Path, prefix: &str, count: usize) -> Vec<PathBuf> {
    fs::create_dir_all(base).unwrap();
    (1..=count)
        .map(|i| {
            let repo = base.join(format!("{prefix}{i:02}"));
            fs::create_dir_all(&repo).unwrap();
            crate::discover::tests::fake_repo(&repo);
            repo
        })
        .collect()
}

/// Run `update` to completion on this thread and return its outcome.
fn run_update(
    sys: LatchSys,
    cwd: &Path,
    args: &UpdateArgs,
    writer: TestWriter,
) -> io::Result<ExitCode> {
    let mut out = writer;
    let mut app = App {
        sys: Box::new(sys),
        cwd: cwd.to_path_buf(),
        home: None,
        config_file: cwd.join("config.toml"),
        colour: false,
        out: &mut out,
    };
    update(&mut app, args)
}

/// Run `update` on a spawned thread, so the test can drive the latch and the
/// writer while the run is in flight.
fn spawn_update(
    sys: LatchSys,
    cwd: &Path,
    args: UpdateArgs,
    writer: TestWriter,
) -> std::thread::JoinHandle<io::Result<ExitCode>> {
    let cwd = cwd.to_path_buf();
    std::thread::spawn(move || run_update(sys, &cwd, &args, writer))
}

/// Run `exec` on a spawned thread, so the test can drive the latch and the
/// writer while the run is in flight.
fn spawn_exec(
    sys: LatchSys,
    cwd: &Path,
    paths: Vec<String>,
    writer: TestWriter,
) -> std::thread::JoinHandle<io::Result<ExitCode>> {
    let cwd = cwd.to_path_buf();
    std::thread::spawn(move || {
        let mut out = writer;
        let mut app = App {
            sys: Box::new(sys),
            cwd: cwd.clone(),
            home: None,
            config_file: cwd.join("config.toml"),
            colour: false,
            out: &mut out,
        };
        exec(&mut app, &paths, 3, &["ls".to_owned()])
    })
}

/// The paths argument naming one base directory, at the usual search depth
/// (`UpdateArgs::default()` alone would search at depth 0).
fn one_path(base: &Path) -> UpdateArgs {
    UpdateArgs {
        paths: vec![base.display().to_string()],
        depth: 3,
        ..Default::default()
    }
}

/// The panic payload's message, when it carries one as text; empty otherwise.
fn panic_text(payload: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(text) = payload.downcast_ref::<String>() {
        return text.clone();
    }
    if let Some(text) = payload.downcast_ref::<&str>() {
        return (*text).to_owned();
    }
    String::new()
}

/// Guards that two `update` workers are inside their start calls at the same
/// time: the latch holds both, so a sequential run could never start the second.
#[test]
fn update_workers_overlap() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("base");
    make_repos(&base, "r", 2);
    let latch = Latch::new();
    let sys = LatchSys::new(&latch, LatchMode::Update, true);
    let writer = TestWriter::new();
    let handle = spawn_update(sys, dir.path(), one_path(&base), writer.clone());
    latch.wait_for_started(2);
    assert!(latch.finished().is_empty());
    latch.release_all();
    assert_eq!(handle.join().unwrap().unwrap(), ExitCode::SUCCESS);
    let text = writer.text();
    assert!(text.contains("r01") && text.contains("r02"), "{text}");
}

/// Guards the same overlap for `exec`: two workers inside their `exec` calls
/// at once, after the `rev-parse` calls that stay outside the latch.
#[test]
fn exec_workers_overlap() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("base");
    make_repos(&base, "r", 2);
    let latch = Latch::new();
    let sys = LatchSys::new(&latch, LatchMode::Exec, true);
    let writer = TestWriter::new();
    let paths = vec![base.display().to_string()];
    let handle = spawn_exec(sys, dir.path(), paths, writer.clone());
    latch.wait_for_started(2);
    assert!(latch.finished().is_empty());
    latch.release_all();
    assert_eq!(handle.join().unwrap().unwrap(), ExitCode::SUCCESS);
    let text = writer.text();
    assert!(text.contains("r01") && text.contains("r02"), "{text}");
}

/// Guards that an `update` whose later-named repository finishes first renders
/// the same output as a run with no reordering: name order, always.
#[test]
fn update_out_of_order_completion_keeps_the_output() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("base");
    let repos = make_repos(&base, "r", 2);
    let latch = Latch::new();
    let sys = LatchSys::new(&latch, LatchMode::Update, true);
    let writer = TestWriter::new();
    let handle = spawn_update(sys, dir.path(), one_path(&base), writer.clone());
    latch.wait_for_started(2);
    latch.release(&repos[1]);
    latch.wait_for_finished(&repos[1]);
    assert!(!latch.finished().contains(&repos[0]));
    latch.release(&repos[0]);
    assert_eq!(handle.join().unwrap().unwrap(), ExitCode::SUCCESS);

    let control_latch = Latch::new();
    let control_sys = LatchSys::new(&control_latch, LatchMode::Update, false);
    let control_writer = TestWriter::new();
    run_update(
        control_sys,
        dir.path(),
        &one_path(&base),
        control_writer.clone(),
    )
    .unwrap();
    assert_eq!(writer.text(), control_writer.text());
}

/// Guards the same ordering for `exec`: the later-named repository's command
/// finishes first, and its block still prints second.
#[test]
fn exec_out_of_order_completion_keeps_the_output() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("base");
    let repos = make_repos(&base, "r", 2);
    let latch = Latch::new();
    let sys = LatchSys::new(&latch, LatchMode::Exec, true);
    let writer = TestWriter::new();
    let paths = vec![base.display().to_string()];
    let handle = spawn_exec(sys, dir.path(), paths, writer.clone());
    latch.wait_for_started(2);
    latch.release(&repos[1]);
    latch.wait_for_finished(&repos[1]);
    assert!(!latch.finished().contains(&repos[0]));
    latch.release(&repos[0]);
    assert_eq!(handle.join().unwrap().unwrap(), ExitCode::SUCCESS);

    let control_latch = Latch::new();
    let control_sys = LatchSys::new(&control_latch, LatchMode::Exec, false);
    let control_writer = TestWriter::new();
    let mut out = control_writer.clone();
    let mut app = App {
        sys: Box::new(control_sys),
        cwd: dir.path().to_path_buf(),
        home: None,
        config_file: dir.path().join("config.toml"),
        colour: false,
        out: &mut out,
    };
    let paths = vec![base.display().to_string()];
    exec(&mut app, &paths, 3, &["ls".to_owned()]).unwrap();
    assert_eq!(writer.text(), control_writer.text());
    let text = writer.text();
    assert!(
        text.find("r01").unwrap() < text.find("r02").unwrap(),
        "{text}"
    );
}

/// Guards the bound for `update`: eight repositories in flight at most, the
/// ninth waits for the next batch even after one of the eight finishes, and
/// the bound is reached, not just respected.
#[test]
fn update_never_exceeds_the_bound() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("base");
    let repos = make_repos(&base, "r", 10);
    let latch = Latch::new();
    let sys = LatchSys::new(&latch, LatchMode::Update, true);
    let writer = TestWriter::new();
    let handle = spawn_update(sys, dir.path(), one_path(&base), writer.clone());
    latch.wait_for_started(8);
    assert_eq!(latch.started().len(), 8);
    assert_eq!(latch.max_in_flight(), 8);
    latch.release(&repos[0]);
    latch.wait_for_finished(&repos[0]);
    assert!(!latch.started().contains(&repos[8]));
    latch.release_all();
    assert_eq!(handle.join().unwrap().unwrap(), ExitCode::SUCCESS);
    assert_eq!(latch.started().len(), 10);
    assert_eq!(latch.max_in_flight(), 8);
    let text = writer.text();
    for repo in &repos {
        let name = repo.file_name().unwrap().to_string_lossy();
        assert!(text.contains(&*name), "{text}");
    }
}

/// Guards the bound for `exec` the same way: at most eight commands in flight.
#[test]
fn exec_never_exceeds_the_bound() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("base");
    let repos = make_repos(&base, "r", 10);
    let latch = Latch::new();
    let sys = LatchSys::new(&latch, LatchMode::Exec, true);
    let writer = TestWriter::new();
    let paths = vec![base.display().to_string()];
    let handle = spawn_exec(sys, dir.path(), paths, writer.clone());
    latch.wait_for_started(8);
    assert_eq!(latch.started().len(), 8);
    assert_eq!(latch.max_in_flight(), 8);
    latch.release(&repos[0]);
    latch.wait_for_finished(&repos[0]);
    assert!(!latch.started().contains(&repos[8]));
    latch.release_all();
    assert_eq!(handle.join().unwrap().unwrap(), ExitCode::SUCCESS);
    assert_eq!(latch.started().len(), 10);
    assert_eq!(latch.max_in_flight(), 8);
}

/// Guards the batch barrier: the next batch spawns only when the batch before
/// it is fully joined AND rendered. One finish is not enough, and neither is
/// every finish while the last render is still held open.
#[test]
fn the_next_batch_waits_for_join_and_render() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("base");
    let repos = make_repos(&base, "r", 9);
    let latch = Latch::new();
    let sys = LatchSys::new(&latch, LatchMode::Update, true);
    // Hold the first batch's last render (r08's name line) open.
    let writer = TestWriter::blocking_on("r08", || {});
    let handle = spawn_update(sys, dir.path(), one_path(&base), writer.clone());
    latch.wait_for_started(8);
    latch.release(&repos[0]);
    latch.wait_for_finished(&repos[0]);
    assert!(!latch.started().contains(&repos[8]));
    latch.release_all();
    writer.wait_blocked();
    assert!(!latch.started().contains(&repos[8]));
    writer.release_block();
    assert_eq!(handle.join().unwrap().unwrap(), ExitCode::SUCCESS);
    assert!(latch.started().contains(&repos[8]));
}

/// Guards that a path is resolved only after the group before it has rendered:
/// while the first group's render is held open, the writer creates a
/// repository under the second path, and the second path's header counts it.
/// The shared repository is visited twice, once per group. Paths were given,
/// so the config file is never read.
#[test]
fn a_path_is_resolved_after_the_group_before_it_renders() {
    let dir = tempfile::tempdir().unwrap();
    let base1 = dir.path().join("base1");
    let repos = make_repos(&base1, "r", 1);
    let base2 = dir.path().join("base2");
    fs::create_dir_all(&base2).unwrap();
    let target = repos[0].clone();
    let link = base2.join("link");
    let writer = TestWriter::blocking_on("r01", move || {
        std::os::unix::fs::symlink(&target, &link).unwrap();
    });
    let latch = Latch::new();
    let sys = LatchSys::new(&latch, LatchMode::Update, false);
    let args = UpdateArgs {
        paths: vec![base1.display().to_string(), base2.display().to_string()],
        depth: 3,
        ..Default::default()
    };
    let handle = spawn_update(sys, dir.path(), args, writer.clone());
    writer.wait_blocked();
    writer.release_block();
    assert_eq!(handle.join().unwrap().unwrap(), ExitCode::SUCCESS);
    let text = writer.text();
    assert!(
        text.contains(&format!("{} (1 repository)", base2.display())),
        "{text}"
    );
    assert!(text.contains("  link"), "{text}");
    // The same repository, under both spellings: visited once per group.
    let started = latch.started();
    assert_eq!(started.len(), 2);
    assert!(started.contains(&repos[0]));
    assert!(started.contains(&base2.join("link")));
    assert_eq!(latch.reads(), 0);
}

/// Guards the same timing for configured paths, and that the configured
/// entries load exactly once for the whole run.
#[test]
fn configured_paths_load_once_and_resolve_after_each_group() {
    let dir = tempfile::tempdir().unwrap();
    let base1 = dir.path().join("base1");
    let repos = make_repos(&base1, "r", 1);
    let base2 = dir.path().join("base2");
    fs::create_dir_all(&base2).unwrap();
    let target = repos[0].clone();
    let link = base2.join("link");
    let writer = TestWriter::blocking_on("r01", move || {
        std::os::unix::fs::symlink(&target, &link).unwrap();
    });
    let latch = Latch::new();
    let mut sys = LatchSys::new(&latch, LatchMode::Update, false);
    sys.config = format!(
        "repositories = [\"{}\", \"{}\"]\n",
        base1.display(),
        base2.display()
    )
    .into_bytes();
    let args = UpdateArgs {
        depth: 3,
        ..Default::default()
    };
    let handle = spawn_update(sys, dir.path(), args, writer.clone());
    writer.wait_blocked();
    writer.release_block();
    assert_eq!(handle.join().unwrap().unwrap(), ExitCode::SUCCESS);
    let text = writer.text();
    assert!(
        text.contains(&format!("{} (1 repository)", base2.display())),
        "{text}"
    );
    assert!(text.contains("  link"), "{text}");
    assert_eq!(latch.reads(), 1);
}

/// Guards that inputs with no repositories — an empty directory, a comment, a
/// missing path — start no repository call, and the run still reaches the
/// real path after them.
#[test]
fn empty_comment_and_missing_paths_start_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let empty = dir.path().join("empty");
    fs::create_dir_all(&empty).unwrap();
    let base = dir.path().join("base");
    let repos = make_repos(&base, "r", 1);
    let latch = Latch::new();
    let sys = LatchSys::new(&latch, LatchMode::Update, false);
    let writer = TestWriter::new();
    let args = UpdateArgs {
        paths: vec![
            empty.display().to_string(),
            "# note".to_owned(),
            "/nonexistent-9up-test".to_owned(),
            base.display().to_string(),
        ],
        depth: 3,
        ..Default::default()
    };
    let code = run_update(sys, dir.path(), &args, writer.clone()).unwrap();
    assert_eq!(code, ExitCode::SUCCESS);
    assert_eq!(latch.started(), HashSet::from([repos[0].clone()]));
    let text = writer.text();
    assert!(text.contains("(0 repositories)"), "{text}");
    assert!(text.contains("note"), "{text}");
    assert!(text.contains("doesn't exist."), "{text}");
    assert!(text.contains("r01"), "{text}");
}

/// Guards that a failed hint write is the run's error: the hint is written
/// before the helper, so only the abort part of the render-error rule applies.
#[test]
fn a_failed_hint_write_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let latch = Latch::new();
    let mut sys = LatchSys::new(&latch, LatchMode::Update, false);
    sys.config = b"repositories = []\n".to_vec();
    let mut out = FailingWriter;
    let mut app = app_with(Box::new(sys), dir.path(), &mut out);
    let err = update(&mut app, &UpdateArgs::default()).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::BrokenPipe);
}

/// Guards that a failed header write aborts the run before any worker starts.
#[test]
fn a_failed_header_write_starts_no_worker() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("base");
    make_repos(&base, "r", 2);
    let latch = Latch::new();
    let sys = LatchSys::new(&latch, LatchMode::Update, false);
    let mut out = FailingWriter;
    let mut app = app_with(Box::new(sys), dir.path(), &mut out);
    let args = one_path(&base);
    let err = update(&mut app, &args).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::BrokenPipe);
    assert!(latch.started().is_empty());
}

/// Guards that a failed comment write aborts the run: no later path prints.
#[test]
fn a_failed_comment_write_aborts_later_paths() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("base");
    make_repos(&base, "r", 1);
    let latch = Latch::new();
    let sys = LatchSys::new(&latch, LatchMode::Update, false);
    let writer = TestWriter::failing_on("note");
    let args = UpdateArgs {
        paths: vec!["# note".to_owned(), base.display().to_string()],
        depth: 3,
        ..Default::default()
    };
    let err = run_update(sys, dir.path(), &args, writer.clone()).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::Other);
    assert!(latch.started().is_empty());
    assert!(!writer.attempted("(1 repository)"));
}

/// Guards that a failed path-error write aborts the run the same way.
#[test]
fn a_failed_path_error_write_aborts_later_paths() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("base");
    make_repos(&base, "r", 1);
    let latch = Latch::new();
    let sys = LatchSys::new(&latch, LatchMode::Update, false);
    let writer = TestWriter::failing_on("doesn't exist.");
    let args = UpdateArgs {
        paths: vec![
            "/nonexistent-9up-test".to_owned(),
            base.display().to_string(),
        ],
        depth: 3,
        ..Default::default()
    };
    let err = run_update(sys, dir.path(), &args, writer.clone()).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::Other);
    assert!(latch.started().is_empty());
    assert!(!writer.attempted("(1 repository)"));
}

/// Guards the render-error rule: the error is returned, every batch-mate had
/// already finished when the call returned, and the reports at and after the
/// failure do not print.
#[test]
fn a_render_error_finishes_the_batch_and_aborts() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("base");
    let repos = make_repos(&base, "r", 3);
    let latch = Latch::new();
    let sys = LatchSys::new(&latch, LatchMode::Update, false);
    let writer = TestWriter::failing_on("r02");
    let err = run_update(sys, dir.path(), &one_path(&base), writer.clone()).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::Other);
    assert_eq!(latch.finished(), HashSet::from_iter(repos.iter().cloned()));
    let text = writer.text();
    assert!(text.contains("r01"), "{text}");
    assert!(!text.contains("r02"), "{text}");
    assert!(!text.contains("r03"), "{text}");
}

/// Guards that a panic behind a render error is discarded: the write error,
/// first in join order, is the run's outcome.
#[test]
fn a_render_error_discards_a_later_panic() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("base");
    let repos = make_repos(&base, "r", 3);
    let latch = Latch::new();
    let mut sys = LatchSys::new(&latch, LatchMode::Update, false);
    sys.panics.insert(repos[2].clone(), PanicAt::Fetch);
    let writer = TestWriter::failing_on("r02");
    let err = run_update(sys, dir.path(), &one_path(&base), writer.clone()).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::Other);
    let finished = latch.finished();
    assert!(finished.contains(&repos[0]) && finished.contains(&repos[1]));
}

/// Guards that a render error in the first of two batches starts no later
/// batch and no later path.
#[test]
fn a_render_error_in_the_first_batch_stops_later_batches_and_paths() {
    let dir = tempfile::tempdir().unwrap();
    let base1 = dir.path().join("base1");
    let repos = make_repos(&base1, "r", 10);
    let base2 = dir.path().join("base2");
    let others = make_repos(&base2, "s", 1);
    let latch = Latch::new();
    let sys = LatchSys::new(&latch, LatchMode::Update, false);
    let writer = TestWriter::failing_on("r03");
    let args = UpdateArgs {
        paths: vec![base1.display().to_string(), base2.display().to_string()],
        depth: 3,
        ..Default::default()
    };
    let err = run_update(sys, dir.path(), &args, writer.clone()).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::Other);
    let started = latch.started();
    assert_eq!(started.len(), 8);
    assert!(!started.contains(&repos[8]) && !started.contains(&repos[9]));
    assert!(!started.contains(&others[0]));
    assert!(!writer.attempted("(1 repository)"));
    let finished = latch.finished();
    for repo in &repos[..8] {
        assert!(finished.contains(repo));
    }
}

/// Guards the second-batch variant: with the failure in the second of three
/// batches, the first batch's reports stand, the failing batch's mates
/// finish, and no third batch starts.
#[test]
fn a_render_error_in_the_second_batch_keeps_the_first() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("base");
    let repos = make_repos(&base, "r", 17);
    let latch = Latch::new();
    let sys = LatchSys::new(&latch, LatchMode::Update, false);
    let writer = TestWriter::failing_on("r10");
    let err = run_update(sys, dir.path(), &one_path(&base), writer.clone()).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::Other);
    let text = writer.text();
    for repo in &repos[..9] {
        let name = repo.file_name().unwrap().to_string_lossy();
        assert!(text.contains(&*name), "{text}");
    }
    assert!(!text.contains("r10"), "{text}");
    let finished = latch.finished();
    for repo in &repos[8..16] {
        assert!(finished.contains(repo));
    }
    assert!(!latch.started().contains(&repos[16]));
}

/// Guards that a worker panic propagates only after its batch-mates finish,
/// and that only the reports before it in name order printed.
#[test]
fn a_panicking_worker_propagates_after_its_batch_mates_finish() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("base");
    let repos = make_repos(&base, "r", 3);
    let latch = Latch::new();
    let mut sys = LatchSys::new(&latch, LatchMode::Update, false);
    sys.panics.insert(repos[1].clone(), PanicAt::Fetch);
    let writer = TestWriter::new();
    let args = one_path(&base);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run_update(sys, dir.path(), &args, writer.clone())
    }));
    let payload = result.unwrap_err();
    assert!(
        panic_text(&payload).contains(&repos[1].display().to_string()),
        "{}",
        panic_text(&payload)
    );
    let finished = latch.finished();
    assert!(finished.contains(&repos[0]) && finished.contains(&repos[2]));
    let text = writer.text();
    assert!(text.contains("r01"), "{text}");
    assert!(!text.contains("r02"), "{text}");
    assert!(!text.contains("r03"), "{text}");
}

/// Guards the panic-first combined case: a panicking worker ahead of a
/// failing render means the render is never attempted and the panic, not the
/// write error, is the outcome.
#[test]
fn a_panic_ahead_of_a_failing_render_propagates() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("base");
    let repos = make_repos(&base, "r", 2);
    let latch = Latch::new();
    let mut sys = LatchSys::new(&latch, LatchMode::Update, false);
    sys.panics.insert(repos[0].clone(), PanicAt::Fetch);
    let writer = TestWriter::failing_on("r02");
    let args = one_path(&base);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run_update(sys, dir.path(), &args, writer.clone())
    }));
    let payload = result.unwrap_err();
    assert!(
        panic_text(&payload).contains(&repos[0].display().to_string()),
        "{}",
        panic_text(&payload)
    );
    assert!(!writer.attempted("r02"));
    assert!(latch.finished().contains(&repos[1]));
}

/// Guards that of two panicking workers, the first in name (join) order is
/// the one that propagates, over the same printed prefix.
#[test]
fn two_panicking_workers_propagate_the_first_in_name_order() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("base");
    let repos = make_repos(&base, "r", 3);
    let latch = Latch::new();
    let mut sys = LatchSys::new(&latch, LatchMode::Update, false);
    sys.panics.insert(repos[1].clone(), PanicAt::Fetch);
    sys.panics.insert(repos[2].clone(), PanicAt::Fetch);
    let writer = TestWriter::new();
    let args = one_path(&base);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run_update(sys, dir.path(), &args, writer.clone())
    }));
    let payload = result.unwrap_err();
    let text = panic_text(&payload);
    assert!(text.contains(&repos[1].display().to_string()), "{text}");
    assert!(!text.contains(&repos[2].display().to_string()), "{text}");
    let out = writer.text();
    assert!(out.contains("r01"), "{out}");
    assert!(!out.contains("r02") && !out.contains("r03"), "{out}");
}

/// Guards that a panic in the first of two batches starts no later batch and
/// no later path.
#[test]
fn a_panic_in_the_first_batch_stops_later_batches_and_paths() {
    let dir = tempfile::tempdir().unwrap();
    let base1 = dir.path().join("base1");
    let repos = make_repos(&base1, "r", 10);
    let base2 = dir.path().join("base2");
    let others = make_repos(&base2, "s", 1);
    let latch = Latch::new();
    let mut sys = LatchSys::new(&latch, LatchMode::Update, false);
    sys.panics.insert(repos[2].clone(), PanicAt::Fetch);
    let writer = TestWriter::new();
    let args = UpdateArgs {
        paths: vec![base1.display().to_string(), base2.display().to_string()],
        depth: 3,
        ..Default::default()
    };
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run_update(sys, dir.path(), &args, writer.clone())
    }));
    let payload = result.unwrap_err();
    assert!(
        panic_text(&payload).contains(&repos[2].display().to_string()),
        "{}",
        panic_text(&payload)
    );
    let started = latch.started();
    assert_eq!(started.len(), 8);
    assert!(!started.contains(&repos[8]) && !started.contains(&repos[9]));
    assert!(!started.contains(&others[0]));
    assert!(!writer.attempted(&base2.display().to_string()));
    let finished = latch.finished();
    for (index, repo) in repos[..8].iter().enumerate() {
        if index != 2 {
            assert!(finished.contains(repo));
        }
    }
}

/// Guards the second-batch panic variant: the first batch's reports stand,
/// the failing batch's mates finish, and no third batch starts.
#[test]
fn a_panic_in_the_second_batch_keeps_the_first() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("base");
    let repos = make_repos(&base, "r", 17);
    let latch = Latch::new();
    let mut sys = LatchSys::new(&latch, LatchMode::Update, false);
    sys.panics.insert(repos[9].clone(), PanicAt::Fetch);
    let writer = TestWriter::new();
    let args = one_path(&base);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        run_update(sys, dir.path(), &args, writer.clone())
    }));
    let payload = result.unwrap_err();
    assert!(
        panic_text(&payload).contains(&repos[9].display().to_string()),
        "{}",
        panic_text(&payload)
    );
    let text = writer.text();
    for repo in &repos[..8] {
        let name = repo.file_name().unwrap().to_string_lossy();
        assert!(text.contains(&*name), "{text}");
    }
    let finished = latch.finished();
    for (index, repo) in repos[8..16].iter().enumerate() {
        if index != 1 {
            assert!(finished.contains(repo));
        }
    }
    assert!(!latch.started().contains(&repos[16]));
}

/// Guards that a failure in the second of three paths leaves the first path's
/// output standing complete and the third path untouched.
#[test]
fn a_failure_in_the_second_of_three_paths() {
    let dir = tempfile::tempdir().unwrap();
    let base_a = dir.path().join("base-a");
    make_repos(&base_a, "a", 1);
    let base_b = dir.path().join("base-b");
    make_repos(&base_b, "b", 1);
    let base_c = dir.path().join("base-c");
    let repos_c = make_repos(&base_c, "c", 1);
    let latch = Latch::new();
    let sys = LatchSys::new(&latch, LatchMode::Update, false);
    let writer = TestWriter::failing_on("b01");
    let args = UpdateArgs {
        paths: vec![
            base_a.display().to_string(),
            base_b.display().to_string(),
            base_c.display().to_string(),
        ],
        depth: 3,
        ..Default::default()
    };
    let err = run_update(sys, dir.path(), &args, writer.clone()).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::Other);
    let text = writer.text();
    assert!(text.contains("a01"), "{text}");
    assert!(text.contains("origin is up to date."), "{text}");
    assert!(!latch.started().contains(&repos_c[0]));
    assert!(!writer.attempted(&base_c.display().to_string()));
}

/// Guards the render-error rule for `exec`: with the batch latched behind the
/// failing render, the error is returned, every batch-mate finished before
/// the call returned, and no later report prints.
#[test]
fn an_exec_render_error_finishes_the_latched_batch() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("base");
    let repos = make_repos(&base, "r", 3);
    let latch = Latch::new();
    let sys = LatchSys::new(&latch, LatchMode::Exec, true);
    let writer = TestWriter::failing_on("r02");
    let paths = vec![base.display().to_string()];
    let handle = spawn_exec(sys, dir.path(), paths, writer.clone());
    latch.wait_for_started(3);
    latch.release_all();
    let err = handle.join().unwrap().unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::Other);
    assert_eq!(latch.finished(), HashSet::from_iter(repos.iter().cloned()));
    let text = writer.text();
    assert!(text.contains("r01"), "{text}");
    assert!(!text.contains("r02"), "{text}");
    assert!(!text.contains("r03"), "{text}");
}
