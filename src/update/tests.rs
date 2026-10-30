use super::*;
use crate::sys::Sys;
use std::io;
use std::os::unix::process::ExitStatusExt;
use std::process::{ExitStatus, Output};

/// Build a canned git result, so each stub answer fits on one line.
fn output(raw_status: i32, stdout: &str, stderr: &str) -> Output {
    Output {
        status: ExitStatus::from_raw(raw_status),
        stdout: stdout.as_bytes().to_vec(),
        stderr: stderr.as_bytes().to_vec(),
    }
}

const ORIGIN_REGEXP: &str = "remote.origin.url /srv/git.git\n\
    remote.origin.fetch +refs/heads/*:refs/remotes/origin/*\n";

/// The git queries most stubs share, scripted once; a stub overrides only the
/// answers its scenario changes and keeps its own arms for the rest.
#[derive(Clone, Copy)]
struct CommonScript {
    regexp: &'static str,
    current: &'static str,
    branch_remote: Option<&'static str>,
    branch_merge: Option<&'static str>,
    refs: &'static str,
}

/// The plain repository most scenarios run against: remote `origin`, branch `main`.
const COMMON: CommonScript = CommonScript {
    regexp: ORIGIN_REGEXP,
    current: "main",
    branch_remote: Some("origin\n"),
    branch_merge: Some("refs/heads/main\n"),
    refs: "",
};

/// A scripted branch config value, or git's exit 1 when the key is unset.
fn config_get(value: Option<&'static str>) -> io::Result<Output> {
    match value {
        Some(value) => Ok(output(0, value, "")),
        None => Ok(output(1, "", "")),
    }
}

/// Answer a shared query from the script; `None` leaves the call to the stub.
fn common_answer(script: &CommonScript, argv: &[&str]) -> Option<io::Result<Output>> {
    let answer = match argv {
        ["config", "--get-regexp", ..] => output(0, script.regexp, ""),
        ["symbolic-ref", "-q", "HEAD"] => {
            output(0, &format!("refs/heads/{}\n", script.current), "")
        }
        ["config", "--get", "branch.main.remote"] => return Some(config_get(script.branch_remote)),
        ["config", "--get", "branch.main.merge"] => return Some(config_get(script.branch_merge)),
        ["rev-parse", "--is-bare-repository"] => output(0, "false\n", ""),
        ["rev-parse", "--show-toplevel"] => output(0, "/repo\n", ""),
        ["for-each-ref", ..] => output(0, script.refs, ""),
        _ => return None,
    };
    Some(Ok(answer))
}

/// A system answering enough git for the fetch and exec flows; fetch is scripted per test.
struct StubSys {
    fetch: FetchStub,
    prune_seen: std::sync::atomic::AtomicBool,
}

impl StubSys {
    /// Start with `prune_seen` unset, so a test can tell whether fetch received `--prune`.
    fn new(fetch: FetchStub) -> Self {
        Self {
            fetch,
            prune_seen: std::sync::atomic::AtomicBool::new(false),
        }
    }
}

enum FetchStub {
    CannotStart,
    Fails,
    FailsSilently,
    Succeeds,
    Changes,
}

impl Sys for StubSys {
    /// Answer the shared queries from `COMMON`, with only the fetch varying per test.
    fn git(&self, _dir: &Path, argv: &[&str]) -> io::Result<Output> {
        if let Some(answer) = common_answer(&COMMON, argv) {
            return answer;
        }
        match argv {
            ["fetch", ..] => {
                if argv.contains(&"--prune") {
                    self.prune_seen
                        .store(true, std::sync::atomic::Ordering::Relaxed);
                }
                match self.fetch {
                    FetchStub::CannotStart => {
                        Err(io::Error::new(io::ErrorKind::NotFound, "no git here"))
                    }
                    FetchStub::Fails => Ok(output(256, "", "fatal: boom\n")),
                    FetchStub::FailsSilently => Ok(output(256, "", "")),
                    FetchStub::Succeeds => Ok(output(0, "", "")),
                    FetchStub::Changes => Ok(output(0, "* 0 0 refs/remotes/origin/feat\n", "")),
                }
            }
            _ => panic!("unexpected git call: {argv:?}"),
        }
    }

    /// Always fail to start, so the exec error path runs.
    fn exec(&self, _dir: &Path, _argv: &[String]) -> io::Result<Output> {
        Err(io::Error::new(io::ErrorKind::NotFound, "no such file"))
    }
}

/// Guards a fetch that cannot start git; it must become an error step, not a lost repository.
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

/// Guards that a failed fetch keeps git's `fatal:` lines; the user sees why it failed.
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

/// Guards a command that cannot start; the report names the command and the cause.
#[test]
fn a_command_that_cannot_start_is_an_error_report() {
    let sys = StubSys::new(FetchStub::Fails);
    let report = exec_repo(&sys, Path::new("/repo"), "repo", &["frobnicate".to_owned()]);
    assert_eq!(
        report.error.as_deref(),
        Some("Could not run frobnicate: no such file")
    );
}

/// Guards `current_only` and `prune`; only the tracked remote is fetched and `--prune` reaches git.
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
    assert!(
        sys.prune_seen.load(std::sync::atomic::Ordering::Relaxed),
        "fetch argv must carry --prune"
    );
}

