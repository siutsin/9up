use super::*;
use std::fs;

fn shim(args: &[&str]) -> Vec<String> {
    insert_update(args.iter().map(OsString::from).collect())
        .iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect()
}

#[test]
fn bare_paths_gain_the_update_command() {
    assert_eq!(shim(&["9up", "a", "b"]), ["9up", "update", "a", "b"]);
    assert_eq!(shim(&["9up", "-c", "a"]), ["9up", "update", "-c", "a"]);
    assert_eq!(
        shim(&["9up", "-t", "-1", "a"]),
        ["9up", "update", "-t", "-1", "a"]
    );
}

#[test]
fn commands_and_flags_pass_through() {
    assert_eq!(shim(&["9up"]), ["9up"]);
    assert_eq!(shim(&["9up", "list"]), ["9up", "list"]);
    assert_eq!(
        shim(&["9up", "-C", "f", "list"]),
        ["9up", "-C", "f", "list"]
    );
    assert_eq!(
        shim(&["9up", "--config", "f", "list"]),
        ["9up", "--config", "f", "list"]
    );
    assert_eq!(shim(&["9up", "--version"]), ["9up", "--version"]);
    assert_eq!(shim(&["9up", "help"]), ["9up", "help"]);
}

#[test]
fn bare_options_without_a_path_mean_update() {
    assert_eq!(shim(&["9up", "-c"]), ["9up", "update", "-c"]);
    assert_eq!(shim(&["9up", "-p", "-f"]), ["9up", "update", "-p", "-f"]);
    assert_eq!(shim(&["9up", "-t3"]), ["9up", "update", "-t3"]);
    assert_eq!(shim(&["9up", "-t-1"]), ["9up", "update", "-t-1"]);
    assert_eq!(shim(&["9up", "-t=3"]), ["9up", "update", "-t=3"]);
    assert_eq!(shim(&["9up", "--depth=2"]), ["9up", "update", "--depth=2"]);
    assert_eq!(
        shim(&["9up", "-C", "f", "-c"]),
        ["9up", "update", "-C", "f", "-c"]
    );
    assert_eq!(
        shim(&["9up", "--config", "f", "-c"]),
        ["9up", "update", "--config", "f", "-c"]
    );
    assert_eq!(shim(&["9up", "-C", "f"]), ["9up", "-C", "f"]);
}

#[test]
fn print_header_shows_the_absolute_path_and_count() {
    let mut out: Vec<u8> = Vec::new();
    print_header(&mut out, false, "base", None, Path::new("/work"), 1).unwrap();
    assert_eq!(
        String::from_utf8(out).unwrap(),
        "/work/base (1 repository)\n"
    );
    let mut out: Vec<u8> = Vec::new();
    print_header(&mut out, false, "/srv/repos", None, Path::new("/work"), 2).unwrap();
    assert_eq!(
        String::from_utf8(out).unwrap(),
        "/srv/repos (2 repositories)\n"
    );
    let mut out: Vec<u8> = Vec::new();
    print_header(
        &mut out,
        false,
        "~/repos",
        Some(Path::new("/home/a")),
        Path::new("/work"),
        0,
    )
    .unwrap();
    assert_eq!(
        String::from_utf8(out).unwrap(),
        "/home/a/repos (0 repositories)\n"
    );
}

