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
            ["merge-base", ..] => Ok(output(0, "1111111111111111111111111111111111111111\n", "")),
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
fn a_malformed_ref_line_is_skipped() {
    let sys = BranchSys {
        current: "main",
        refs: "garbage-line\nrefs/heads/main\0\
            1111111111111111111111111111111111111111\0refs/remotes/origin/main\n\
            refs/remotes/origin/main\0\
            2222222222222222222222222222222222222222\0\n",
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
fn a_full_update_with_a_succeeding_fetch_has_no_branch_steps() {
    let sys = StubSys::new(FetchStub::Succeeds);
    let options = Options {
        current_only: false,
        fetch_only: false,
        prune: false,
    };
    let report = update_repo(&sys, Path::new("/repo"), "repo", &options);
    match &report.steps[..] {
        [
            Step::Fetch {
                outcome: FetchOutcome::UpToDate,
                ..
            },
        ] => {}
        steps => panic!("unexpected steps: {}", steps.len()),
    }
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
fn parse_fetch_output_skips_lines_without_fields() {
    assert!(parse_fetch_output("garbage", "origin").is_empty());
    assert!(parse_fetch_output("", "origin").is_empty());
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