/// Guards that pruning stays off unless asked; a stray `--prune` would delete remote refs.
#[test]
fn without_prune_fetch_has_no_prune_flag() {
    let sys = StubSys::new(FetchStub::Succeeds);
    let options = Options {
        current_only: false,
        fetch_only: true,
        prune: false,
    };
    let _ = update_repo(&sys, Path::new("/repo"), "repo", &options);
    assert!(!sys.prune_seen.load(std::sync::atomic::Ordering::Relaxed));
}

/// Guards remote parsing; config order and refspecs must survive, including a remote with none.
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

/// Guards a fetch that fails with no stderr; the user still gets a line naming the failure.
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

/// A system scripting the branch config answers for the current-only flow.
struct FlowSys {
    regexp: &'static str,
    branch_remote: Option<&'static str>,
    branch_merge: Option<&'static str>,
}

impl Sys for FlowSys {
    /// Answer every query from the script, failing the branch gets set to `None`.
    fn git(&self, _dir: &Path, argv: &[&str]) -> io::Result<Output> {
        let script = CommonScript {
            regexp: self.regexp,
            branch_remote: self.branch_remote,
            branch_merge: self.branch_merge,
            ..COMMON
        };
        if let Some(answer) = common_answer(&script, argv) {
            return answer;
        }
        panic!("unexpected git call: {argv:?}")
    }
}

/// Share one fetch-only, current-only update, so each test sets up only its stub.
fn current_only_report(sys: &FlowSys) -> Report {
    let options = Options {
        current_only: true,
        fetch_only: true,
        prune: false,
    };
    update_repo(sys, Path::new("/repo"), "repo", &options)
}

/// Check the report holds just one repository error, the shape every early exit takes.
fn assert_error(report: &Report, message: &str) {
    match &report.steps[..] {
        [Step::Error(text)] => assert_eq!(text, message),
        steps => panic!("unexpected steps: {}", steps.len()),
    }
}

/// Guards that both the remote and the merge config are required; either alone tracks nothing.
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

/// Guards a repository with no remotes; it is an error even when the branch names a remote.
#[test]
fn current_only_with_no_remotes_at_all_is_an_error() {
    let sys = FlowSys {
        regexp: "",
        branch_remote: Some("origin\n"),
        branch_merge: Some("refs/heads/main\n"),
    };
    assert_error(&current_only_report(&sys), "No remotes.");
}

/// Guards a tracked remote missing from the config; it is a fetch skip, not a repository error.
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

/// A system scripting the ref snapshot and branch action: `merge` if checked out, else `branch -f`.
struct BranchSys {
    current: &'static str,
    refs: &'static str,
    refs_fail: bool,
    action: ActionStub,
}

