use super::*;
use std::fs;
use std::io::Write;
use std::path::Path;

use crate::render::tests::FailingWriter;

/// A system that serves one in-memory config file and answers every git call with empty success.
struct MemorySys {
    config: Vec<u8>,
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
}

/// Build an `App` around `MemorySys`, so a test needs only a config and a writer.
fn test_app<'a>(config: Vec<u8>, cwd: &Path, out: &'a mut (dyn Write + 'a)) -> App<'a> {
    App {
        sys: Box::new(MemorySys { config }),
        cwd: cwd.to_path_buf(),
        home: None,
        config_file: cwd.join("config.toml"),
        colour: false,
        out,
    }
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

/// Guards that a failed hint write is an error, so `main` can decide the exit code.
#[test]
fn update_reports_a_failed_hint_write() {
    let dir = tempfile::tempdir().unwrap();
    let mut out = FailingWriter;
    let mut app = test_app(Vec::new(), dir.path(), &mut out);
    assert!(update(&mut app, &UpdateArgs::default()).is_err());
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

/// Guards that a failed header write stops `update` with an error.
#[test]
fn update_reports_a_failed_header_write() {
    let dir = tempfile::tempdir().unwrap();
    let config = repo_config(dir.path());
    let mut out = FailingWriter;
    let mut app = test_app(config, dir.path(), &mut out);
    assert!(update(&mut app, &UpdateArgs::default()).is_err());
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

/// Guards that a failed write in `exec` is an error; a closed pipe stops the run.
#[test]
fn exec_reports_a_failed_write() {
    let dir = tempfile::tempdir().unwrap();
    let config = b"repositories = [\"/nonexistent-9up-test\"]\n".to_vec();
    let mut out = FailingWriter;
    let mut app = test_app(config, dir.path(), &mut out);
    assert!(exec(&mut app, &[], 3, &["true".to_owned()]).is_err());
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

/// Guards that `add` returns its write error, so `main` can decide the exit code.
#[test]
fn add_reports_a_failed_write() {
    let dir = tempfile::tempdir().unwrap();
    let mut out = FailingWriter;
    let mut app = test_app(b"repositories = []\n".to_vec(), dir.path(), &mut out);
    assert!(add(&mut app, &["/x".to_owned()]).is_err());
}

/// Guards that `delete` returns its write error, so `main` can decide the exit code.
#[test]
fn delete_reports_a_failed_write() {
    let dir = tempfile::tempdir().unwrap();
    let mut out = FailingWriter;
    let mut app = test_app(b"repositories = [\"/x\"]\n".to_vec(), dir.path(), &mut out);
    assert!(delete(&mut app, &["/x".to_owned()]).is_err());
}

/// Guards that `list` returns its write error, so `main` can decide the exit code.
#[test]
fn list_reports_a_failed_write() {
    let dir = tempfile::tempdir().unwrap();
    let mut out = FailingWriter;
    let mut app = test_app(b"repositories = [\"/x\"]\n".to_vec(), dir.path(), &mut out);
    assert!(list(&mut app).is_err());
}

/// Guards that `clean` returns its write error, so `main` can decide the exit code.
#[test]
fn clean_reports_a_failed_write() {
    let dir = tempfile::tempdir().unwrap();
    let config = b"repositories = [\"/nonexistent-9up-test\"]\n".to_vec();
    let mut out = FailingWriter;
    let mut app = test_app(config, dir.path(), &mut out);
    assert!(clean(&mut app).is_err());
}

/// Guards that a failed header write stops `exec` before any command runs.
#[test]
fn exec_reports_a_failed_header_write() {
    let dir = tempfile::tempdir().unwrap();
    let config = repo_config(dir.path());
    let mut out = FailingWriter;
    let mut app = test_app(config, dir.path(), &mut out);
    assert!(exec(&mut app, &[], 3, &["true".to_owned()]).is_err());
}
