// Integration tests run the real binary, so unwrap is the assertion.
#![allow(clippy::unwrap_used)]

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

/// Run the built binary in `dir` with `HOME` pinned so tests stay hermetic.
fn nine_up(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_9up"))
        .args(args)
        .current_dir(dir)
        .env("HOME", dir)
        .output()
        .unwrap()
}

/// Run the binary with extra environment variables, for `HOME` and `XDG_CONFIG_HOME` tests.
fn nine_up_env(dir: &Path, args: &[&str], envs: &[(&str, &str)]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_9up"));
    command.args(args).current_dir(dir).env("HOME", dir);
    for (key, value) in envs {
        command.env(key, value);
    }
    command.output().unwrap()
}

/// A temp directory and its config file, as a path and as text for `-C`.
fn temp_config() -> (tempfile::TempDir, std::path::PathBuf, String) {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("config.toml");
    let text = file.to_str().unwrap().to_owned();
    (dir, file, text)
}

/// Run the binary with `suffix` from a deleted current directory; the config names only
/// `/srv/repos`.
fn run_from_deleted_cwd(dir: &Path, suffix: &str) -> Output {
    let gone = dir.join("gone");
    fs::create_dir(&gone).unwrap();
    let config = dir.join("config.toml");
    fs::write(&config, "repositories = [\"/srv/repos\"]\n").unwrap();
    let script = format!(
        "cd {} && rmdir {} && exec {} -C {} {suffix}",
        gone.display(),
        gone.display(),
        env!("CARGO_BIN_EXE_9up"),
        config.display()
    );
    Command::new("sh")
        .args(["-c", &script])
        .env("HOME", dir)
        .output()
        .unwrap()
}

/// Decode stdout leniently, so a failing assertion can still print what the binary wrote.
fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// Guards `add`, `list` and `delete` end to end; they are the main way users edit the config.
#[test]
fn add_list_and_delete_round_trip() {
    let (dir, file, text) = temp_config();
    let b = text.as_str();
    // getcwd resolves the macOS /var symlink over temp dirs; canonicalise to match the stored path.
    let repo = fs::canonicalize(dir.path()).unwrap().join("repo");
    let repo = repo.display().to_string();

    let added = nine_up(dir.path(), &["-C", b, "add", "repo"]);
    assert!(added.status.success());
    assert!(stdout(&added).contains("Added"));
    assert!(stdout(&added).contains(&repo));
    // A new file is the one key, with each entry on its own line.
    assert_eq!(
        fs::read_to_string(&file).unwrap(),
        format!("repositories = [\n    \"{repo}\"\n]\n")
    );

    let again = nine_up(dir.path(), &["-C", b, "add", "repo"]);
    assert!(stdout(&again).contains("already in the config file"));

    let listed = nine_up(dir.path(), &["--config", b, "list"]);
    assert!(stdout(&listed).contains(&repo));

    let deleted = nine_up(dir.path(), &["-C", b, "delete", "repo"]);
    assert!(stdout(&deleted).contains("Deleted"));

    let missing = nine_up(dir.path(), &["-C", b, "delete", "repo"]);
    assert!(stdout(&missing).contains("not in the config file"));

    let empty = nine_up(dir.path(), &["-C", b, "list"]);
    assert!(stdout(&empty).contains("No repositories configured."));
    assert_eq!(fs::read_to_string(&file).unwrap(), "repositories = []\n");
}

/// Run git isolated from the user's config, so a local setting cannot change a test's result.
fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?} failed");
}

/// Commit one file, the smallest step that gives a remote something new to fetch.
fn commit_file(dir: &Path, name: &str) {
    fs::write(dir.join(name), name).unwrap();
    git(dir, &["add", name]);
    git(dir, &["commit", "-qm", &format!("add {name}")]);
}

/// Create an empty repository on `main`, enough for discovery to find it.
fn make_repo(dir: &Path) {
    fs::create_dir_all(dir).unwrap();
    git(dir, &["init", "-b", "main"]);
}

/// Guards discovery; a base path, a single repository and a missing path each report differently.
#[test]
fn discovery_prints_headers_and_names() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("base");
    make_repo(&base.join("a/repo"));
    make_repo(&dir.path().join("solo"));

    let listed = nine_up(dir.path(), &["update", base.to_str().unwrap()]);
    assert!(stdout(&listed).contains("(1 repository)"));
    assert!(stdout(&listed).contains("a/repo"));

    let solo = nine_up(dir.path(), &[dir.path().join("solo").to_str().unwrap()]);
    assert!(stdout(&solo).contains("\n  solo"), "{}", stdout(&solo));

    let empty = nine_up(
        dir.path(),
        &["update", dir.path().join("nope").to_str().unwrap()],
    );
    assert!(stdout(&empty).contains("doesn't exist"));
}