enum ActionStub {
    FailsSilently,
    FailsWith(&'static str),
    CannotStart,
}

const BRANCH_REFS: &str = "refs/heads/main\0\
    1111111111111111111111111111111111111111\0refs/remotes/origin/main\n\
    refs/remotes/origin/main\0\
    2222222222222222222222222222222222222222\0\n";

impl Sys for BranchSys {
    /// Serve the scripted snapshot and action; the merge base always allows a fast-forward.
    fn git(&self, _dir: &Path, argv: &[&str]) -> io::Result<Output> {
        if self.refs_fail && matches!(argv, ["for-each-ref", ..]) {
            return Err(io::Error::new(io::ErrorKind::NotFound, "no git here"));
        }
        let script = CommonScript {
            current: self.current,
            refs: self.refs,
            ..COMMON
        };
        if let Some(answer) = common_answer(&script, argv) {
            return answer;
        }
        match argv {
            ["fetch", ..] => Ok(output(0, "", "")),
            ["merge-base", ..] => Ok(output(0, "1111111111111111111111111111111111111111\n", "")),
            ["merge", ..] | ["branch", "-f", ..] => match self.action {
                ActionStub::FailsSilently => Ok(output(256, "", "")),
                ActionStub::FailsWith(stderr) => Ok(output(256, "", stderr)),
                ActionStub::CannotStart => {
                    Err(io::Error::new(io::ErrorKind::NotFound, "no git here"))
                }
            },
            _ => panic!("unexpected git call: {argv:?}"),
        }
    }
}

/// Run a full update so the branch step runs after the fetch.
fn branch_report(sys: &BranchSys) -> Report {
    let options = Options {
        current_only: false,
        fetch_only: false,
        prune: false,
    };
    update_repo(sys, Path::new("/repo"), "repo", &options)
}

/// Pick out the branch step, so tests can ignore the fetch step before it.
fn branch_step(report: &Report) -> &Step {
    report
        .steps
        .iter()
        .find(|step| matches!(step, Step::Branch { .. }))
        .expect("a branch step")
}

/// Guards a malformed ref line; it is skipped and the valid branches are still processed.
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

/// Guards an empty ref snapshot; a successful fetch alone must not invent branch steps.
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

/// Guards a ref snapshot that cannot be read; the branch step is dropped without an error.
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

/// Guards a merge that cannot start git; the current branch is skipped with the cause.
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

/// Guards a merge that fails for no known reason; it is reported as a plain merge failure.
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

/// Guards a `branch -f` that fails with no stderr; the error still names the command and status.
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

/// Guards a `branch -f` that cannot start git; it is a branch error, whereas a merge is skipped.
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

/// Guards exec in a directory git cannot read; the report says so instead of running the command.
#[test]
fn exec_on_an_unreadable_repository_is_an_error() {
    let report = exec_repo(&DeadSys, Path::new("/repo"), "repo", &["ls".to_owned()]);
    assert_eq!(
        report.error.as_deref(),
        Some("Could not read the repository.")
    );
}

/// Guards remote parsing against lines that are not remote fields; they must not create remotes.
#[test]
fn parse_remotes_skips_lines_it_cannot_use() {
    let text = "no-separator\nbranch.main.remote origin\nremote.bare /srv/x.git\n";
    assert!(parse_remotes(text).is_empty());
}

/// Guards fetch parsing against short or empty lines; they must not panic or report changes.
#[test]
fn parse_fetch_output_skips_lines_without_fields() {
    assert!(parse_fetch_output("garbage", "origin").is_empty());
    assert!(parse_fetch_output("", "origin").is_empty());
}

/// Guards that fetched changes reach the report, so the user sees new branches.
#[test]
fn a_fetch_with_changes_reports_the_new_branch() {
    let sys = StubSys::new(FetchStub::Changes);
    let options = Options {
        current_only: false,
        fetch_only: true,
        prune: false,
    };
    let report = update_repo(&sys, Path::new("/repo"), "repo", &options);
    match &report.steps[..] {
        [
            Step::Fetch {
                outcome: FetchOutcome::Changed(changes),
                ..
            },
        ] => assert_eq!(changes, &[(Kind::NewBranch, vec!["feat".to_owned()])]),
        _ => panic!("unexpected steps"),
    }
}

/// A system with a working remote config whose HEAD is detached.
struct DetachedSys;

impl Sys for DetachedSys {
    /// The config call succeeds; `symbolic-ref` fails, as it does on a detached HEAD.
    fn git(&self, _dir: &Path, argv: &[&str]) -> io::Result<Output> {
        match argv {
            ["config", "--get-regexp", ..] => Ok(output(0, ORIGIN_REGEXP, "")),
            ["symbolic-ref", "-q", "HEAD"] => Ok(output(256, "", "")),
            _ => panic!("unexpected git call: {argv:?}"),
        }
    }
}

/// Guards `current_only` on a detached HEAD; with no branch there is no remote to pick.
#[test]
fn a_detached_head_with_current_only_is_a_repository_error() {
    let options = Options {
        current_only: true,
        fetch_only: true,
        prune: false,
    };
    let report = update_repo(&DetachedSys, Path::new("/repo"), "repo", &options);
    match &report.steps[..] {
        [Step::Error(message)] => {
            assert_eq!(message, "HEAD is detached, so no remote is tracked.")
        }
        _ => panic!("unexpected steps"),
    }
}

/// A system whose git cannot start at all.
struct ConfigStartSys;

impl Sys for ConfigStartSys {
    /// The config call is the first one `update_repo` makes, and it cannot start.
    fn git(&self, _dir: &Path, argv: &[&str]) -> io::Result<Output> {
        match argv {
            ["config", "--get-regexp", ..] => {
                Err(io::Error::new(io::ErrorKind::NotFound, "no git here"))
            }
            _ => panic!("unexpected git call: {argv:?}"),
        }
    }
}

/// Guards a git that cannot start on the config call; it is a repository error,
/// not the "No remotes." that a working git with no remotes earns.
#[test]
fn a_config_call_that_cannot_start_is_a_repository_error() {
    let options = Options {
        current_only: false,
        fetch_only: true,
        prune: false,
    };
    let report = update_repo(&ConfigStartSys, Path::new("/repo"), "repo", &options);
    match &report.steps[..] {
        [Step::Error(message)] => {
            assert!(message.starts_with("Could not run git"), "{message}");
            assert_ne!(message, "No remotes.");
        }
        _ => panic!("unexpected steps"),
    }
}

/// A system whose config call exits with a scripted status and stderr.
struct ConfigStatusSys {
    raw_status: i32,
    stderr: &'static str,
}

impl Sys for ConfigStatusSys {
    /// Only the config call is scripted; the update ends on its answer.
    fn git(&self, _dir: &Path, argv: &[&str]) -> io::Result<Output> {
        match argv {
            ["config", "--get-regexp", ..] => Ok(output(self.raw_status, "", self.stderr)),
            _ => panic!("unexpected git call: {argv:?}"),
        }
    }
}

/// Guards the config call's exit contract: exit 1 is git's no-match answer, a
/// hard failure reports git's error line, and a silent one the fallback naming
/// the command and status.
#[test]
fn the_config_calls_exit_status_sets_the_error() {
    for (raw_status, stderr, message) in [
        (256, "", "No remotes."),
        (512, "fatal: bad config\n", "fatal: bad config"),
        (512, "", "git config failed with exit status: 2."),
    ] {
        let sys = ConfigStatusSys { raw_status, stderr };
        let options = Options {
            current_only: false,
            fetch_only: true,
            prune: false,
        };
        let report = update_repo(&sys, Path::new("/repo"), "repo", &options);
        assert_error(&report, message);
    }
}

/// A system whose only remote has a URL but no fetch refspec.
struct RefspecLessSys;

impl Sys for RefspecLessSys {
    /// A remote with a URL but no fetch line is the case the skip line exists for.
    fn git(&self, _dir: &Path, argv: &[&str]) -> io::Result<Output> {
        let script = CommonScript {
            regexp: "remote.empty.url /srv/git.git\n",
            ..COMMON
        };
        if let Some(answer) = common_answer(&script, argv) {
            return answer;
        }
        panic!("unexpected git call: {argv:?}")
    }
}

/// Guards a remote with no refspec; fetch is skipped and reported, since git has nothing to fetch.
#[test]
fn a_remote_without_a_refspec_is_a_fetch_skip() {
    let options = Options {
        current_only: false,
        fetch_only: false,
        prune: false,
    };
    let report = update_repo(&RefspecLessSys, Path::new("/repo"), "repo", &options);
    match &report.steps[..] {
        [
            Step::Fetch {
                remote,
                outcome: FetchOutcome::NoRefspec,
            },
        ] => assert_eq!(remote, "empty"),
        _ => panic!("unexpected steps"),
    }
}

/// One branch per silent-skip arm of the branch step, plus up-to-date, diverged and stuck
/// branches and the current branch fast-forwarding.
struct SkipSys;

const SKIP_REFS: &str = "refs/heads/main\0aa\0refs/remotes/origin/main\n\
    refs/remotes/origin/main\0bb\0\n\
    refs/heads/lonely\0cc\0\n\
    refs/heads/ghost\0dd\0refs/remotes/origin/ghost\n\
    refs/heads/orphan\0ee\0refs/remotes/origin/orphan\n\
    refs/remotes/origin/orphan\0ff\0\n\
    refs/heads/calm\0gg\0refs/remotes/origin/calm\n\
    refs/remotes/origin/calm\0hh\0\n\
    refs/heads/forked\0ii\0refs/remotes/origin/forked\n\
    refs/remotes/origin/forked\0jj\0\n\
    refs/heads/stuck\0kk\0refs/remotes/origin/stuck\n\
    refs/remotes/origin/stuck\0ll\0\n\
    refs/heads/moved\0mm\0refs/remotes/origin/moved\n\
    refs/remotes/origin/moved\0nn\0\n";

impl Sys for SkipSys {
    /// Answer each branch's merge base so every arm of the branch step is reached once.
    fn git(&self, _dir: &Path, argv: &[&str]) -> io::Result<Output> {
        let script = CommonScript {
            refs: SKIP_REFS,
            ..COMMON
        };
        if let Some(answer) = common_answer(&script, argv) {
            return answer;
        }
        match argv {
            ["fetch", ..] => Ok(output(0, "", "")),
            ["merge-base", "aa", "bb"] => Ok(output(0, "aa\n", "")),
            ["merge-base", "ee", "ff"] => Ok(output(256, "", "")),
            ["merge-base", "gg", "hh"] => Ok(output(0, "hh\n", "")),
            ["merge-base", "ii", "jj"] => Ok(output(0, "zz\n", "")),
            ["merge-base", "kk", "ll"] => Ok(output(0, "kk\n", "")),
            ["merge-base", "mm", "nn"] => Ok(output(0, "mm\n", "")),
            ["merge", ..] => Ok(output(0, "", "")),
            ["branch", "-f", "--", "stuck", ..] => Ok(output(256, "", "some plain failure\n")),
            ["branch", "-f", ..] => Ok(output(0, "", "")),
            _ => panic!("unexpected git call: {argv:?}"),
        }
    }
}

/// Find one branch's outcome by name, since step order is not what these tests check.
fn branch_outcome<'a>(report: &'a Report, name: &str) -> &'a BranchOutcome {
    report
        .steps
        .iter()
        .find_map(|step| match step {
            Step::Branch {
                name: found,
                outcome,
            } if found == name => Some(outcome),
            _ => None,
        })
        .expect("a branch step for the name")
}