#[test]
fn render_report_prints_fetch_and_branch_lines() {
    let report = update::Report {
        name: "repo".to_owned(),
        steps: vec![
            update::Step::Fetch {
                remote: "origin".to_owned(),
                outcome: update::FetchOutcome::Changed(vec![(
                    update::Kind::NewBranch,
                    vec!["feat".to_owned()],
                )]),
            },
            update::Step::Fetch {
                remote: "upstream".to_owned(),
                outcome: update::FetchOutcome::UpToDate,
            },
            update::Step::Branch {
                name: "main".to_owned(),
                outcome: update::BranchOutcome::Updated {
                    from: "1111111111111111111111111111111111111111".to_owned(),
                    to: "2222222222222222222222222222222222222222".to_owned(),
                },
            },
            update::Step::Branch {
                name: "side".to_owned(),
                outcome: update::BranchOutcome::UpToDate,
            },
            update::Step::Branch {
                name: "old".to_owned(),
                outcome: update::BranchOutcome::Skipped(
                    "a fast-forward is not possible".to_owned(),
                ),
            },
            update::Step::Error("No remotes.".to_owned()),
        ],
    };
    let mut out: Vec<u8> = Vec::new();
    render_report(&report, false, &mut out).unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("  repo\n"), "{text}");
    assert!(
        text.contains("Fetched origin: new branches: feat."),
        "{text}"
    );
    assert!(text.contains("upstream is up to date."), "{text}");
    assert!(text.contains("main updated: 1111111..2222222"), "{text}");
    assert!(text.contains("side is up to date."), "{text}");
    assert!(
        text.contains("old skipped: a fast-forward is not possible."),
        "{text}"
    );
    assert!(text.contains("No remotes."), "{text}");
}

#[test]
fn render_report_prints_fetch_skip_and_error_lines() {
    let report = update::Report {
        name: "repo".to_owned(),
        steps: vec![
            update::Step::Fetch {
                remote: "empty".to_owned(),
                outcome: update::FetchOutcome::NoRefspec,
            },
            update::Step::Fetch {
                remote: "origin".to_owned(),
                outcome: update::FetchOutcome::Error(vec!["fatal: boom".to_owned()]),
            },
            update::Step::Branch {
                name: "main".to_owned(),
                outcome: update::BranchOutcome::Error("error: failed".to_owned()),
            },
        ],
    };
    let mut out: Vec<u8> = Vec::new();
    render_report(&report, false, &mut out).unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(
        text.contains("empty has no fetch refspec. Skipped."),
        "{text}"
    );
    assert!(text.contains("fatal: boom"), "{text}");
    assert!(text.contains("main: error: failed"), "{text}");
}

#[test]
fn a_bare_long_option_means_update() {
    assert_eq!(
        shim(&["9up", "--fetch-only"]),
        ["9up", "update", "--fetch-only"]
    );
    assert_eq!(shim(&["9up", "--prune"]), ["9up", "update", "--prune"]);
}

#[test]
fn paint_wraps_the_text_in_the_codes_when_enabled() {
    assert_eq!(paint(true, RED, "x"), format!("{RED}x{RESET}"));
    assert_eq!(paint(false, RED, "x"), "x");
}

struct FailingWriter;

impl Write for FailingWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let _ = buf;
        Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed"))
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// A writer that refuses the fetch-skip line, so a render fails
/// midway rather than on its first line.
struct SkipLineRefusingWriter;

impl Write for SkipLineRefusingWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if String::from_utf8_lossy(buf).contains("refspec") {
            return Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed"));
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn print_header_reports_a_failed_write() {
    let mut out = FailingWriter;
    assert!(print_header(&mut out, false, "/x", None, Path::new("/"), 1).is_err());
}

#[test]
fn the_test_writers_flush_successfully() {
    assert!(FailingWriter.flush().is_ok());
    assert!(SkipLineRefusingWriter.flush().is_ok());
}

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
    crate::discover::fake_repo(&repo);
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
fn exec_reports_a_failed_header_write() {
    let dir = tempfile::tempdir().unwrap();
    let config = repo_config(dir.path());
    let mut out = FailingWriter;
    let mut app = test_app(config, dir.path(), &mut out);
    assert!(exec(&mut app, &[], 3, &["true".to_owned()]).is_err());
}

#[test]
fn render_report_reports_a_failed_write() {
    let report = update::Report {
        name: "repo".to_owned(),
        steps: vec![update::Step::Fetch {
            remote: "empty".to_owned(),
            outcome: update::FetchOutcome::NoRefspec,
        }],
    };
    let mut out = SkipLineRefusingWriter;
    assert!(render_report(&report, false, &mut out).is_err());
}

#[test]
fn a_path_after_the_separator_is_a_path() {
    assert_eq!(
        shim(&["9up", "--", "list"]),
        ["9up", "update", "--", "list"]
    );
    assert_eq!(shim(&["9up", "--", "a"]), ["9up", "update", "--", "a"]);
}