/// Guards the no argument run and `clean`, which drops missing paths but keeps comment entries.
#[test]
fn bare_run_uses_the_configured_paths_and_clean_prunes_them() {
    let (dir, file, text) = temp_config();
    let b = text.as_str();
    make_repo(&dir.path().join("kept"));
    fs::write(
        &file,
        format!(
            "repositories = [\"{}\", \"/gone\", \"# note\"]\n",
            dir.path().join("kept").display()
        ),
    )
    .unwrap();

    let bare = nine_up(dir.path(), &["-C", b]);
    assert!(stdout(&bare).contains("kept"));
    assert!(stdout(&bare).contains("doesn't exist"));
    assert!(stdout(&bare).contains("note"));

    // Clean removes /gone but never a comment entry.
    let cleaned = nine_up(dir.path(), &["-C", b, "clean"]);
    assert!(stdout(&cleaned).contains("Removed /gone."));
    assert!(!stdout(&cleaned).contains("Removed # note."));
    assert_eq!(
        fs::read_to_string(&file).unwrap(),
        format!(
            "repositories = [\"{}\", \"# note\"]\n",
            dir.path().join("kept").display()
        )
    );

    let listed = nine_up(dir.path(), &["-C", b, "list"]);
    assert!(stdout(&listed).contains(&dir.path().join("kept").display().to_string()));
    assert!(stdout(&listed).contains("# note"));

    let again = nine_up(dir.path(), &["-C", b, "clean"]);
    assert!(stdout(&again).contains("Nothing to clean."));
}

/// Guards an empty config; the user gets a hint to add a repository, not silence.
#[test]
fn no_configured_paths_prints_the_hint() {
    let (dir, file, _) = temp_config();
    let out = nine_up(dir.path(), &["-C", file.to_str().unwrap()]);
    assert!(stdout(&out).contains("No repositories configured."));
}

/// Guards the fetch summary; new branches, new tags and moved references must each be named.
#[test]
fn update_reports_what_a_fetch_brought_in() {
    let dir = tempfile::tempdir().unwrap();
    let (_remote, work, stale) = remote_work_stale(dir.path());

    git(&work, &["checkout", "-qb", "feat"]);
    commit_file(&work, "b");
    git(&work, &["push", "-q", "origin", "feat"]);
    git(&work, &["checkout", "-q", "main"]);
    commit_file(&work, "c");
    git(&work, &["push", "-q", "origin", "main"]);
    git(&work, &["tag", "v1"]);
    git(&work, &["push", "-q", "origin", "v1"]);

    let out = nine_up(dir.path(), &["update", stale.to_str().unwrap()]);
    let text = stdout(&out);
    assert!(text.contains("new branches: feat"), "{text}");
    assert!(text.contains("new tags: v1"), "{text}");
    assert!(text.contains("updated references: main"), "{text}");

    let again = nine_up(dir.path(), &["update", stale.to_str().unwrap()]);
    assert!(stdout(&again).contains("origin is up to date."));
}

/// Guards fetching every remote; a remote with no refspec is skipped, not failed.
#[test]
fn two_remotes_both_fetch_and_a_refspec_less_remote_is_skipped() {
    let dir = tempfile::tempdir().unwrap();
    let (remote, work, stale) = remote_work_stale(dir.path());
    git(
        &stale,
        &["remote", "add", "upstream", remote.to_str().unwrap()],
    );
    git(
        &stale,
        &["remote", "add", "empty", remote.to_str().unwrap()],
    );
    git(&stale, &["config", "--unset", "remote.empty.fetch"]);

    commit_file(&work, "b");
    git(&work, &["push", "-q", "origin", "main"]);

    let out = nine_up(dir.path(), &["update", stale.to_str().unwrap()]);
    let text = stdout(&out);
    assert!(
        text.contains("Fetched origin: updated references: main."),
        "{text}"
    );
    assert!(
        text.contains("Fetched upstream: new branches: main."),
        "{text}"
    );
    assert!(
        text.contains("empty has no fetch refspec. Skipped."),
        "{text}"
    );
}

/// Guards that a failed fetch stops later remotes, so none reports success after an error.
#[test]
fn a_failed_fetch_stops_the_remaining_remotes() {
    let dir = tempfile::tempdir().unwrap();
    let (remote, _work, stale) = remote_work_stale(dir.path());
    git(
        &stale,
        &["remote", "add", "upstream", remote.to_str().unwrap()],
    );
    git(
        &stale,
        &["remote", "set-url", "origin", "/nonexistent-9up/remote.git"],
    );

    let out = nine_up(dir.path(), &["update", stale.to_str().unwrap()]);
    let text = stdout(&out);
    assert!(text.contains("fatal:"), "{text}");
    assert!(!text.contains("Fetched upstream"), "{text}");
    assert!(!text.contains("upstream is up to date."), "{text}");
}

