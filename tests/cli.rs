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
