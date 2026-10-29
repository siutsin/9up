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
        assert!(call(&mut app).is_err(), "{name}");
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
            ["rev-parse", "--absolute-git-dir"] => "/repo/.git\n",
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