/// Read a reference's commit, so tests can prove whether a branch moved.
fn sha(dir: &Path, reference: &str) -> String {
    let out = Command::new("git")
        .args(["rev-parse", reference])
        .current_dir(dir)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

/// Guards the branch step; only fast-forwards move branches, so a diverged one keeps local commits.
#[test]
fn branch_step_fast_forwards_and_skips() {
    let dir = tempfile::tempdir().unwrap();
    let (_remote, work, stale) = remote_work_stale(dir.path());
    git(&work, &["checkout", "-qb", "side"]);
    commit_file(&work, "s");
    git(&work, &["push", "-q", "origin", "side"]);
    git(&work, &["checkout", "-qb", "diverged"]);
    commit_file(&work, "d");
    git(&work, &["push", "-q", "origin", "diverged"]);
    git(&work, &["checkout", "-q", "main"]);

    git(&stale, &["fetch", "-q", "origin"]);
    git(&stale, &["checkout", "-qb", "side", "origin/side"]);
    git(&stale, &["checkout", "-qb", "diverged", "origin/diverged"]);
    git(&stale, &["checkout", "-qb", "lonely"]);
    commit_file(&stale, "mine");
    git(&stale, &["checkout", "-q", "main"]);
    // Diverge: a local commit on diverged that the remote never sees.
    git(&stale, &["checkout", "-q", "diverged"]);
    commit_file(&stale, "local-only");
    git(&stale, &["checkout", "-q", "main"]);
    let diverged_before = sha(&stale, "diverged");

    git(&work, &["checkout", "-q", "main"]);
    commit_file(&work, "b");
    git(&work, &["push", "-q", "origin", "main"]);
    git(&work, &["checkout", "-q", "side"]);
    commit_file(&work, "s2");
    git(&work, &["push", "-q", "origin", "side"]);
    git(&work, &["checkout", "-q", "diverged"]);
    commit_file(&work, "d2");
    git(&work, &["push", "-q", "origin", "diverged"]);

    let out = nine_up(dir.path(), &["update", stale.to_str().unwrap()]);
    let text = stdout(&out);
    assert!(text.contains("main updated:"), "{text}");
    assert!(text.contains("side updated:"), "{text}");
    assert!(
        text.contains("diverged skipped: a fast-forward is not possible."),
        "{text}"
    );
    assert!(!text.contains("lonely"), "{text}");
    assert_eq!(sha(&stale, "main"), sha(&stale, "origin/main"));
    assert_eq!(sha(&stale, "side"), sha(&stale, "origin/side"));
    assert_eq!(sha(&stale, "diverged"), diverged_before);

    let again = nine_up(dir.path(), &["update", stale.to_str().unwrap()]);
    assert!(stdout(&again).contains("main is up to date."));
}

/// Guards `-f` and a dirty work tree; neither may move the checked out branch.
#[test]
fn fetch_only_and_uncommitted_changes() {
    let dir = tempfile::tempdir().unwrap();
    let (_remote, work, stale) = remote_work_stale(dir.path());
    commit_file(&work, "b");
    git(&work, &["push", "-q", "origin", "main"]);
    let behind = sha(&stale, "main");

    let fetched = nine_up(dir.path(), &["update", "-f", stale.to_str().unwrap()]);
    assert!(
        !stdout(&fetched).contains("main updated:"),
        "{}",
        stdout(&fetched)
    );
    assert_eq!(sha(&stale, "main"), behind);

    // The incoming commit changes "a"; a dirty "a" blocks the merge.
    fs::write(work.join("a"), "changed remotely").unwrap();
    git(&work, &["add", "a"]);
    git(&work, &["commit", "-qm", "change a"]);
    git(&work, &["push", "-q", "origin", "main"]);
    fs::write(stale.join("a"), "dirty locally").unwrap();
    let blocked = nine_up(dir.path(), &["update", stale.to_str().unwrap()]);
    let text = stdout(&blocked);
    assert!(
        text.contains("main skipped: uncommitted changes."),
        "{text}"
    );
    assert_eq!(sha(&stale, "main"), behind);
}

/// Guards bare repositories; with no work tree, the `HEAD` branch must still fast-forward.
#[test]
fn a_bare_repository_updates_its_head_branch() {
    let dir = tempfile::tempdir().unwrap();
    let (remote, work, _stale) = remote_work_stale(dir.path());

    let bare = dir.path().join("bare.git");
    git(
        dir.path(),
        &["init", "-q", "-b", "main", "--bare", bare.to_str().unwrap()],
    );
    git(
        &bare,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    git(&bare, &["fetch", "-q", "origin"]);
    git(&bare, &["branch", "main", "refs/remotes/origin/main"]);
    git(&bare, &["symbolic-ref", "HEAD", "refs/heads/main"]);

    commit_file(&work, "b");
    git(&work, &["push", "-q", "origin", "main"]);

    let out = nine_up(dir.path(), &["update", bare.to_str().unwrap()]);
    let text = stdout(&out);
    assert!(text.contains("main updated:"), "{text}");
    assert_eq!(sha(&bare, "main"), sha(&bare, "refs/remotes/origin/main"));
}

/// Guards a branch tracking a local branch; it must follow that branch after it moves.
#[test]
fn a_branch_tracking_a_local_branch_sees_its_move() {
    let dir = tempfile::tempdir().unwrap();
    let (_remote, work, stale) = remote_work_stale(dir.path());
    git(&stale, &["branch", "-m", "main", "a-main"]);
    git(&stale, &["checkout", "-qb", "b-follow", "a-main"]);
    git(&stale, &["branch", "--set-upstream-to=a-main", "b-follow"]);
    git(&stale, &["checkout", "-q", "a-main"]);

    commit_file(&work, "b");
    git(&work, &["push", "-q", "origin", "main"]);

    let out = nine_up(dir.path(), &["update", stale.to_str().unwrap()]);
    let text = stdout(&out);
    assert!(text.contains("a-main updated:"), "{text}");
    assert!(text.contains("b-follow updated:"), "{text}");
    assert_eq!(sha(&stale, "b-follow"), sha(&stale, "origin/main"));
}

/// Guards `exec` output order and exit status; a failing command must not fail the run.
#[test]
fn exec_runs_in_each_repository_and_ignores_the_exit_status() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    make_repo(&repo);

    let out = nine_up(
        dir.path(),
        &[
            "exec",
            repo.to_str().unwrap(),
            "--",
            "sh",
            "-c",
            "echo from $(basename $(pwd)); echo to-stderr >&2; exit 3",
        ],
    );
    assert!(out.status.success(), "{out:?}");
    let text = stdout(&out);
    let stdout_at = text.find("from repo").unwrap();
    let stderr_at = text.find("to-stderr").unwrap();
    assert!(stdout_at < stderr_at, "{text}");

    let missing = nine_up(
        dir.path(),
        &["exec", repo.to_str().unwrap(), "--", "/nonexistent-9up/cmd"],
    );
    assert!(stdout(&missing).contains("Could not run /nonexistent-9up/cmd:"));
}

/// Guards `exec` with no command; it is a usage error and must not change the config.
#[test]
fn exec_with_no_command_is_a_usage_error_and_touches_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("config.toml");
    fs::write(&file, "repositories = [\"/somewhere\"]\n").unwrap();
    let out = nine_up(dir.path(), &["-C", file.to_str().unwrap(), "exec"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(stdout(&out).is_empty(), "{}", stdout(&out));
    assert_eq!(
        fs::read_to_string(&file).unwrap(),
        "repositories = [\"/somewhere\"]\n"
    );
}

/// Guards `exec` with an empty config; only the banner is printed, with no hint or error.
#[test]
fn exec_with_no_configured_paths_prints_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("config.toml");
    let out = nine_up(
        dir.path(),
        &["-C", file.to_str().unwrap(), "exec", "--", "true"],
    );
    assert_eq!(stdout(&out), "9up\n\n");
}

/// Guards the `-t` short flag for depth; it must parse even with a negative value.
#[test]
fn depth_uses_the_short_flag_t() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    make_repo(&repo);
    let out = nine_up(dir.path(), &["update", "-t", "-1", repo.to_str().unwrap()]);
    assert!(out.status.success(), "{out:?}");
    assert!(stdout(&out).contains("repo (1 repository)"));
}