/// Guards every arm of the branch step in one run; silent skips stay silent and the rest report.
#[test]
fn the_branch_step_skips_and_reports_each_arm() {
    let options = Options {
        current_only: false,
        fetch_only: false,
        prune: false,
    };
    let report = update_repo(&SkipSys, Path::new("/repo"), "repo", &options);
    let branches: Vec<&Step> = report
        .steps
        .iter()
        .filter(|step| matches!(step, Step::Branch { .. }))
        .collect();
    assert_eq!(branches.len(), 5);
    match branch_outcome(&report, "main") {
        BranchOutcome::Updated { from, to } => {
            assert_eq!(from, "aa");
            assert_eq!(to, "bb");
        }
        _ => panic!("unexpected outcome for main"),
    }
    assert!(matches!(
        branch_outcome(&report, "calm"),
        BranchOutcome::UpToDate
    ));
    match branch_outcome(&report, "forked") {
        BranchOutcome::Skipped(reason) => {
            assert_eq!(reason, "a fast-forward is not possible")
        }
        _ => panic!("unexpected outcome for forked"),
    }
    match branch_outcome(&report, "stuck") {
        BranchOutcome::Error(line) => {
            assert_eq!(line, "git branch -f stuck failed with exit status: 1.")
        }
        _ => panic!("unexpected outcome for stuck"),
    }
    match branch_outcome(&report, "moved") {
        BranchOutcome::Updated { from, to } => {
            assert_eq!(from, "mm");
            assert_eq!(to, "nn");
        }
        _ => panic!("unexpected outcome for moved"),
    }
}

