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
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?} failed");
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