/// Check that a reference exists, so tests can prove a prune removed it.
fn ref_exists(dir: &Path, reference: &str) -> bool {
    Command::new("git")
        .args(["rev-parse", "--verify", "-q", reference])
        .current_dir(dir)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap()
        .status
        .success()
}

// Shared setup: a bare remote, a work clone that pushes to it, and a
// stale clone that falls behind. Returns (remote, work, stale).
fn remote_work_stale(dir: &Path) -> (std::path::PathBuf, std::path::PathBuf, std::path::PathBuf) {
    let remote = dir.join("remote.git");
    git(
        dir,
        &[
            "init",
            "-q",
            "-b",
            "main",
            "--bare",
            remote.to_str().unwrap(),
        ],
    );
    let work = dir.join("work");
    git(
        dir,
        &[
            "clone",
            "-q",
            remote.to_str().unwrap(),
            work.to_str().unwrap(),
        ],
    );
    commit_file(&work, "a");
    git(&work, &["push", "-q", "origin", "main"]);
    let stale = dir.join("stale");
    git(
        dir,
        &[
            "clone",
            "-q",
            remote.to_str().unwrap(),
            stale.to_str().unwrap(),
        ],
    );
    (remote, work, stale)
}

/// Guards `-c`; only the current branch's remote is fetched, so other remotes stay untouched.
#[test]
fn current_only_fetches_only_the_tracked_remote() {
    let dir = tempfile::tempdir().unwrap();
    let (remote, work, stale) = remote_work_stale(dir.path());
    let other = dir.path().join("other.git");
    git(
        dir.path(),
        &[
            "init",
            "-q",
            "-b",
            "main",
            "--bare",
            other.to_str().unwrap(),
        ],
    );
    git(&work, &["remote", "add", "other", other.to_str().unwrap()]);
    git(&work, &["push", "-q", "other", "main"]);
    git(
        &stale,
        &["remote", "add", "upstream", other.to_str().unwrap()],
    );
    git(&stale, &["fetch", "-q", "upstream"]);
    let upstream_before = sha(&stale, "refs/remotes/upstream/main");

    commit_file(&work, "b");
    git(&work, &["push", "-q", "origin", "main"]);
    git(&work, &["push", "-q", "other", "main"]);
    let _ = remote;

    let out = nine_up(dir.path(), &["update", "-c", stale.to_str().unwrap()]);
    let text = stdout(&out);
    assert!(text.contains("Fetched origin"), "{text}");
    assert!(!text.contains("upstream"), "{text}");
    assert_eq!(sha(&stale, "refs/remotes/origin/main"), sha(&work, "main"));
    assert_eq!(sha(&stale, "refs/remotes/upstream/main"), upstream_before);
}