/// `adep` tracks the local branch `zzz`, and sorts before it in the snapshot.
const CHAIN_REFS: &str = "refs/heads/adep\0a1\0refs/heads/zzz\n\
    refs/heads/zzz\0z1\0refs/remotes/origin/zzz\n\
    refs/remotes/origin/zzz\0z2\0\n";

/// A system where `zzz` moves to a new SHA and `adep` can follow it there.
struct ChainSys;

impl Sys for ChainSys {
    /// `adep` is up to date against `zzz`'s old SHA but fast-forwards to its new one,
    /// so only processing `zzz` first lets `adep` move.
    fn git(&self, _dir: &Path, argv: &[&str]) -> io::Result<Output> {
        let script = CommonScript {
            current: "other",
            refs: CHAIN_REFS,
            ..COMMON
        };
        if let Some(answer) = common_answer(&script, argv) {
            return answer;
        }
        match argv {
            ["fetch", ..] => Ok(output(0, "", "")),
            ["merge-base", "a1", "z1"] => Ok(output(0, "z1\n", "")),
            ["merge-base", "a1", "z2"] => Ok(output(0, "a1\n", "")),
            ["merge-base", "z1", "z2"] => Ok(output(0, "z1\n", "")),
            ["branch", "-f", ..] => Ok(output(0, "", "")),
            _ => panic!("unexpected git call: {argv:?}"),
        }
    }
}

