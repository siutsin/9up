// Integration tests run the real binary, so unwrap is the assertion.
#![allow(clippy::unwrap_used)]

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

fn nine_up(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_9up"))
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn add_list_and_delete_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("bookmarks");
    let b = file.to_str().unwrap();

    let added = nine_up(dir.path(), &["-b", b, "add", "repo"]);
    assert!(added.status.success());
    assert!(stdout(&added).contains("Added"));
    assert!(stdout(&added).contains(&dir.path().join("repo").display().to_string()));

    let again = nine_up(dir.path(), &["-b", b, "add", "repo"]);
    assert!(stdout(&again).contains("already bookmarked"));

    let listed = nine_up(dir.path(), &["-b", b, "list"]);
    assert!(stdout(&listed).contains(&dir.path().join("repo").display().to_string()));

    let deleted = nine_up(dir.path(), &["-b", b, "delete", "repo"]);
    assert!(stdout(&deleted).contains("Deleted"));

    let missing = nine_up(dir.path(), &["-b", b, "delete", "repo"]);
    assert!(stdout(&missing).contains("not bookmarked"));

    let empty = nine_up(dir.path(), &["-b", b, "list"]);
    assert!(stdout(&empty).contains("No bookmarks."));
    assert_eq!(fs::read_to_string(&file).unwrap(), "");
}

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?} failed");
}

fn commit_file(dir: &Path, name: &str) {
    fs::write(dir.join(name), name).unwrap();
    git(dir, &["add", name]);
    git(dir, &["commit", "-qm", &format!("add {name}")]);
}

fn make_repo(dir: &Path) {
    fs::create_dir_all(dir).unwrap();
    git(dir, &["init", "-b", "main"]);
}

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
    assert!(stdout(&solo).contains("solo"));

    let empty = nine_up(
        dir.path(),
        &["update", dir.path().join("nope").to_str().unwrap()],
    );
    assert!(stdout(&empty).contains("doesn't exist"));
}

#[test]
fn bare_run_uses_the_bookmarks_and_clean_prunes_them() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("bookmarks");
    let b = file.to_str().unwrap();
    make_repo(&dir.path().join("kept"));
    fs::write(
        &file,
        format!("{}\n/gone\n# note\n", dir.path().join("kept").display()),
    )
    .unwrap();

    let bare = nine_up(dir.path(), &["-b", b]);
    assert!(stdout(&bare).contains("kept"));
    assert!(stdout(&bare).contains("doesn't exist"));
    assert!(stdout(&bare).contains("note"));

    let cleaned = nine_up(dir.path(), &["-b", b, "clean"]);
    assert!(stdout(&cleaned).contains("Removed /gone."));
    assert!(stdout(&cleaned).contains("Removed # note."));
    assert_eq!(
        fs::read_to_string(&file).unwrap(),
        dir.path().join("kept").display().to_string()
    );

    let again = nine_up(dir.path(), &["-b", b, "clean"]);
    assert!(stdout(&again).contains("Nothing to clean."));
}

#[test]
fn no_bookmarks_prints_the_hint() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("bookmarks");
    let out = nine_up(dir.path(), &["-b", file.to_str().unwrap()]);
    assert!(stdout(&out).contains("No bookmarks."));
}

#[test]
fn update_reports_what_a_fetch_brought_in() {
    let dir = tempfile::tempdir().unwrap();
    let remote = dir.path().join("remote.git");
    git(
        dir.path(),
        &[
            "init",
            "-q",
            "-b",
            "main",
            "--bare",
            remote.to_str().unwrap(),
        ],
    );

    let work = dir.path().join("work");
    git(
        dir.path(),
        &[
            "clone",
            "-q",
            remote.to_str().unwrap(),
            work.to_str().unwrap(),
        ],
    );
    commit_file(&work, "a");
    git(&work, &["push", "-q", "origin", "main"]);

    let stale = dir.path().join("stale");
    git(
        dir.path(),
        &[
            "clone",
            "-q",
            remote.to_str().unwrap(),
            stale.to_str().unwrap(),
        ],
    );

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

#[test]
fn two_remotes_both_fetch_and_a_refspec_less_remote_is_skipped() {
    let dir = tempfile::tempdir().unwrap();
    let remote = dir.path().join("remote.git");
    git(
        dir.path(),
        &[
            "init",
            "-q",
            "-b",
            "main",
            "--bare",
            remote.to_str().unwrap(),
        ],
    );
    let work = dir.path().join("work");
    git(
        dir.path(),
        &[
            "clone",
            "-q",
            remote.to_str().unwrap(),
            work.to_str().unwrap(),
        ],
    );
    commit_file(&work, "a");
    git(&work, &["push", "-q", "origin", "main"]);

    let stale = dir.path().join("stale");
    git(
        dir.path(),
        &[
            "clone",
            "-q",
            remote.to_str().unwrap(),
            stale.to_str().unwrap(),
        ],
    );
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

#[test]
fn a_failed_fetch_stops_the_remaining_remotes() {
    let dir = tempfile::tempdir().unwrap();
    let remote = dir.path().join("remote.git");
    git(
        dir.path(),
        &[
            "init",
            "-q",
            "-b",
            "main",
            "--bare",
            remote.to_str().unwrap(),
        ],
    );
    let work = dir.path().join("work");
    git(
        dir.path(),
        &[
            "clone",
            "-q",
            remote.to_str().unwrap(),
            work.to_str().unwrap(),
        ],
    );
    commit_file(&work, "a");
    git(&work, &["push", "-q", "origin", "main"]);

    let stale = dir.path().join("stale");
    git(
        dir.path(),
        &[
            "clone",
            "-q",
            remote.to_str().unwrap(),
            stale.to_str().unwrap(),
        ],
    );
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