/// Guards `list` from a deleted current directory; it needs no cwd, so it must still work.
#[test]
fn list_survives_a_deleted_current_directory() {
    let dir = tempfile::tempdir().unwrap();
    let out = run_from_deleted_cwd(dir.path(), "list");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(stdout(&out).contains("/srv/repos"), "{}", stdout(&out));
}

/// Guards `-c` scope; it limits the fetch, not the branch step, so other branches still update.
#[test]
fn current_only_still_updates_a_non_current_branch() {
    let dir = tempfile::tempdir().unwrap();
    let (_remote, work, stale) = remote_work_stale(dir.path());
    git(&work, &["checkout", "-q", "-b", "side"]);
    commit_file(&work, "s1");
    git(&work, &["push", "-q", "origin", "side"]);
    git(&work, &["checkout", "-q", "main"]);
    git(&stale, &["fetch", "-q", "origin"]);
    git(&stale, &["branch", "--track", "side", "origin/side"]);
    git(&work, &["checkout", "-q", "side"]);
    commit_file(&work, "s2");
    git(&work, &["push", "-q", "origin", "side"]);
    git(&work, &["checkout", "-q", "main"]);

    let out = nine_up(dir.path(), &["update", "-c", stale.to_str().unwrap()]);
    let text = stdout(&out);
    assert!(text.contains("side updated:"), "{text}");
    assert_eq!(sha(&stale, "side"), sha(&work, "side"));
}

/// Guards `-c` on a detached `HEAD`; with no tracked remote, the user gets a clear error.
#[test]
fn current_only_with_a_detached_head_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let (_remote, _work, stale) = remote_work_stale(dir.path());
    let head = sha(&stale, "HEAD");
    git(&stale, &["checkout", "-q", &head]);
    let out = nine_up(dir.path(), &["update", "-c", stale.to_str().unwrap()]);
    assert!(
        stdout(&out).contains("HEAD is detached, so no remote is tracked."),
        "{}",
        stdout(&out)
    );
}

/// Guards a detached `HEAD`; local branches must still fast-forward during a full update.
#[test]
fn a_detached_head_update_still_updates_branches() {
    let dir = tempfile::tempdir().unwrap();
    let (_remote, work, stale) = remote_work_stale(dir.path());
    let head = sha(&stale, "HEAD");
    git(&stale, &["checkout", "-q", &head]);
    commit_file(&work, "b");
    git(&work, &["push", "-q", "origin", "main"]);
    let out = nine_up(dir.path(), &["update", stale.to_str().unwrap()]);
    let text = stdout(&out);
    assert!(text.contains("main updated:"), "{text}");
    assert_eq!(sha(&stale, "main"), sha(&stale, "refs/remotes/origin/main"));
}

/// Guards `--prune`; a branch deleted on the remote must lose its remote tracking reference.
#[test]
fn prune_removes_a_branch_deleted_on_the_remote() {
    let dir = tempfile::tempdir().unwrap();
    let (_remote, work, stale) = remote_work_stale(dir.path());
    git(&work, &["checkout", "-qb", "feat"]);
    commit_file(&work, "f");
    git(&work, &["push", "-q", "origin", "feat"]);
    git(&work, &["checkout", "-q", "main"]);
    let first = nine_up(dir.path(), &["update", stale.to_str().unwrap()]);
    assert!(stdout(&first).contains("feat"), "{}", stdout(&first));
    assert!(ref_exists(&stale, "refs/remotes/origin/feat"));

    git(&work, &["push", "-q", "origin", "--delete", "feat"]);
    let out = nine_up(dir.path(), &["update", "--prune", stale.to_str().unwrap()]);
    assert!(out.status.success(), "{out:?}");
    assert!(!ref_exists(&stale, "refs/remotes/origin/feat"));
}