/// Guards local-branch chains; a follower that sorts first must still see the
/// tracked branch's move, and the steps stay in name order.
#[test]
fn a_branch_tracking_a_local_branch_sees_its_move() {
    let options = Options {
        current_only: false,
        fetch_only: false,
        prune: false,
    };
    let report = update_repo(&ChainSys, Path::new("/repo"), "repo", &options);
    let names: Vec<&str> = report
        .steps
        .iter()
        .filter_map(|step| match step {
            Step::Branch { name, .. } => Some(name.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(names, ["adep", "zzz"]);
    match branch_outcome(&report, "adep") {
        BranchOutcome::Updated { from, to } => {
            assert_eq!(from, "a1");
            assert_eq!(to, "z2");
        }
        _ => panic!("unexpected outcome for adep"),
    }
    match branch_outcome(&report, "zzz") {
        BranchOutcome::Updated { from, to } => {
            assert_eq!(from, "z1");
            assert_eq!(to, "z2");
        }
        _ => panic!("unexpected outcome for zzz"),
    }
}

/// A branch whose name holds the replacement character, as lossy decoding of a
/// non-UTF-8 name produces; its upstream has moved.
const MANGLED_REFS: &str = "refs/heads/bad\u{FFFD}name\0b1\0refs/remotes/origin/main\n\
    refs/remotes/origin/main\0m2\0\n";

/// A system that panics on any mutating call, so a mangled name can never be moved.
struct MangledSys {
    refs: &'static str,
}

impl Sys for MangledSys {
    /// The merge base would allow a fast-forward; only the mangled name blocks the move.
    fn git(&self, _dir: &Path, argv: &[&str]) -> io::Result<Output> {
        let script = CommonScript {
            current: "other",
            refs: self.refs,
            ..COMMON
        };
        if let Some(answer) = common_answer(&script, argv) {
            return answer;
        }
        match argv {
            ["fetch", ..] => Ok(output(0, "", "")),
            ["merge-base", "b1", "m2"] => Ok(output(0, "b1\n", "")),
            ["merge-base", "g1", "g2"] => Ok(output(0, "g1\n", "")),
            ["merge-base", "m2", "m2"] => Ok(output(0, "m2\n", "")),
            ["merge", ..] | ["branch", ..] => {
                panic!("mutating git call for a non-UTF-8 name: {argv:?}")
            }
            _ => panic!("unexpected git call: {argv:?}"),
        }
    }
}

/// Guards a non-UTF-8 branch name; it is an error step, never an update under
/// the mangled name, and git is never asked to move it.
#[test]
fn a_branch_name_that_is_not_valid_utf8_is_an_error() {
    let options = Options {
        current_only: false,
        fetch_only: false,
        prune: false,
    };
    let report = update_repo(
        &MangledSys { refs: MANGLED_REFS },
        Path::new("/repo"),
        "repo",
        &options,
    );
    match branch_outcome(&report, "bad\u{FFFD}name") {
        BranchOutcome::Error(message) => {
            assert_eq!(message, "branch or upstream name is not valid UTF-8")
        }
        _ => panic!("unexpected outcome for the mangled branch"),
    }
}

/// Guards the dirty work tree case; git's wording must map to "uncommitted changes".
#[test]
fn a_merge_failure_with_the_local_changes_wording_reports_uncommitted_changes() {
    let sys = BranchSys {
        current: "main",
        refs: BRANCH_REFS,
        refs_fail: false,
        action: ActionStub::FailsWith(
            "error: Your local changes to the file would be overwritten by merge\n",
        ),
    };
    match branch_step(&branch_report(&sys)) {
        Step::Branch {
            outcome: BranchOutcome::Skipped(reason),
            ..
        } => assert_eq!(reason, "uncommitted changes"),
        _ => panic!("unexpected step"),
    }
}

/// Guards the wording match against false positives; blaming local changes needs both phrases.
#[test]
fn a_merge_failure_mentioning_only_local_changes_is_a_plain_merge_failure() {
    let sys = BranchSys {
        current: "main",
        refs: BRANCH_REFS,
        refs_fail: false,
        action: ActionStub::FailsWith("error: Your local changes would be lost\n"),
    };
    match branch_step(&branch_report(&sys)) {
        Step::Branch {
            outcome: BranchOutcome::Skipped(reason),
            ..
        } => assert_eq!(reason, "merge failed"),
        _ => panic!("unexpected step"),
    }
}

/// A system that reads a repository and runs the command.
struct ExecSys;

impl Sys for ExecSys {
    /// Answer the repository lookups that `exec_repo` makes first.
    fn git(&self, _dir: &Path, argv: &[&str]) -> io::Result<Output> {
        if let Some(answer) = common_answer(&COMMON, argv) {
            return answer;
        }
        panic!("unexpected git call: {argv:?}")
    }

    /// Return fixed output, so the test can check both streams reach the report.
    fn exec(&self, _dir: &Path, _argv: &[String]) -> io::Result<Output> {
        Ok(output(0, "hello\n", "warn\n"))
    }
}

/// A repository whose path bytes are non-UTF-8: git's path answers come back
/// mangled, while its bare answer is plain text, as git prints it.
struct MangledPathSys {
    bare: bool,
}

impl Sys for MangledPathSys {
    fn git(&self, _dir: &Path, argv: &[&str]) -> io::Result<Output> {
        match argv {
            ["rev-parse", "--is-bare-repository"] => {
                Ok(output(0, if self.bare { "true\n" } else { "false\n" }, ""))
            }
            ["rev-parse", "--show-toplevel"] => Ok(output(0, "/repo\u{FFFD}\n", "")),
            _ => panic!("unexpected git call: {argv:?}"),
        }
    }

    /// Echo the directory back, so the test can see where the command ran.
    fn exec(&self, dir: &Path, _argv: &[String]) -> io::Result<Output> {
        Ok(output(0, &dir.display().to_string(), ""))
    }
}

/// Guards a non-UTF-8 repository path; exec refuses it as unreadable instead of
/// running the command in a mangled directory that does not exist.
#[test]
fn exec_on_a_non_utf8_repository_path_is_an_error() {
    let report = exec_repo(
        &MangledPathSys { bare: false },
        Path::new("/repo"),
        "repo",
        &["ls".to_owned()],
    );
    assert_eq!(
        report.error.as_deref(),
        Some("Could not read the repository.")
    );
}

/// Guards a bare non-UTF-8 repository; its work dir is the repository itself,
/// so exec still runs there instead of refusing a mangled path answer.
#[test]
fn exec_in_a_bare_non_utf8_repository_uses_the_repository_directory() {
    let report = exec_repo(
        &MangledPathSys { bare: true },
        Path::new("/repo"),
        "repo",
        &["ls".to_owned()],
    );
    assert_eq!(report.error, None);
    assert_eq!(report.stdout, "/repo");
}

/// The snapshot for the git-directory scenarios: `main` at `aaa`, its upstream at `bbb`.
const GIT_DIR_REFS: &str = "refs/heads/main\0aaa\0refs/remotes/origin/main\n\
    refs/remotes/origin/main\0bbb\0\n";

/// A path that is a git directory: not bare, but it has no top level, because
/// git will not name a work tree from inside the git directory.
struct GitDirSys;

impl Sys for GitDirSys {
    fn git(&self, _dir: &Path, argv: &[&str]) -> io::Result<Output> {
        match argv {
            ["rev-parse", "--is-bare-repository"] => return Ok(output(0, "false\n", "")),
            ["rev-parse", "--show-toplevel"] => {
                return Ok(output(
                    128,
                    "",
                    "fatal: this operation must be run in a work tree\n",
                ));
            }
            _ => {}
        }
        let script = CommonScript {
            refs: GIT_DIR_REFS,
            ..COMMON
        };
        if let Some(answer) = common_answer(&script, argv) {
            return answer;
        }
        match argv {
            ["fetch", ..] => Ok(output(0, "", "")),
            ["merge-base", "aaa", "bbb"] => Ok(output(0, "aaa\n", "")),
            ["branch", "-f", ..] => Ok(output(0, "", "")),
            _ => panic!("unexpected git call: {argv:?}"),
        }
    }

    /// Echo the directory back, so the test can see where the command ran.
    fn exec(&self, dir: &Path, _argv: &[String]) -> io::Result<Output> {
        Ok(output(0, &dir.display().to_string(), ""))
    }
}

/// A system where git answers the bare check but the top-level call cannot start.
struct TopCallFailsSys;

impl Sys for TopCallFailsSys {
    fn git(&self, _dir: &Path, argv: &[&str]) -> io::Result<Output> {
        match argv {
            ["rev-parse", "--is-bare-repository"] => Ok(output(0, "false\n", "")),
            _ => Err(io::Error::new(io::ErrorKind::NotFound, "no git here")),
        }
    }
}

/// Guards a top-level call that cannot run; exec refuses rather than guessing
/// a working directory.
#[test]
fn exec_is_an_error_when_the_top_level_call_cannot_run() {
    let report = exec_repo(
        &TopCallFailsSys,
        Path::new("/repo"),
        "repo",
        &["ls".to_owned()],
    );
    assert_eq!(
        report.error.as_deref(),
        Some("Could not read the repository.")
    );
}

/// Guards a git directory given as the path; exec runs in it, as it has no work tree.
#[test]
fn exec_in_a_git_directory_runs_there() {
    let report = exec_repo(
        &GitDirSys,
        Path::new("/repo/.git"),
        ".git",
        &["ls".to_owned()],
    );
    assert_eq!(report.error, None);
    assert_eq!(report.stdout, "/repo/.git");
}

/// Guards an update through a git directory; with no work tree it moves the
/// branch with `branch -f`, like a bare repository, instead of merging.
#[test]
fn update_through_a_git_directory_fast_forwards() {
    let options = Options {
        current_only: false,
        fetch_only: false,
        prune: false,
    };
    let report = update_repo(&GitDirSys, Path::new("/repo/.git"), ".git", &options);
    match branch_outcome(&report, "main") {
        BranchOutcome::Updated { from, to } => {
            assert_eq!(from, "aaa");
            assert_eq!(to, "bbb");
        }
        _ => panic!("unexpected outcome for main"),
    }
}

/// Guards the unborn case; the tracked remote fetches while the empty ref
/// snapshot gives the branch step nothing to visit.
#[test]
fn an_unborn_branch_with_current_only_fetches_and_visits_no_branches() {
    let sys = StubSys::new(FetchStub::Succeeds);
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
                outcome: FetchOutcome::UpToDate,
            },
        ] => assert_eq!(remote, "origin"),
        steps => panic!("unexpected steps: {}", steps.len()),
    }
}

