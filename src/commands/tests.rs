use super::*;
use std::fs;
use std::io::Write;
use std::path::Path;

use crate::render::tests::FailingWriter;

/// A system that serves one in-memory config file and answers
/// every git call with empty success.
struct MemorySys {
    config: Vec<u8>,
}

impl crate::sys::Sys for MemorySys {
    fn git(&self, _dir: &Path, _argv: &[&str]) -> io::Result<std::process::Output> {
        Ok(std::process::Output {
            status: std::os::unix::process::ExitStatusExt::from_raw(0),
            stdout: Vec::new(),
            stderr: Vec::new(),
        })
    }

    fn read(&self, _path: &Path) -> io::Result<Vec<u8>> {
        Ok(self.config.clone())
    }
}

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

/// Create a fake repository under `dir` and return the config bytes
/// that name it.
fn repo_config(dir: &Path) -> Vec<u8> {
    let repo = dir.join("repo");
    fs::create_dir_all(&repo).unwrap();
    crate::discover::tests::fake_repo(&repo);
    format!("repositories = [\"{}\"]\n", repo.display()).into_bytes()
}

#[test]
fn update_with_no_configured_repositories_prints_the_hint() {
    let dir = tempfile::tempdir().unwrap();
    let mut out: Vec<u8> = Vec::new();
    let mut app = test_app(Vec::new(), dir.path(), &mut out);
    assert!(update(&mut app, &UpdateArgs::default()).is_ok());
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("No repositories configured."), "{text}");
}

#[test]
fn update_reports_a_failed_hint_write() {
    let dir = tempfile::tempdir().unwrap();
    let mut out = FailingWriter;
    let mut app = test_app(Vec::new(), dir.path(), &mut out);
    assert!(update(&mut app, &UpdateArgs::default()).is_err());
}

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
fn update_reports_a_failed_header_write() {
    let dir = tempfile::tempdir().unwrap();
    let config = repo_config(dir.path());
    let mut out = FailingWriter;
    let mut app = test_app(config, dir.path(), &mut out);
    assert!(update(&mut app, &UpdateArgs::default()).is_err());
}

#[test]
fn exec_prints_a_missing_path_through_the_app() {
    let dir = tempfile::tempdir().unwrap();
    let config = b"repositories = [\"/nonexistent-9up-test\"]\n".to_vec();
    let mut out: Vec<u8> = Vec::new();
    let mut app = test_app(config, dir.path(), &mut out);
    assert!(exec(&mut app, &[], 3, &["true".to_owned()]).is_ok());
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("doesn't exist."), "{text}");
}

#[test]
fn exec_prints_nothing_for_an_empty_comment() {
    let dir = tempfile::tempdir().unwrap();
    let config = b"repositories = [\"#\"]\n".to_vec();
    let mut out: Vec<u8> = Vec::new();
    let mut app = test_app(config, dir.path(), &mut out);
    assert!(exec(&mut app, &[], 3, &["true".to_owned()]).is_ok());
    assert!(out.is_empty(), "{}", String::from_utf8_lossy(&out));
}

#[test]
fn exec_reports_a_failed_write() {
    let dir = tempfile::tempdir().unwrap();
    let config = b"repositories = [\"/nonexistent-9up-test\"]\n".to_vec();
    let mut out = FailingWriter;
    let mut app = test_app(config, dir.path(), &mut out);
    assert!(exec(&mut app, &[], 3, &["true".to_owned()]).is_err());
}

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

/// A system that reads a repository and runs the command, for
/// driving `exec` through the App end to end.
struct RepoExecSys;

impl crate::sys::Sys for RepoExecSys {
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

    fn exec(&self, _dir: &Path, _argv: &[String]) -> io::Result<std::process::Output> {
        Ok(std::process::Output {
            status: std::os::unix::process::ExitStatusExt::from_raw(0),
            stdout: b"hello\n".to_vec(),
            stderr: Vec::new(),
        })
    }
}

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

#[test]
fn exec_reports_an_unparseable_config() {
    let dir = tempfile::tempdir().unwrap();
    let mut out: Vec<u8> = Vec::new();
    let mut app = test_app(b"repositories = [\n".to_vec(), dir.path(), &mut out);
    assert!(exec(&mut app, &[], 3, &["true".to_owned()]).is_err());
}

#[test]
fn add_reports_a_failed_write() {
    let dir = tempfile::tempdir().unwrap();
    let mut out = FailingWriter;
    let mut app = test_app(b"repositories = []\n".to_vec(), dir.path(), &mut out);
    assert!(add(&mut app, &["/x".to_owned()]).is_err());
}

#[test]
fn delete_reports_a_failed_write() {
    let dir = tempfile::tempdir().unwrap();
    let mut out = FailingWriter;
    let mut app = test_app(b"repositories = [\"/x\"]\n".to_vec(), dir.path(), &mut out);
    assert!(delete(&mut app, &["/x".to_owned()]).is_err());
}

#[test]
fn list_reports_a_failed_write() {
    let dir = tempfile::tempdir().unwrap();
    let mut out = FailingWriter;
    let mut app = test_app(b"repositories = [\"/x\"]\n".to_vec(), dir.path(), &mut out);
    assert!(list(&mut app).is_err());
}

#[test]
fn clean_reports_a_failed_write() {
    let dir = tempfile::tempdir().unwrap();
    let config = b"repositories = [\"/nonexistent-9up-test\"]\n".to_vec();
    let mut out = FailingWriter;
    let mut app = test_app(config, dir.path(), &mut out);
    assert!(clean(&mut app).is_err());
}

#[test]
fn exec_reports_a_failed_header_write() {
    let dir = tempfile::tempdir().unwrap();
    let config = repo_config(dir.path());
    let mut out = FailingWriter;
    let mut app = test_app(config, dir.path(), &mut out);
    assert!(exec(&mut app, &[], 3, &["true".to_owned()]).is_err());
}