/// Guards the `--` separators in git calls; a dash leading name must never be read as an option.
#[test]
fn dash_leading_branch_and_remote_are_protected_by_separators() {
    let dir = tempfile::tempdir().unwrap();
    let (remote, work, stale) = remote_work_stale(dir.path());
    // A branch named `-x`, created by ref so no flag parsing is involved.
    git(&work, &["update-ref", "refs/heads/-x", "HEAD"]);
    git(
        &work,
        &["push", "-q", "origin", "refs/heads/-x:refs/heads/-x"],
    );
    git(&stale, &["fetch", "-q", "origin"]);
    git(
        &stale,
        &["update-ref", "refs/heads/-x", "refs/remotes/origin/-x"],
    );
    git(&stale, &["config", "branch.-x.remote", "origin"]);
    git(&stale, &["config", "branch.-x.merge", "refs/heads/-x"]);
    // A remote named `-x`, set by config for the same reason.
    git(
        &stale,
        &["config", "remote.-x.url", remote.to_str().unwrap()],
    );
    git(
        &stale,
        &[
            "config",
            "remote.-x.fetch",
            "+refs/heads/*:refs/remotes/-x/*",
        ],
    );
    git(&stale, &["fetch", "-q", "--", "-x"]);
    assert!(ref_exists(&stale, "refs/remotes/-x/main"));

    commit_file(&work, "b");
    git(&work, &["push", "-q", "origin", "main"]);
    git(&work, &["update-ref", "refs/heads/-x", "HEAD"]);
    git(
        &work,
        &["push", "-q", "origin", "refs/heads/-x:refs/heads/-x"],
    );

    let out = nine_up(dir.path(), &["update", stale.to_str().unwrap()]);
    let text = stdout(&out);
    // Without `--` in `git fetch ... -- -x`, git sees an unknown switch and the ref does not move.
    assert_eq!(sha(&stale, "refs/remotes/-x/main"), sha(&work, "main"));
    assert!(text.contains("Fetched -x:"), "{text}");
    // The branch step needs `--` in `git branch -f -- -x ...` to get the name past option parsing
    // to git's branch name check, which rejects it; without `--` git reports an unknown switch.
    assert!(
        text.contains("-x: fatal: '-x' is not a valid branch name"),
        "{text}"
    );
}

/// Guards depth for `exec`; `-t` and `--depth` must limit the search as they do for `update`.
#[test]
fn exec_depth_limits_the_search() {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("base");
    make_repo(&base.join("one"));
    make_repo(&base.join("a/two"));

    let shallow = nine_up(
        dir.path(),
        &["exec", "-t", "1", base.to_str().unwrap(), "--", "true"],
    );
    let text = stdout(&shallow);
    assert!(text.contains("(1 repository)"), "{text}");
    assert!(text.contains("one"), "{text}");
    assert!(!text.contains("a/two"), "{text}");

    let deep = nine_up(
        dir.path(),
        &["exec", "--depth", "2", base.to_str().unwrap(), "--", "true"],
    );
    let text = stdout(&deep);
    assert!(text.contains("(2 repositories)"), "{text}");
    assert!(text.contains("a/two"), "{text}");
}

/// Guards the default config path; an invalid `XDG_CONFIG_HOME` or empty `-C` falls back to it.
#[test]
fn default_config_path_ignores_relative_and_empty_xdg_config_home() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().to_str().unwrap();
    // `dirs` picks the XDG path on Linux; macOS uses Application Support and ignores
    // `XDG_CONFIG_HOME`.
    #[cfg(target_os = "macos")]
    let default_file = dir
        .path()
        .join("Library/Application Support/9up/config.toml");
    #[cfg(not(target_os = "macos"))]
    let default_file = dir.path().join(".config/9up/config.toml");
    fs::create_dir_all(default_file.parent().unwrap()).unwrap();
    fs::write(&default_file, "repositories = [\"/from-default\"]\n").unwrap();

    for xdg in ["relative/path", ""] {
        let out = nine_up_env(
            dir.path(),
            &["list"],
            &[("HOME", home), ("XDG_CONFIG_HOME", xdg)],
        );
        assert!(
            stdout(&out).contains("/from-default"),
            "XDG_CONFIG_HOME={xdg:?}: {}",
            stdout(&out)
        );
        // An empty `-C` value uses the same default path.
        let out = nine_up_env(
            dir.path(),
            &["-C", "", "list"],
            &[("HOME", home), ("XDG_CONFIG_HOME", xdg)],
        );
        assert!(
            stdout(&out).contains("/from-default"),
            "XDG_CONFIG_HOME={xdg:?}: {}",
            stdout(&out)
        );
    }
}