/// A branch tracking a tag reference, outside the heads and remotes namespaces.
const TAG_UPSTREAM_REFS: &str = "refs/heads/main\0aaa\0refs/tags/v1\n\
    refs/tags/v1\0bbb\0\n";

/// A system where `main` can fast-forward to the tag it tracks.
struct TagUpstreamSys;

impl Sys for TagUpstreamSys {
    fn git(&self, _dir: &Path, argv: &[&str]) -> io::Result<Output> {
        let script = CommonScript {
            current: "other",
            refs: TAG_UPSTREAM_REFS,
            ..COMMON
        };
        if let Some(answer) = common_answer(&script, argv) {
            return answer;
        }
        match argv {
            ["fetch", ..] => Ok(output(0, "", "")),
            ["merge-base", "aaa", "bbb"] => Ok(output(0, "aaa\n", "")),
            ["branch", "-f", ..] => Ok(output(0, "", "")),
            _ => panic!("unexpected git call: {argv:?}"),
        }
    }
}

/// Guards an upstream outside refs/heads and refs/remotes; the branch still
/// fast-forwards to the tag, instead of being skipped as if the ref were missing.
#[test]
fn a_branch_tracking_a_tag_fast_forwards() {
    let options = Options {
        current_only: false,
        fetch_only: false,
        prune: false,
    };
    let report = update_repo(&TagUpstreamSys, Path::new("/repo"), "repo", &options);
    match branch_outcome(&report, "main") {
        BranchOutcome::Updated { from, to } => {
            assert_eq!(from, "aaa");
            assert_eq!(to, "bbb");
        }
        _ => panic!("unexpected outcome for main"),
    }
}

/// Guards that exec output is passed through raw, with both streams and no error.
#[test]
fn exec_reports_the_command_output() {
    let report = exec_repo(&ExecSys, Path::new("/repo"), "repo", &["ls".to_owned()]);
    assert_eq!(report.name, "repo");
    assert_eq!(report.stdout, "hello\n");
    assert_eq!(report.stderr, "warn\n");
    assert_eq!(report.error, None);
}

/// Guards a bare reference name; it is kept whole when there are no prefix parts to drop.
#[test]
fn parse_fetch_output_keeps_a_reference_with_no_parts_to_drop() {
    let text = "\n* 0 0 v1\n";
    assert_eq!(
        parse_fetch_output(text, "origin"),
        [(Kind::NewBranch, vec!["v1".to_owned()])]
    );
}

/// Guards grouping; repeated kinds share one group so each kind prints once.
#[test]
fn parse_fetch_output_groups_repeated_kinds_together() {
    let text = "* 0 0 refs/remotes/origin/a\n* 0 0 refs/remotes/origin/b\n";
    assert_eq!(
        parse_fetch_output(text, "origin"),
        [(Kind::NewBranch, vec!["a".to_owned(), "b".to_owned()])]
    );
}

/// Guards that a branch the snapshot does not hold still records its step; only a held
/// branch gets its snapshot SHA moved, so the lookup must tolerate a miss.
#[test]
fn push_updated_records_the_step_when_the_snapshot_lacks_the_branch() {
    let branches = [Branch {
        name: "main".to_owned(),
        sha: "aa".to_owned(),
        upstream: "refs/remotes/origin/main".to_owned(),
    }];
    let mut shas = vec![("refs/remotes/origin/main".to_owned(), "cc".to_owned())];
    let mut slot = None;
    push_updated(&mut slot, &branches, &mut shas, 0, "bb");
    match &slot {
        Some(Step::Branch {
            name,
            outcome: BranchOutcome::Updated { from, to },
        }) => {
            assert_eq!(name, "main");
            assert_eq!(from, "aa");
            assert_eq!(to, "bb");
        }
        _ => panic!("unexpected steps"),
    }
    assert_eq!(
        shas,
        [("refs/remotes/origin/main".to_owned(), "cc".to_owned())]
    );
}

