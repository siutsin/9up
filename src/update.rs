use std::path::{Path, PathBuf};

use crate::sys::Sys;

/// A Report carries data only, so milestone 2 can collect reports from
/// many threads and main can render them in path order.
pub struct Report {
    pub name: String,
    pub steps: Vec<Step>,
}

pub enum Step {
    Fetch {
        remote: String,
        outcome: FetchOutcome,
    },
    Branch {
        name: String,
        outcome: BranchOutcome,
    },
    Error(String),
}

pub enum BranchOutcome {
    Updated { from: String, to: String },
    UpToDate,
    Skipped(String),
    Error(String),
}

pub enum FetchOutcome {
    Changed(Vec<(Kind, Vec<String>)>),
    UpToDate,
    NoRefspec,
    Error(Vec<String>),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    NewBranch,
    NewTag,
    BranchUpdate,
}

#[derive(Clone, Copy)]
pub struct Options {
    pub current_only: bool,
    pub fetch_only: bool,
    pub prune: bool,
}

fn git_text(sys: &dyn Sys, dir: &Path, argv: &[&str]) -> Option<String> {
    let output = sys.git(dir, argv).ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

pub struct RepoInfo {
    pub work_dir: PathBuf,
    pub bare: bool,
}

/// Let git answer where the repository lives. A bare repository has no
/// top level, so its working directory is the git directory.
pub fn repo_info(sys: &dyn Sys, path: &Path) -> Option<RepoInfo> {
    let git_dir = PathBuf::from(git_text(sys, path, &["rev-parse", "--absolute-git-dir"])?);
    let top = git_text(sys, path, &["rev-parse", "--show-toplevel"]);
    let bare = top.is_none();
    let work_dir = top.map(PathBuf::from).unwrap_or_else(|| git_dir);
    Some(RepoInfo { work_dir, bare })
}

struct Remote {
    name: String,
    refspecs: Vec<String>,
}

/// The remotes in configuration order, with their fetch refspecs, from
/// one `git config` call.
fn remotes(sys: &dyn Sys, path: &Path) -> Vec<Remote> {
    let Some(text) = git_text(sys, path, &["config", "--get-regexp", "^remote\\."]) else {
        return Vec::new();
    };
    parse_remotes(&text)
}

fn parse_remotes(text: &str) -> Vec<Remote> {
    let mut remotes: Vec<Remote> = Vec::new();
    for line in text.lines() {
        let Some((key, value)) = line.split_once(' ') else {
            continue;
        };
        let Some(rest) = key.strip_prefix("remote.") else {
            continue;
        };
        let Some((name, field)) = rest.rsplit_once('.') else {
            continue;
        };
        let index = match remotes.iter().position(|remote| remote.name == name) {
            Some(index) => index,
            None => {
                remotes.push(Remote {
                    name: name.to_owned(),
                    refspecs: Vec::new(),
                });
                remotes.len() - 1
            }
        };
        if field == "fetch" {
            remotes[index].refspecs.push(value.to_owned());
        }
    }
    remotes
}

/// The current branch's full name without the `refs/heads/` prefix.
/// None means a detached HEAD. Never `--short`: a tag with the same
/// name would change the answer.
fn current_branch(sys: &dyn Sys, path: &Path) -> Option<String> {
    let full = git_text(sys, path, &["symbolic-ref", "-q", "HEAD"])?;
    full.strip_prefix("refs/heads/").map(str::to_owned)
}

/// The remote the branch tracks, from its configuration. Both the
/// remote and the merge ref must be set. Configuration works on an
/// unborn branch, where no refs exist yet.
fn tracked_remote(sys: &dyn Sys, path: &Path, branch: &str) -> Option<String> {
    let remote = git_text(
        sys,
        path,
        &["config", "--get", &format!("branch.{branch}.remote")],
    )?;
    git_text(
        sys,
        path,
        &["config", "--get", &format!("branch.{branch}.merge")],
    )?;
    Some(remote)
}

/// A report that holds one repository error and nothing else.
fn failed(name: &str, message: &str) -> Report {
    Report {
        name: name.to_owned(),
        steps: vec![Step::Error(message.to_owned())],
    }
}

pub fn update_repo(sys: &dyn Sys, path: &Path, name: &str, options: &Options) -> Report {
    let mut report = Report {
        name: name.to_owned(),
        steps: Vec::new(),
    };
    let all_remotes = remotes(sys, path);
    let picked: Vec<&Remote> = if options.current_only {
        let Some(branch) = current_branch(sys, path) else {
            return failed(name, "HEAD is detached, so no remote is tracked.");
        };
        let Some(remote_name) = tracked_remote(sys, path, &branch) else {
            return failed(name, "The current branch tracks no remote.");
        };
        match all_remotes.iter().find(|remote| remote.name == remote_name) {
            Some(remote) => vec![remote],
            None => {
                if all_remotes.is_empty() {
                    return failed(name, "No remotes.");
                }
                // Not in the configuration at all: same outcome as a
                // refspec-less remote, the fetch skip line. Not a
                // repository error, so the branch step still runs.
                report.steps.push(Step::Fetch {
                    remote: remote_name,
                    outcome: FetchOutcome::NoRefspec,
                });
                Vec::new()
            }
        }
    } else {
        if all_remotes.is_empty() {
            return failed(name, "No remotes.");
        }
        all_remotes.iter().collect()
    };
    for remote in picked {
        if remote.refspecs.is_empty() {
            report.steps.push(Step::Fetch {
                remote: remote.name.clone(),
                outcome: FetchOutcome::NoRefspec,
            });
            continue;
        }
        let outcome = fetch(sys, path, &remote.name, options.prune);
        let failed = matches!(outcome, FetchOutcome::Error(_));
        report.steps.push(Step::Fetch {
            remote: remote.name.clone(),
            outcome,
        });
        if failed {
            break;
        }
    }
    if !options.fetch_only {
        let bare = repo_info(sys, path).map(|info| info.bare).unwrap_or(false);
        branch_step(
            sys,
            path,
            &current_branch(sys, path),
            bare,
            &mut report.steps,
        );
    }
    report
}

/// One branch from the snapshot: its name, its SHA, and the full
/// reference of its upstream (empty when it has none).
struct Branch {
    name: String,
    sha: String,
    upstream: String,
}

fn branch_step(
    sys: &dyn Sys,
    path: &Path,
    current: &Option<String>,
    bare: bool,
    steps: &mut Vec<Step>,
) {
    let Some(text) = git_text(
        sys,
        path,
        &[
            "for-each-ref",
            "--format=%(refname)%00%(objectname)%00%(upstream)",
            "refs/heads",
            "refs/remotes",
        ],
    ) else {
        return;
    };
    let mut branches: Vec<Branch> = Vec::new();
    let mut shas: Vec<(String, String)> = Vec::new();
    for line in text.lines() {
        let mut parts = line.split('\0');
        let (Some(reference), Some(sha), upstream) = (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        shas.push((reference.to_owned(), sha.to_owned()));
        if let Some(name) = reference.strip_prefix("refs/heads/") {
            branches.push(Branch {
                name: name.to_owned(),
                sha: sha.to_owned(),
                upstream: upstream.unwrap_or("").to_owned(),
            });
        }
    }
    for index in 0..branches.len() {
        let branch = &branches[index];
        if branch.upstream.is_empty() {
            continue; // No upstream: skipped silently.
        }
        let Some(upstream_sha) = shas
            .iter()
            .find(|(reference, _)| *reference == branch.upstream)
            .map(|(_, sha)| sha.clone())
        else {
            continue; // The upstream reference is missing locally.
        };
        let Some(base) = git_text(sys, path, &["merge-base", &branch.sha, &upstream_sha]) else {
            continue; // No merge base: skipped silently.
        };
        let name = branch.name.clone();
        let upstream_ref = branch.upstream.clone();
        if base == upstream_sha {
            steps.push(Step::Branch {
                name,
                outcome: BranchOutcome::UpToDate,
            });
        } else if base != branch.sha {
            steps.push(Step::Branch {
                name,
                outcome: BranchOutcome::Skipped("a fast-forward is not possible".to_owned()),
            });
        } else if !bare && current.as_deref() == Some(name.as_str()) {
            match sys.git(path, &["merge", "--ff-only", &upstream_ref]) {
                Ok(output) if output.status.success() => {
                    push_updated(steps, &branches, &mut shas, index, &upstream_sha);
                }
                Ok(output) => {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    let reason = if stderr.contains("local changes")
                        && stderr.contains("would be overwritten")
                    {
                        "uncommitted changes"
                    } else {
                        "merge failed"
                    };
                    steps.push(Step::Branch {
                        name,
                        outcome: BranchOutcome::Skipped(reason.to_owned()),
                    });
                }
                Err(err) => steps.push(Step::Branch {
                    name,
                    outcome: BranchOutcome::Skipped(format!("Could not run git: {err}")),
                }),
            }
        } else {
            match sys.git(path, &["branch", "-f", "--", &name, &upstream_ref]) {
                Ok(output) if output.status.success() => {
                    push_updated(steps, &branches, &mut shas, index, &upstream_sha);
                }
                Ok(output) => {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    let line = stderr
                        .lines()
                        .map(str::trim)
                        .find(|line| line.starts_with("error:") || line.starts_with("fatal:"))
                        .map(str::to_owned)
                        .unwrap_or_else(|| {
                            format!("git branch -f {name} failed with {}.", output.status)
                        });
                    steps.push(Step::Branch {
                        name,
                        outcome: BranchOutcome::Error(line),
                    });
                }
                Err(err) => steps.push(Step::Branch {
                    name,
                    outcome: BranchOutcome::Error(format!("Could not run git: {err}")),
                }),
            }
        }
    }
}

fn push_updated(
    steps: &mut Vec<Step>,
    branches: &[Branch],
    shas: &mut [(String, String)],
    index: usize,
    upstream_sha: &str,
) {
    let branch = &branches[index];
    // A later branch may track this one: its snapshot SHA moves too.
    let reference = format!("refs/heads/{}", branch.name);
    if let Some((_, sha)) = shas.iter_mut().find(|(r, _)| *r == reference) {
        *sha = upstream_sha.to_owned();
    }
    steps.push(Step::Branch {
        name: branch.name.clone(),
        outcome: BranchOutcome::Updated {
            from: branch.sha.clone(),
            to: upstream_sha.to_owned(),
        },
    });
}

fn fetch(sys: &dyn Sys, path: &Path, remote: &str, prune: bool) -> FetchOutcome {
    let mut argv = vec!["fetch", "--porcelain"];
    if prune {
        argv.push("--prune");
    }
    argv.push("--");
    argv.push(remote);
    let output = match sys.git(path, &argv) {
        Ok(output) => output,
        Err(err) => return FetchOutcome::Error(vec![format!("Could not run git: {err}")]),
    };
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let mut lines: Vec<String> = stderr
            .lines()
            .map(str::trim)
            .filter(|line| line.starts_with("error:") || line.starts_with("fatal:"))
            .map(str::to_owned)
            .collect();
        if lines.is_empty() {
            lines.push(format!("git fetch {remote} failed with {}.", output.status));
        }
        return FetchOutcome::Error(lines);
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let changed = parse_fetch_output(&stdout, remote);
    if changed.is_empty() {
        FetchOutcome::UpToDate
    } else {
        FetchOutcome::Changed(changed)
    }
}

/// The changed references in fetch porcelain output, grouped by kind.
/// A `*` flag is a new reference, a line with an old and a new value is
/// an update, and every other flag is not reported.
fn parse_fetch_output(text: &str, remote: &str) -> Vec<(Kind, Vec<String>)> {
    let remote_prefix = format!("refs/remotes/{remote}/");
    let mut groups: Vec<(Kind, Vec<String>)> = Vec::new();
    for line in text.lines() {
        // The line is <flag><space><old> <new> <ref>. The flag of an
        // update line is itself a space, so the fields start after the
        // first two characters.
        let mut chars = line.chars();
        let Some(flag) = chars.next() else { continue };
        let rest = chars.as_str().strip_prefix(' ').unwrap_or(chars.as_str());
        let Some(reference) = rest.split(' ').nth(2) else {
            continue;
        };
        let kind = match flag {
            '*' if reference.starts_with("refs/tags/") => Kind::NewTag,
            '*' => Kind::NewBranch,
            ' ' => Kind::BranchUpdate,
            _ => continue,
        };
        let name = if let Some(short) = reference.strip_prefix(&remote_prefix) {
            short.to_owned()
        } else {
            // Drop the first two parts: refs/tags/, refs/heads/, and so on.
            reference
                .splitn(3, '/')
                .nth(2)
                .unwrap_or(reference)
                .to_owned()
        };
        match groups
            .iter_mut()
            .find(|(group_kind, _)| *group_kind == kind)
        {
            Some((_, names)) => names.push(name),
            None => groups.push((kind, vec![name])),
        }
    }
    groups
}

/// The result of running the exec command in one repository: its
/// standard output and standard error as raw text, or the error line
/// when the command could not start. main splits the text into lines.
pub struct ExecReport {
    pub name: String,
    pub stdout: String,
    pub stderr: String,
    pub error: Option<String>,
}

/// Run the command without a shell in the repository's working
/// directory (the repository directory itself for a bare repository).
/// The exit status is ignored.
pub fn exec_repo(sys: &dyn Sys, path: &Path, name: &str, command: &[String]) -> ExecReport {
    let blank = |error: Option<String>| ExecReport {
        name: name.to_owned(),
        stdout: String::new(),
        stderr: String::new(),
        error,
    };
    let Some(info) = repo_info(sys, path) else {
        return blank(Some("Could not read the repository.".to_owned()));
    };
    match sys.exec(&info.work_dir, command) {
        Ok(output) => ExecReport {
            name: name.to_owned(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            error: None,
        },
        Err(err) => blank(Some(format!("Could not run {}: {err}", command[0]))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sys::Sys;
    use std::io;
    use std::os::unix::process::ExitStatusExt;
    use std::process::{ExitStatus, Output};

    fn output(raw_status: i32, stdout: &str, stderr: &str) -> Output {
        Output {
            status: ExitStatus::from_raw(raw_status),
            stdout: stdout.as_bytes().to_vec(),
            stderr: stderr.as_bytes().to_vec(),
        }
    }

    /// A system whose git answers just enough for the fetch and exec
    /// flows, with the fetch scripted per test.
    struct StubSys {
        fetch: FetchStub,
        prune_seen: std::cell::Cell<bool>,
    }

    impl StubSys {
        fn new(fetch: FetchStub) -> Self {
            Self {
                fetch,
                prune_seen: std::cell::Cell::new(false),
            }
        }
    }

    enum FetchStub {
        CannotStart,
        Fails,
        FailsSilently,
        Succeeds,
    }

    impl Sys for StubSys {
        fn git(&self, _dir: &Path, argv: &[&str]) -> io::Result<Output> {
            match argv {
                ["config", "--get-regexp", ..] => Ok(output(
                    0,
                    "remote.origin.url /srv/git.git\nremote.origin.fetch +refs/heads/*:refs/remotes/origin/*\n",
                    "",
                )),
                ["symbolic-ref", "-q", "HEAD"] => Ok(output(0, "refs/heads/main\n", "")),
                ["config", "--get", "branch.main.remote"] => Ok(output(0, "origin\n", "")),
                ["config", "--get", "branch.main.merge"] => Ok(output(0, "refs/heads/main\n", "")),
                ["rev-parse", "--absolute-git-dir"] => Ok(output(0, "/repo/.git\n", "")),
                ["rev-parse", "--show-toplevel"] => Ok(output(0, "/repo\n", "")),
                ["fetch", ..] => {
                    if argv.contains(&"--prune") {
                        self.prune_seen.set(true);
                    }
                    match self.fetch {
                        FetchStub::CannotStart => {
                            Err(io::Error::new(io::ErrorKind::NotFound, "no git here"))
                        }
                        FetchStub::Fails => Ok(output(256, "", "fatal: boom\n")),
                        FetchStub::FailsSilently => Ok(output(256, "", "")),
                        FetchStub::Succeeds => Ok(output(0, "", "")),
                    }
                }
                _ => Ok(output(0, "", "")),
            }
        }

        fn exec(&self, _dir: &Path, _argv: &[String]) -> io::Result<Output> {
            Err(io::Error::new(io::ErrorKind::NotFound, "no such file"))
        }
    }

    #[test]
    fn a_fetch_that_cannot_start_is_an_error_step() {
        let sys = StubSys::new(FetchStub::CannotStart);
        let options = Options {
            current_only: false,
            fetch_only: true,
            prune: false,
        };
        let report = update_repo(&sys, Path::new("/repo"), "repo", &options);
        match &report.steps[..] {
            [
                Step::Fetch {
                    outcome: FetchOutcome::Error(lines),
                    ..
                },
            ] => assert!(lines[0].contains("Could not run git"), "{lines:?}"),
            steps => panic!("unexpected steps: {}", steps.len()),
        }
    }

    #[test]
    fn a_failed_fetch_reports_gits_error_lines() {
        let sys = StubSys::new(FetchStub::Fails);
        let options = Options {
            current_only: false,
            fetch_only: true,
            prune: false,
        };
        let report = update_repo(&sys, Path::new("/repo"), "repo", &options);
        match &report.steps[..] {
            [
                Step::Fetch {
                    outcome: FetchOutcome::Error(lines),
                    ..
                },
            ] => assert_eq!(lines, &["fatal: boom"]),
            steps => panic!("unexpected steps: {}", steps.len()),
        }
    }

    #[test]
    fn a_command_that_cannot_start_is_an_error_report() {
        let sys = StubSys::new(FetchStub::Fails);
        let report = exec_repo(&sys, Path::new("/repo"), "repo", &["frobnicate".to_owned()]);
        assert_eq!(
            report.error.as_deref(),
            Some("Could not run frobnicate: no such file")
        );
    }

    #[test]
    fn current_only_fetches_the_tracked_remote_and_prune_reaches_fetch() {
        let sys = StubSys::new(FetchStub::Succeeds);
        let options = Options {
            current_only: true,
            fetch_only: true,
            prune: true,
        };
        let report = update_repo(&sys, Path::new("/repo"), "repo", &options);
        match &report.steps[..] {
            [
                Step::Fetch {
                    remote,
                    outcome: FetchOutcome::UpToDate,
                },
            ] => assert_eq!(remote, "origin"),
            steps => panic!("unexpected steps: {}", steps.len()),
        }
        assert!(sys.prune_seen.get(), "fetch argv must carry --prune");
    }

    #[test]
    fn without_prune_fetch_has_no_prune_flag() {
        let sys = StubSys::new(FetchStub::Succeeds);
        let options = Options {
            current_only: false,
            fetch_only: true,
            prune: false,
        };
        let _ = update_repo(&sys, Path::new("/repo"), "repo", &options);
        assert!(!sys.prune_seen.get());
    }

    #[test]
    fn parse_remotes_keeps_order_and_refspecs() {
        let text = "remote.origin.url /srv/git.git\nremote.origin.fetch +refs/heads/*:refs/remotes/origin/*\nremote.upstream.url /srv/up.git\nremote.mirror.fetch +refs/*:refs/*\nremote.mirror.url /srv/mirror.git\n";
        let remotes = parse_remotes(text);
        assert_eq!(remotes.len(), 3);
        assert_eq!(remotes[0].name, "origin");
        assert_eq!(remotes[0].refspecs.len(), 1);
        assert!(remotes[1].refspecs.is_empty());
        assert_eq!(remotes[2].name, "mirror");
        assert_eq!(remotes[2].refspecs.len(), 1);
    }

    #[test]
    fn a_silent_fetch_failure_gets_the_fallback_line() {
        let sys = StubSys::new(FetchStub::FailsSilently);
        let options = Options {
            current_only: false,
            fetch_only: true,
            prune: false,
        };
        let report = update_repo(&sys, Path::new("/repo"), "repo", &options);
        match &report.steps[..] {
            [
                Step::Fetch {
                    outcome: FetchOutcome::Error(lines),
                    ..
                },
            ] => assert!(lines[0].contains("failed with"), "{lines:?}"),
            steps => panic!("unexpected steps: {}", steps.len()),
        }
    }

    /// A system that scripts the branch config answers, for the
    /// current-only flow.
    struct FlowSys {
        regexp: &'static str,
        branch_remote: Option<&'static str>,
        branch_merge: Option<&'static str>,
    }

    impl Sys for FlowSys {
        fn git(&self, _dir: &Path, argv: &[&str]) -> io::Result<Output> {
            match argv {
                ["config", "--get-regexp", ..] => Ok(output(0, self.regexp, "")),
                ["symbolic-ref", "-q", "HEAD"] => Ok(output(0, "refs/heads/main\n", "")),
                ["config", "--get", "branch.main.remote"] => match self.branch_remote {
                    Some(value) => Ok(output(0, value, "")),
                    None => Ok(output(1, "", "")),
                },
                ["config", "--get", "branch.main.merge"] => match self.branch_merge {
                    Some(value) => Ok(output(0, value, "")),
                    None => Ok(output(1, "", "")),
                },
                _ => Ok(output(0, "", "")),
            }
        }
    }

    fn current_only_report(sys: &FlowSys) -> Report {
        let options = Options {
            current_only: true,
            fetch_only: true,
            prune: false,
        };
        update_repo(sys, Path::new("/repo"), "repo", &options)
    }

    fn assert_error(report: &Report, message: &str) {
        match &report.steps[..] {
            [Step::Error(text)] => assert_eq!(text, message),
            steps => panic!("unexpected steps: {}", steps.len()),
        }
    }

    #[test]
    fn current_only_without_a_tracked_remote_is_an_error() {
        let no_remote = FlowSys {
            regexp: "remote.origin.url /srv/git.git\n",
            branch_remote: None,
            branch_merge: Some("refs/heads/main\n"),
        };
        assert_error(
            &current_only_report(&no_remote),
            "The current branch tracks no remote.",
        );
        let no_merge = FlowSys {
            branch_remote: Some("origin\n"),
            branch_merge: None,
            ..no_remote
        };
        assert_error(
            &current_only_report(&no_merge),
            "The current branch tracks no remote.",
        );
    }

    #[test]
    fn current_only_with_no_remotes_at_all_is_an_error() {
        let sys = FlowSys {
            regexp: "",
            branch_remote: Some("origin\n"),
            branch_merge: Some("refs/heads/main\n"),
        };
        assert_error(&current_only_report(&sys), "No remotes.");
    }

    #[test]
    fn current_only_with_an_unconfigured_tracked_remote_skips_the_fetch() {
        let sys = FlowSys {
            regexp: "remote.other.url /srv/other.git\nremote.other.fetch +refs/heads/*:refs/remotes/other/*\n",
            branch_remote: Some("origin\n"),
            branch_merge: Some("refs/heads/main\n"),
        };
        let options = Options {
            current_only: true,
            fetch_only: false,
            prune: false,
        };
        let report = update_repo(&sys, Path::new("/repo"), "repo", &options);
        match &report.steps[..] {
            [
                Step::Fetch {
                    remote,
                    outcome: FetchOutcome::NoRefspec,
                },
            ] => assert_eq!(remote, "origin"),
            steps => panic!("unexpected steps: {}", steps.len()),
        }
    }

    /// A system that scripts the ref snapshot and the branch action
    /// (merge for the checked-out branch, branch -f for the rest).
    struct BranchSys {
        current: &'static str,
        refs: &'static str,
        refs_fail: bool,
        action: ActionStub,
    }

    enum ActionStub {
        FailsSilently,
        CannotStart,
    }

    const BRANCH_REFS: &str = "refs/heads/main\0\
        1111111111111111111111111111111111111111\0refs/remotes/origin/main\n\
        refs/remotes/origin/main\0\
        2222222222222222222222222222222222222222\0\n";

    impl Sys for BranchSys {
        fn git(&self, _dir: &Path, argv: &[&str]) -> io::Result<Output> {
            match argv {
                ["config", "--get-regexp", ..] => Ok(output(
                    0,
                    "remote.origin.url /srv/git.git\nremote.origin.fetch +refs/heads/*:refs/remotes/origin/*\n",
                    "",
                )),
                ["symbolic-ref", "-q", "HEAD"] => {
                    Ok(output(0, &format!("refs/heads/{}\n", self.current), ""))
                }
                ["rev-parse", "--absolute-git-dir"] => Ok(output(0, "/repo/.git\n", "")),
                ["rev-parse", "--show-toplevel"] => Ok(output(0, "/repo\n", "")),
                ["for-each-ref", ..] => {
                    if self.refs_fail {
                        Err(io::Error::new(io::ErrorKind::NotFound, "no git here"))
                    } else {
                        Ok(output(0, self.refs, ""))
                    }
                }
                ["merge-base", ..] => {
                    Ok(output(0, "1111111111111111111111111111111111111111\n", ""))
                }
                ["merge", ..] | ["branch", "-f", ..] => match self.action {
                    ActionStub::FailsSilently => Ok(output(256, "", "")),
                    ActionStub::CannotStart => {
                        Err(io::Error::new(io::ErrorKind::NotFound, "no git here"))
                    }
                },
                _ => Ok(output(0, "", "")),
            }
        }
    }

    fn branch_report(sys: &BranchSys) -> Report {
        let options = Options {
            current_only: false,
            fetch_only: false,
            prune: false,
        };
        update_repo(sys, Path::new("/repo"), "repo", &options)
    }

    fn branch_step(report: &Report) -> &Step {
        report
            .steps
            .iter()
            .find(|step| matches!(step, Step::Branch { .. }))
            .expect("a branch step")
    }

    #[test]
    fn a_failed_ref_snapshot_skips_the_branch_step_silently() {
        let sys = BranchSys {
            current: "main",
            refs: "",
            refs_fail: true,
            action: ActionStub::FailsSilently,
        };
        let report = branch_report(&sys);
        assert_eq!(report.steps.len(), 1);
        assert!(matches!(report.steps[0], Step::Fetch { .. }));
    }

    #[test]
    fn a_merge_that_cannot_start_skips_the_branch() {
        let sys = BranchSys {
            current: "main",
            refs: BRANCH_REFS,
            refs_fail: false,
            action: ActionStub::CannotStart,
        };
        match branch_step(&branch_report(&sys)) {
            Step::Branch {
                outcome: BranchOutcome::Skipped(reason),
                ..
            } => assert!(reason.starts_with("Could not run git"), "{reason}"),
            _ => panic!("unexpected step"),
        }
    }

    #[test]
    fn a_failed_merge_skips_the_branch_as_a_merge_failure() {
        let sys = BranchSys {
            current: "main",
            refs: BRANCH_REFS,
            refs_fail: false,
            action: ActionStub::FailsSilently,
        };
        match branch_step(&branch_report(&sys)) {
            Step::Branch {
                outcome: BranchOutcome::Skipped(reason),
                ..
            } => assert_eq!(reason, "merge failed"),
            _ => panic!("unexpected step"),
        }
    }

    #[test]
    fn a_failed_branch_force_update_reports_the_fallback() {
        let sys = BranchSys {
            current: "other",
            refs: BRANCH_REFS,
            refs_fail: false,
            action: ActionStub::FailsSilently,
        };
        match branch_step(&branch_report(&sys)) {
            Step::Branch {
                outcome: BranchOutcome::Error(line),
                ..
            } => assert!(line.contains("failed with"), "{line}"),
            _ => panic!("unexpected step"),
        }
    }

    #[test]
    fn a_branch_force_update_that_cannot_start_is_an_error() {
        let sys = BranchSys {
            current: "other",
            refs: BRANCH_REFS,
            refs_fail: false,
            action: ActionStub::CannotStart,
        };
        match branch_step(&branch_report(&sys)) {
            Step::Branch {
                outcome: BranchOutcome::Error(line),
                ..
            } => assert!(line.starts_with("Could not run git"), "{line}"),
            _ => panic!("unexpected step"),
        }
    }

    /// A system that fails every call, through the trait defaults.
    struct DeadSys;

    impl Sys for DeadSys {}

    #[test]
    fn exec_on_an_unreadable_repository_is_an_error() {
        let report = exec_repo(&DeadSys, Path::new("/repo"), "repo", &["ls".to_owned()]);
        assert_eq!(
            report.error.as_deref(),
            Some("Could not read the repository.")
        );
    }

    #[test]
    fn parse_remotes_skips_lines_it_cannot_use() {
        let text = "no-separator\nbranch.main.remote origin\nremote.bare /srv/x.git\n";
        assert!(parse_remotes(text).is_empty());
    }

    #[test]
    fn parse_fetch_output_classifies_and_names() {
        let text = "  1111111111111111111111111111111111111111 2222222222222222222222222222222222222222 refs/remotes/origin/main\n* 0000000000000000000000000000000000000000 3333333333333333333333333333333333333333 refs/remotes/origin/feat\n* 0000000000000000000000000000000000000000 4444444444444444444444444444444444444444 refs/tags/v1\n+ 1111111111111111111111111111111111111111 2222222222222222222222222222222222222222 refs/remotes/origin/forced\n";
        let groups = parse_fetch_output(text, "origin");
        assert_eq!(
            groups,
            [
                (Kind::BranchUpdate, vec!["main".to_owned()]),
                (Kind::NewBranch, vec!["feat".to_owned()]),
                (Kind::NewTag, vec!["v1".to_owned()]),
            ]
        );
    }
}