/// Guards config edits; user comments and unknown keys must survive `add` and `delete`.
#[test]
fn a_toml_comment_and_an_unknown_key_survive_add_and_delete() {
    let (dir, file, text) = temp_config();
    let b = text.as_str();
    fs::write(
        &file,
        "# my note\nother = 1\nrepositories = [\n    # first\n    \"/a\",\n]\n",
    )
    .unwrap();

    let added = nine_up(dir.path(), &["-C", b, "add", "repo"]);
    assert!(added.status.success(), "{}", stdout(&added));
    let deleted = nine_up(dir.path(), &["-C", b, "delete", "repo"]);
    assert!(deleted.status.success(), "{}", stdout(&deleted));

    let text = fs::read_to_string(&file).unwrap();
    assert!(text.contains("# my note"), "{text}");
    assert!(text.contains("# first"), "{text}");
    assert!(text.contains("other = 1"), "{text}");
    let listed = nine_up(dir.path(), &["-C", b, "list"]);
    assert_eq!(stdout(&listed), "9up\n\n/a\n");
}

/// Guards `delete` with duplicates; only the first match goes, so later entries keep their place.
#[test]
fn delete_removes_only_the_first_match() {
    let (dir, file, text) = temp_config();
    let b = text.as_str();
    fs::write(&file, "repositories = [\"/a\", \"/b\", \"/a\"]\n").unwrap();

    let deleted = nine_up(dir.path(), &["-C", b, "delete", "/a"]);
    assert!(stdout(&deleted).contains("Deleted /a."));

    let listed = nine_up(dir.path(), &["-C", b, "list"]);
    assert_eq!(stdout(&listed), "9up\n\n/b\n/a\n");
}

/// Guards no op edits; a command that changes nothing must not rewrite the user's file.
#[test]
fn a_no_op_delete_and_clean_leave_the_file_unchanged() {
    let (dir, file, text) = temp_config();
    let b = text.as_str();
    // Non-canonical spacing: any rewrite of the file would show.
    let text = format!(
        "# heading\nrepositories = [\"{}\", \"# note\"]\n",
        dir.path().display()
    );
    fs::write(&file, &text).unwrap();

    let missing = nine_up(dir.path(), &["-C", b, "delete", "/missing"]);
    assert!(stdout(&missing).contains("not in the config file"));
    assert_eq!(fs::read_to_string(&file).unwrap(), text);

    let cleaned = nine_up(dir.path(), &["-C", b, "clean"]);
    assert!(stdout(&cleaned).contains("Nothing to clean."));
    assert_eq!(fs::read_to_string(&file).unwrap(), text);
}

