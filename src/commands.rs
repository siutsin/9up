use std::io;
use std::process::ExitCode;

use crate::cli::UpdateArgs;
use crate::render::{BOLD_GREEN, RED, paint, print_header, red_line, render_report};
use crate::sys::App;
use crate::{config, discover, update};

pub(crate) fn add(app: &mut App, paths: &[String]) -> io::Result<ExitCode> {
    let out = &mut *app.out;
    let mut entries = config::load(&*app.sys, &app.config_file)?;
    for raw in paths {
        let normalized = config::normalize(raw, &app.cwd);
        if entries.contains(&normalized) {
            writeln!(out, "{normalized} is already in the config file.")?;
        } else {
            writeln!(out, "Added {normalized}.")?;
            entries.push(normalized);
        }
    }
    config::save(&*app.sys, &app.config_file, &entries)?;
    Ok(ExitCode::SUCCESS)
}

pub(crate) fn delete(app: &mut App, paths: &[String]) -> io::Result<ExitCode> {
    let out = &mut *app.out;
    let mut entries = config::load(&*app.sys, &app.config_file)?;
    let mut changed = false;
    for raw in paths {
        let normalized = config::normalize(raw, &app.cwd);
        if let Some(index) = entries.iter().position(|entry| entry == &normalized) {
            entries.remove(index);
            changed = true;
            writeln!(out, "Deleted {normalized}.")?;
        } else {
            writeln!(out, "{normalized} is not in the config file.")?;
        }
    }
    if changed {
        config::save(&*app.sys, &app.config_file, &entries)?;
    }
    Ok(ExitCode::SUCCESS)
}

pub(crate) fn list(app: &mut App) -> io::Result<ExitCode> {
    let out = &mut *app.out;
    let entries = config::load(&*app.sys, &app.config_file)?;
    if entries.is_empty() {
        writeln!(out, "No repositories configured.")?;
    }
    for entry in &entries {
        writeln!(out, "{entry}")?;
    }
    Ok(ExitCode::SUCCESS)
}

pub(crate) fn update(app: &mut App, args: &UpdateArgs) -> io::Result<ExitCode> {
    let inputs: Vec<String> = if args.paths.is_empty() {
        let entries = config::load(&*app.sys, &app.config_file)?;
        if entries.is_empty() {
            writeln!(
                &mut *app.out,
                "No repositories configured. Add one with `9up add <path>`."
            )?;
            return Ok(ExitCode::SUCCESS);
        }
        entries
    } else {
        args.paths.clone()
    };
    let options = update::Options {
        current_only: args.current_only,
        fetch_only: args.fetch_only,
        prune: args.prune,
    };
    for_each_repo(app, &inputs, args.depth, |app, repo| {
        let report = update::update_repo(&*app.sys, &repo.path, &repo.name, &options);
        render_report(&report, app.colour, &mut *app.out)
    })?;
    Ok(ExitCode::SUCCESS)
}

pub(crate) fn clean(app: &mut App) -> io::Result<ExitCode> {
    let out = &mut *app.out;
    let entries = config::load(&*app.sys, &app.config_file)?;
    if entries.is_empty() {
        writeln!(out, "No repositories configured.")?;
        return Ok(ExitCode::SUCCESS);
    }
    let mut kept = Vec::new();
    let mut removed = false;
    for entry in &entries {
        // A comment entry is not a path, so the liveness test below
        // cannot judge it. The plan keeps it.
        if discover::comment_text(entry).is_some() {
            kept.push(entry.clone());
            continue;
        }
        let expanded = discover::expand_home(entry, app.home.as_deref());
        if discover::path_is_live(&expanded) {
            kept.push(entry.clone());
        } else {
            removed = true;
            writeln!(out, "Removed {entry}.")?;
        }
    }
    if removed {
        config::save(&*app.sys, &app.config_file, &kept)?;
    } else {
        writeln!(out, "Nothing to clean.")?;
    }
    Ok(ExitCode::SUCCESS)
}

pub(crate) fn exec(
    app: &mut App,
    paths: &[String],
    depth: i32,
    command: &[String],
) -> io::Result<ExitCode> {
    let inputs: Vec<String> = if paths.is_empty() {
        // No hint here: an exec run with no configured paths prints nothing.
        config::load(&*app.sys, &app.config_file)?
    } else {
        paths.to_vec()
    };
    for_each_repo(app, &inputs, depth, |app, repo| {
        let report = update::exec_repo(&*app.sys, &repo.path, &repo.name, command);
        let out = &mut *app.out;
        writeln!(out, "  {}", paint(app.colour, BOLD_GREEN, &report.name))?;
        if let Some(error) = &report.error {
            red_line(out, app.colour, error)
        } else {
            for line in report.stdout.lines().chain(report.stderr.lines()) {
                writeln!(out, "    {line}")?;
            }
            Ok(())
        }
    })?;
    Ok(ExitCode::SUCCESS)
}

/// The walk `update` and `exec` share: resolve each input in turn, print
/// its group header, and run `op` on every repository found. A comment
/// input prints as a line of its own; a bad path prints in red.
fn for_each_repo(
    app: &mut App,
    inputs: &[String],
    depth: i32,
    mut op: impl FnMut(&mut App, &discover::Repo) -> io::Result<()>,
) -> io::Result<()> {
    for input in inputs {
        match discover::resolve(input, &app.cwd, app.home.as_deref(), depth) {
            discover::Resolved::Repos(repos) => {
                let printed = print_header(
                    &mut *app.out,
                    app.colour,
                    input,
                    app.home.as_deref(),
                    &app.cwd,
                    repos.len(),
                );
                printed?;
                for repo in &repos {
                    op(app, repo)?;
                }
            }
            discover::Resolved::Comment(text) => {
                if !text.is_empty() {
                    writeln!(&mut *app.out, "{text}")?;
                }
            }
            discover::Resolved::PathError(message) => {
                writeln!(&mut *app.out, "{}", paint(app.colour, RED, &message))?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;
    use std::path::Path;

    use crate::render::FailingWriter;

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
    fn update_prints_comments_and_missing_paths() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        fs::create_dir_all(&repo).unwrap();
        crate::discover::fake_repo(&repo);
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
        crate::discover::fake_repo(&repo);
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
}