/// Guards parsing of each fetch flag; forced updates are not reported and names lose their prefix.
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

/// Two branches tracking each other, plus one tracking itself: the dependency
/// order has no ready branch until the cycle is broken by name order.
const CYCLE_REFS: &str = "refs/heads/a\0a1\0refs/heads/b\n\
    refs/heads/b\0b1\0refs/heads/a\n\
    refs/heads/loop\0l1\0refs/heads/loop\n";

/// A system scripting the cycle: `a` fast-forwards to `b`, after which `b` is up
/// to date against the moved `a`; `loop` is up to date against itself.
struct CycleSys;

impl Sys for CycleSys {
    /// Answer the snapshot and merge bases for the mutual-tracking cycle.
    fn git(&self, _dir: &Path, argv: &[&str]) -> io::Result<Output> {
        let script = CommonScript {
            current: "other",
            refs: CYCLE_REFS,
            ..COMMON
        };
        if let Some(answer) = common_answer(&script, argv) {
            return answer;
        }
        match argv {
            ["fetch", ..] => Ok(output(0, "", "")),
            ["merge-base", "a1", "b1"] => Ok(output(0, "a1\n", "")),
            ["merge-base", "b1", "b1"] => Ok(output(0, "b1\n", "")),
            ["merge-base", "l1", "l1"] => Ok(output(0, "l1\n", "")),
            ["branch", "-f", ..] => Ok(output(0, "", "")),
            _ => panic!("unexpected git call: {argv:?}"),
        }
    }
}

/// Guards a tracking cycle; processing must terminate, the movable branch still
/// moves, and a self-tracking branch reads as up to date, not as a dependency.
#[test]
fn a_tracking_cycle_terminates_and_still_updates() {
    let options = Options {
        current_only: false,
        fetch_only: false,
        prune: false,
    };
    let report = update_repo(&CycleSys, Path::new("/repo"), "repo", &options);
    match branch_outcome(&report, "a") {
        BranchOutcome::Updated { from, to } => {
            assert_eq!(from, "a1");
            assert_eq!(to, "b1");
        }
        _ => panic!("unexpected outcome for a"),
    }
    assert!(matches!(
        branch_outcome(&report, "b"),
        BranchOutcome::UpToDate
    ));
    assert!(matches!(
        branch_outcome(&report, "loop"),
        BranchOutcome::UpToDate
    ));
}

/// A clean branch name whose upstream holds the replacement character, as lossy
/// decoding of a non-UTF-8 remote branch produces.
const MANGLED_UPSTREAM_REFS: &str = "refs/heads/good\0g1\0refs/remotes/origin/bad\u{FFFD}\n\
    refs/remotes/origin/bad\u{FFFD}\0g2\0\n";

/// Guards a non-UTF-8 upstream name; the branch is an error step and git is never
/// asked to move it, even though its own name is clean.
#[test]
fn an_upstream_name_that_is_not_valid_utf8_is_an_error() {
    let options = Options {
        current_only: false,
        fetch_only: false,
        prune: false,
    };
    let report = update_repo(
        &MangledSys {
            refs: MANGLED_UPSTREAM_REFS,
        },
        Path::new("/repo"),
        "repo",
        &options,
    );
    match branch_outcome(&report, "good") {
        BranchOutcome::Error(message) => {
            assert_eq!(message, "branch or upstream name is not valid UTF-8")
        }
        _ => panic!("unexpected outcome for good"),
    }
}

/// Mangled names on branches that need no move: one has no upstream, the other's
/// upstream is missing locally. Both are silent cases, refusal or not.
const MANGLED_INERT_REFS: &str = "refs/heads/bad\u{FFFD}name\0b1\0\n\
    refs/heads/lost\u{FFFD}name\0c1\0refs/remotes/origin/gone\n";

/// Guards the refusal's ordering: a mangled branch that would not move anyway is
/// skipped silently, like any other branch with no upstream or a missing one.
#[test]
fn a_mangled_branch_that_needs_no_move_is_skipped_silently() {
    let options = Options {
        current_only: false,
        fetch_only: false,
        prune: false,
    };
    let report = update_repo(
        &MangledSys {
            refs: MANGLED_INERT_REFS,
        },
        Path::new("/repo"),
        "repo",
        &options,
    );
    let branch_steps = report
        .steps
        .iter()
        .filter(|step| matches!(step, Step::Branch { .. }))
        .count();
    assert_eq!(branch_steps, 0);
}

/// A mangled branch already at its upstream: the snapshot's own SHA is all the
/// comparison needs, so it reads as up to date, not as a name error.
const MANGLED_CURRENT_REFS: &str = "refs/heads/bad\u{FFFD}name\0m2\0refs/remotes/origin/main\n\
    refs/remotes/origin/main\0m2\0\n";

/// Guards the refusal's ordering: a mangled branch that is up to date reports
/// that, because no move means no name is spelled back to git.
#[test]
fn a_mangled_branch_that_is_up_to_date_reports_up_to_date() {
    let options = Options {
        current_only: false,
        fetch_only: false,
        prune: false,
    };
    let report = update_repo(
        &MangledSys {
            refs: MANGLED_CURRENT_REFS,
        },
        Path::new("/repo"),
        "repo",
        &options,
    );
    assert!(matches!(
        branch_outcome(&report, "bad\u{FFFD}name"),
        BranchOutcome::UpToDate
    ));
}