/// Guards broken configs; bad TOML, wrong types and invalid UTF-8 exit 1 with a message.
#[test]
fn an_unparseable_config_fails_with_exit_1() {
    let (dir, file, text) = temp_config();
    let b = text.as_str();
    for text in [
        "repositories = [\n",
        "repositories = \"/a\"\n",
        "repositories = [1]\n",
    ] {
        fs::write(&file, text).unwrap();
        let out = nine_up(dir.path(), &["-C", b, "list"]);
        assert_eq!(out.status.code(), Some(1), "{text}");
        assert!(!out.stderr.is_empty(), "{text}");
    }
    fs::write(&file, [0xff, 0xfe]).unwrap();
    let out = nine_up(dir.path(), &["-C", b, "list"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(!out.stderr.is_empty());
}

/// Guards a config without `repositories`; it means an empty list, not an error.
#[test]
fn a_missing_repositories_key_means_no_repositories() {
    let (dir, file, _) = temp_config();
    fs::write(&file, "other = 1\n").unwrap();
    let out = nine_up(dir.path(), &["-C", file.to_str().unwrap(), "list"]);
    assert!(out.status.success());
    assert!(stdout(&out).contains("No repositories configured."));
}

/// Guards that a `gitup` bookmark file is not read; 9up has its own config and no import.
#[test]
fn a_gitup_bookmark_file_is_ignored() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().to_str().unwrap();
    let old = dir.path().join(".config/gitup");
    fs::create_dir_all(&old).unwrap();
    fs::write(old.join("bookmarks"), "/from-gitup\n").unwrap();

    let out = nine_up_env(
        dir.path(),
        &["list"],
        &[("HOME", home), ("XDG_CONFIG_HOME", "")],
    );
    assert!(out.status.success());
    assert!(stdout(&out).contains("No repositories configured."));
    assert!(!stdout(&out).contains("/from-gitup"));
}

/// Guards a branch whose upstream reference is missing; it must be skipped with no noise.
#[test]
fn a_branch_with_a_missing_upstream_reference_is_silent() {
    let dir = tempfile::tempdir().unwrap();
    let (_remote, _work, stale) = remote_work_stale(dir.path());
    git(&stale, &["checkout", "-qb", "ghost"]);
    commit_file(&stale, "g");
    git(&stale, &["config", "branch.ghost.remote", "origin"]);
    git(
        &stale,
        &["config", "branch.ghost.merge", "refs/heads/ghost"],
    );
    git(&stale, &["checkout", "-q", "main"]);
    let out = nine_up(dir.path(), &["update", stale.to_str().unwrap()]);
    let text = stdout(&out);
    assert!(!text.contains("ghost"), "{text}");
}

/// Guards a branch with no merge base with its upstream; it must be skipped with no noise.
#[test]
fn a_branch_with_no_merge_base_with_its_upstream_is_silent() {
    let dir = tempfile::tempdir().unwrap();
    let (_remote, _work, stale) = remote_work_stale(dir.path());
    git(&stale, &["checkout", "-q", "--orphan", "alien"]);
    commit_file(&stale, "alien-file");
    git(&stale, &["config", "branch.alien.remote", "origin"]);
    git(&stale, &["config", "branch.alien.merge", "refs/heads/main"]);
    git(&stale, &["checkout", "-q", "main"]);
    let out = nine_up(dir.path(), &["update", stale.to_str().unwrap()]);
    let text = stdout(&out);
    assert!(!text.contains("alien"), "{text}");
}

/// Guards `exec` in a bare repository; with no work tree, it runs in the repository directory.
#[test]
fn exec_in_a_bare_repository_runs_in_the_repository_directory() {
    let dir = tempfile::tempdir().unwrap();
    let bare = dir.path().join("bare.git");
    git(
        dir.path(),
        &["init", "-q", "-b", "main", "--bare", bare.to_str().unwrap()],
    );
    let out = nine_up(
        dir.path(),
        &["exec", bare.to_str().unwrap(), "--", "sh", "-c", "pwd"],
    );
    let text = stdout(&out);
    let canonical = fs::canonicalize(&bare).unwrap();
    // Match a whole trimmed line: the header contains the path, so contains() could not fail.
    let printed = text.lines().any(|line| {
        line.trim() == bare.display().to_string() || line.trim() == canonical.display().to_string()
    });
    assert!(printed, "{text}");
}

/// Guards `clean` with an empty config; the user sees the hint, not a silent success.
#[test]
fn clean_with_no_configured_repositories_says_so() {
    let (dir, _file, text) = temp_config();
    let b = text.as_str();
    let out = nine_up(dir.path(), &["-C", b, "clean"]);
    assert!(out.status.success());
    assert!(stdout(&out).contains("No repositories configured."));
}

/// Guards a lone `#` entry; it is an empty comment and must print nothing.
#[test]
fn update_with_a_lone_hash_entry_prints_nothing_for_it() {
    let (dir, file, text) = temp_config();
    fs::write(&file, "repositories = [\"#\"]\n").unwrap();
    let b = text.as_str();
    let out = nine_up(dir.path(), &["-C", b]);
    assert!(out.status.success(), "{}", stdout(&out));
    assert!(!stdout(&out).contains('#'), "{}", stdout(&out));
}

/// Guards `exec` on comment and missing entries; they are reported as `update` reports them.
#[test]
fn exec_prints_comment_and_missing_entries() {
    let (dir, file, text) = temp_config();
    fs::write(
        &file,
        "repositories = [\"# note\", \"/nonexistent-9up-test\"]\n",
    )
    .unwrap();
    let b = text.as_str();
    let out = nine_up(dir.path(), &["-C", b, "exec", "--", "true"]);
    assert!(out.status.success(), "{}", stdout(&out));
    let text = stdout(&out);
    assert!(text.contains("note"), "{text}");
    assert!(text.contains("doesn't exist."), "{text}");
}

/// Guards `update` from a deleted cwd; it needs one, so it must fail with a message.
#[test]
fn update_fails_from_a_deleted_current_directory() {
    let dir = tempfile::tempdir().unwrap();
    let out = run_from_deleted_cwd(dir.path(), "");
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("9up:"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Guards a relative `-C` path; it resolves against the cwd for both writes and reads.
#[test]
fn a_relative_config_path_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let added = nine_up(dir.path(), &["-C", "cfg.toml", "add", "repo"]);
    assert!(added.status.success(), "{}", stdout(&added));
    assert!(dir.path().join("cfg.toml").exists());
    let listed = nine_up(dir.path(), &["-C", "cfg.toml", "list"]);
    assert!(listed.status.success());
    assert!(stdout(&listed).contains("repo"), "{}", stdout(&listed));
}

/// Guards a closed stdout; a broken pipe, as from `head`, must not turn into a failure.
#[test]
fn a_closed_stdout_still_exits_successfully() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config.toml");
    fs::write(&config, "repositories = [\"/srv/repos\"]\n").unwrap();
    let (reader, writer) = std::io::pipe().unwrap();
    drop(reader);
    let status = Command::new(env!("CARGO_BIN_EXE_9up"))
        .args(["-C", config.to_str().unwrap()])
        .current_dir(dir.path())
        .env("HOME", dir.path())
        .stdout(writer)
        .status()
        .unwrap();
    assert!(status.success());
}
