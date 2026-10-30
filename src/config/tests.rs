use super::*;
use crate::sys::RealSys;
use std::fs;

/// A fixed working directory, so `normalize` results match on every machine.
fn cwd() -> &'static Path {
    Path::new("/work")
}

/// A system with a fixed read result and a write that can be set to fail, so each test
/// picks the seam behaviour it needs. The path is not on disk, so a passing read proves
/// the call went through the seam.
struct FakeSys {
    read: io::Result<Vec<u8>>,
    write_fails: bool,
}

impl Sys for FakeSys {
    /// Serve the fixed result; a stored error is rebuilt, as `io::Error` is not cloneable.
    fn read(&self, _path: &Path) -> io::Result<Vec<u8>> {
        match &self.read {
            Ok(bytes) => Ok(bytes.clone()),
            Err(err) => Err(io::Error::new(err.kind(), err.to_string())),
        }
    }

    /// Fail only when the test set the flag, so a read failure is never a write failure in disguise.
    fn write(&self, _path: &Path, _contents: &[u8]) -> io::Result<()> {
        if self.write_fails {
            Err(io::Error::new(io::ErrorKind::PermissionDenied, "denied"))
        } else {
            Ok(())
        }
    }
}

/// Guards that `save` refuses a broken file; overwriting it would lose the user's content.
#[test]
fn save_fails_on_an_unparseable_existing_file() {
    let sys = FakeSys {
        read: Ok(b"repositories = [\n".to_vec()),
        write_fails: false,
    };
    assert!(save(&sys, Path::new("/x/config.toml"), &[]).is_err());
}

/// Guards that saving no entries into a file without the key writes an empty array.
#[test]
fn save_writes_an_empty_array_when_the_key_is_missing() {
    let (text, kept) = run_save("other = 1\n", &[]);
    assert!(text.contains("repositories = []"), "{text}");
    assert!(kept.is_empty());
}

/// Guards that a denied read stops `save`, names the file, and the file's other keys survive.
#[test]
fn save_does_not_overwrite_a_file_it_cannot_read() {
    let sys = FakeSys {
        read: Err(io::Error::new(io::ErrorKind::PermissionDenied, "denied")),
        write_fails: false,
    };
    let err = save(&sys, Path::new("/x/config.toml"), &[]).unwrap_err();
    assert!(
        err.to_string().contains("cannot read /x/config.toml"),
        "{err}"
    );
}

/// Guards that `save` refuses a wrong-typed `repositories`, like `load`; replacing it
/// would silently drop its content, and nothing is written.
#[test]
fn save_rejects_a_wrong_typed_repositories() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("config.toml");
    for seed in ["[repositories]\nx = 1\n", "repositories = \"/a\"\n"] {
        fs::write(&file, seed).unwrap();
        let err = save(&RealSys, &file, &["/b".to_owned()]).unwrap_err();
        assert!(
            err.to_string()
                .contains("`repositories` is not an array of strings"),
            "{err}"
        );
        assert_eq!(fs::read_to_string(&file).unwrap(), seed);
    }
}

/// Guards that `load` and `save` go through `Sys`, so tests can swap in a fake, and that
/// a failed write names the file it could not write.
#[test]
fn load_reads_through_the_seam_and_a_failed_write_is_an_error() {
    let sys = FakeSys {
        read: Ok(b"repositories = [\"/a\"]\n".to_vec()),
        write_fails: true,
    };
    assert_eq!(load(&sys, Path::new("/x/config.toml")).unwrap(), ["/a"]);
    let err = save(&sys, Path::new("/x/config.toml"), &["/a".to_owned()]).unwrap_err();
    assert!(
        err.to_string().contains("cannot write /x/config.toml"),
        "{err}"
    );
}

/// Guards that a missing config means no repositories; a first run is not an error.
#[test]
fn load_of_a_missing_file_is_empty() {
    assert!(
        load(&RealSys, Path::new("/nonexistent/config.toml"))
            .unwrap()
            .is_empty()
    );
}

/// Guards that a file without `repositories` loads as empty, not as an error.
#[test]
fn load_without_the_key_is_empty() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("config.toml");
    fs::write(&file, "other = 1\n").unwrap();
    assert!(load(&RealSys, &file).unwrap().is_empty());
}

/// Guards that bad TOML, wrong types and invalid UTF-8 fail; none reads as an empty list.
#[test]
fn load_rejects_an_unparseable_file() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("config.toml");
    for text in [
        "repositories = [\n",
        "repositories = \"/a\"\n",
        "repositories = [1]\n",
    ] {
        fs::write(&file, text).unwrap();
        assert!(load(&RealSys, &file).is_err(), "{text}");
    }
    fs::write(&file, [0xff, 0xfe]).unwrap();
    assert!(load(&RealSys, &file).is_err());
}

/// Guards the layout of a new config file and that it loads back the same entries.
#[test]
fn save_creates_the_one_key_document() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("deep/config.toml");
    let entries = vec!["/a".to_owned(), "/b".to_owned()];
    save(&RealSys, &file, &entries).unwrap();
    assert_eq!(
        fs::read_to_string(&file).unwrap(),
        "repositories = [\n    \"/a\",\n    \"/b\"\n]\n"
    );
    assert_eq!(load(&RealSys, &file).unwrap(), entries);
}

fn run_save(seed: &str, entries: &[&str]) -> (String, Vec<String>) {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("config.toml");
    fs::write(&file, seed).unwrap();
    let entries: Vec<String> = entries.iter().map(|entry| entry.to_string()).collect();
    save(&RealSys, &file, &entries).unwrap();
    (
        fs::read_to_string(&file).unwrap(),
        load(&RealSys, &file).unwrap(),
    )
}

/// Guards that deleting or appending entries keeps a kept entry's comments; a removed
/// entry's own comments go with it, as they are part of its decor.
#[test]
fn save_keeps_comments_inside_the_array() {
    let seed = "repositories = [\n    # the main one\n    \"/a\", # trailing note\n    \"/b\",\n    \"/c\",\n]\n";
    // Delete the last entry and append one: every comment stays.
    let (text, loaded) = run_save(seed, &["/a", "/b", "/d"]);
    assert_eq!(text.matches("# the main one").count(), 1, "{text}");
    assert!(text.contains("# trailing note"), "{text}");
    assert_eq!(loaded, ["/a", "/b", "/d"]);

    // Delete the middle entry: the note trailing /a is stored on /b, so it goes with /b.
    let (text, loaded) = run_save(seed, &["/a", "/c"]);
    assert_eq!(text.matches("# the main one").count(), 1, "{text}");
    assert!(!text.contains("# trailing note"), "{text}");
    assert_eq!(loaded, ["/a", "/c"]);

    // Delete the first entry: its leading comment goes with it; /b's note stays.
    let (text, loaded) = run_save(seed, &["/b", "/c"]);
    assert!(!text.contains("# the main one"), "{text}");
    assert_eq!(text.matches("# trailing note").count(), 1, "{text}");
    assert_eq!(loaded, ["/b", "/c"]);
}

/// Guards comments in awkward spots around removed entries: the array's trailing text
/// stays, a removed entry's own comments go with it, and an append keeps the last
/// entry's trailing comment in place.
#[test]
fn save_keeps_comments_around_removed_entries() {
    // A comment after the last entry's comma sits in the trailing text and stays.
    let (text, loaded) = run_save(
        "repositories = [\n    \"/a\",\n    \"/b\", # tail note\n]\n",
        &["/a"],
    );
    assert_eq!(text.matches("# tail note").count(), 1, "{text}");
    assert_eq!(loaded, ["/a"]);

    // A comment in a removed entry's suffix goes with it.
    let (text, loaded) = run_save(
        "repositories = [\n    \"/a\" # mid note\n    ,\n    \"/b\",\n]\n",
        &["/b"],
    );
    assert!(!text.contains("# mid note"), "{text}");
    assert_eq!(loaded, ["/b"]);

    // The last entry has a comment above it (its own prefix: it goes) and one after its
    // comma (the array's trailing text: it stays).
    let (text, loaded) = run_save(
        "repositories = [\n    \"/a\",\n    # above third\n    \"/c\", # trailing gamma\n]\n",
        &["/a"],
    );
    assert!(!text.contains("# above third"), "{text}");
    assert_eq!(text.matches("# trailing gamma").count(), 1, "{text}");
    assert_eq!(loaded, ["/a"]);

    // Appending must not move the last entry's trailing comment.
    let (text, loaded) = run_save(
        "repositories = [\n    \"/a\", # trailing a\n]\n",
        &["/a", "/b"],
    );
    assert_eq!(text.matches("# trailing a").count(), 1, "{text}");
    assert_eq!(loaded, ["/a", "/b"]);
}

/// Guards that removing the last or only entry leaves a valid, unglued file; the removed
/// entry's own comments go with it.
#[test]
fn save_keeps_comments_when_the_last_entry_goes() {
    // No trailing comma, a comment above the removed entry: `]` keeps its own line.
    let (text, loaded) = run_save(
        "repositories = [\n    \"/a\",\n    # above b\n    \"/b\"\n]\n",
        &["/a"],
    );
    assert_eq!(text, "repositories = [\n    \"/a\"\n]\n");
    assert_eq!(loaded, ["/a"]);

    // A comment on its own line before the closing bracket is part of the last
    // entry's suffix when there is no trailing comma, so it goes with that entry.
    let (text, loaded) = run_save(
        "repositories = [\n    \"/a\",\n    \"/b\"\n    # before bracket\n]\n",
        &["/a"],
    );
    assert!(!text.contains("# before bracket"), "{text}");
    assert_eq!(loaded, ["/a"]);

    // The only entry goes; its comment goes with it and the array is empty.
    let (text, loaded) = run_save(
        "repositories = [\n    # above only\n    \"/only\"\n]\n",
        &[],
    );
    assert_eq!(text, "repositories = []\n");
    assert!(loaded.is_empty());
}

/// Guards that duplicates match in order, so the right copy is removed.
#[test]
fn save_removes_the_first_of_interleaved_duplicates() {
    let (_, loaded) = run_save("repositories = [\"/a\", \"/b\", \"/a\"]\n", &["/b", "/a"]);
    assert_eq!(loaded, ["/b", "/a"]);
}

/// Guards that removing a value with a trailing comment from an inline array stays
/// valid: the comment is part of the removed value's decor and goes with it.
#[test]
fn save_removes_an_inline_trailing_comment_with_its_entry() {
    let (text, kept) = run_save("repositories = [\"/a\", \"/b\" # note\n]\n", &["/a"]);
    assert_eq!(text, "repositories = [\"/a\"]\n");
    assert_eq!(kept, ["/a"]);
}

/// Guards that appending to the layout `save` itself writes (no trailing comma on the
/// last entry) keeps one entry per line, and a second append does not degrade it.
#[test]
fn save_appends_cleanly_to_its_own_layout() {
    let seed = "repositories = [\n    \"/a\",\n    \"/b\"\n]\n";
    let (text, loaded) = run_save(seed, &["/a", "/b", "/c"]);
    assert_eq!(
        text,
        "repositories = [\n    \"/a\",\n    \"/b\",\n    \"/c\"\n]\n"
    );
    assert_eq!(loaded, ["/a", "/b", "/c"]);
    let (text, loaded) = run_save(&text, &["/a", "/b", "/c", "/d"]);
    assert_eq!(
        text,
        "repositories = [\n    \"/a\",\n    \"/b\",\n    \"/c\",\n    \"/d\"\n]\n"
    );
    assert_eq!(loaded, ["/a", "/b", "/c", "/d"]);
}

/// Guards that a delete and an append in one save keep the one-entry-per-line layout.
#[test]
fn save_appends_cleanly_after_a_delete() {
    let seed = "repositories = [\n    \"/a\",\n    \"/b\"\n]\n";
    let (text, loaded) = run_save(seed, &["/a", "/c"]);
    assert_eq!(text, "repositories = [\n    \"/a\",\n    \"/c\"\n]\n");
    assert_eq!(loaded, ["/a", "/c"]);
}

/// Guards that adding to an empty array keeps its lone comment.
#[test]
fn save_keeps_the_comment_of_an_empty_array() {
    for seed in [
        "repositories = [\n    # only note\n]\n",
        "repositories = [ # only note\n]\n",
    ] {
        let (text, loaded) = run_save(seed, &["/a"]);
        assert!(text.contains("# only note"), "{text}");
        assert_eq!(loaded, ["/a"]);
    }
}

/// Guards that `save` always ends the file with a newline, even when the source lacked one.
#[test]
fn save_adds_the_trailing_newline_an_existing_file_lacks() {
    let (text, loaded) = run_save("repositories = [\"/a\"]\nother = 1", &["/a"]);
    assert!(text.ends_with('\n'), "{text}");
    assert!(text.contains("other = 1"), "{text}");
    assert_eq!(loaded, ["/a"]);
    // A file ending in a bare comment renders without a newline; the guard in save adds it.
    let (text, _) = run_save("repositories = [\"/a\"]\n# end comment", &["/a"]);
    assert!(text.ends_with('\n'), "{text}");
    assert!(text.contains("# end comment"), "{text}");
}

/// Guards that a comment block after the last entry stays on append.
#[test]
fn save_keeps_a_stacked_trailing_comment_on_append() {
    let (text, loaded) = run_save(
        "repositories = [\n    \"/a\", # note one\n    # note two\n]\n",
        &["/a", "/b"],
    );
    assert!(text.contains("# note one"), "{text}");
    assert!(text.contains("# note two"), "{text}");
    assert_eq!(loaded, ["/a", "/b"]);
}

/// Guards that `save` edits in place; comments and other keys survive.
#[test]
fn save_keeps_comments_and_other_keys() {
    let (text, loaded) = run_save(
        "# a note\nother = 1\nrepositories = [\"/a\"]\n",
        &["/a", "/b"],
    );
    assert!(text.contains("# a note"), "{text}");
    assert!(text.contains("other = 1"), "{text}");
    assert_eq!(loaded, ["/a", "/b"]);
}

/// Guards that relative and empty paths are stored against the working directory.
#[test]
fn normalize_makes_relative_paths_absolute() {
    assert_eq!(normalize("a/b", cwd()), "/work/a/b");
    assert_eq!(normalize("a/", cwd()), "/work/a");
    assert_eq!(normalize("", cwd()), "/work");
}

/// Guards that normalising is lexical, so it works on paths that do not exist yet.
#[test]
fn normalize_is_lexical() {
    assert_eq!(normalize("/a//b/./c/../d", cwd()), "/a/b/d");
    assert_eq!(normalize("/", cwd()), "/");
    assert_eq!(normalize("/../a", cwd()), "/a");
}

/// Guards that a mid-path `~` is an ordinary component that `..` can remove.
#[test]
fn normalize_resolves_a_mid_path_tilde() {
    assert_eq!(normalize("a/~/../b", cwd()), "/work/a/b");
}

/// Guards that a leading `~` survives, so stored paths follow the home directory.
#[test]
fn normalize_keeps_a_leading_tilde() {
    assert_eq!(normalize("~", cwd()), "~");
    assert_eq!(normalize("~/x/../y", cwd()), "~/y");
    assert_eq!(normalize("~/..", cwd()), "~");
}

/// Guards the config path order: the `-C` flag, the platform directory, then `.config`.
#[test]
fn file_path_uses_the_flag_then_the_platform_directory() {
    assert_eq!(
        file_path(None, Some(Path::new("/cfg"))),
        PathBuf::from("/cfg/9up/config.toml")
    );
    // No platform directory at all: the relative fallback.
    assert_eq!(
        file_path(None, None),
        PathBuf::from(".config/9up/config.toml")
    );
    assert_eq!(
        file_path(Some("mine.toml"), None),
        PathBuf::from("mine.toml")
    );
    // An empty flag means the default path, like no flag.
    assert_eq!(
        file_path(Some(""), Some(Path::new("/cfg"))),
        PathBuf::from("/cfg/9up/config.toml")
    );
}

/// Guards the append break move on a single-line array: the last entry's suffix holds
/// the break and the array trailing is empty, so the break must move to the trailing
/// text or the appended comma strands on its own line.
#[test]
fn save_appends_cleanly_when_the_break_sits_in_the_last_suffix() {
    let (text, loaded) = run_save("repositories = [\"/a\", \"/b\"\n]\n", &["/a", "/b", "/c"]);
    assert_eq!(text, "repositories = [\"/a\", \"/b\", \"/c\"\n]\n");
    assert_eq!(loaded, ["/a", "/b", "/c"]);
}

/// Guards the same move when the array already has trailing text: the suffix is
/// cleared, but the trailing text is the array's own and stays untouched.
#[test]
fn save_appends_with_trailing_text_present() {
    let (text, loaded) = run_save(
        "repositories = [\n    \"/a\",\n    \"/b\"\n,\n]\n",
        &["/a", "/b", "/c"],
    );
    assert_eq!(loaded, ["/a", "/b", "/c"]);
    assert!(text.contains("\"/b\",\n"), "{text}");
    assert!(!text.contains("\n,"), "{text}");
}

/// Guards the break-move exception: with a trailing comment on the last entry and
/// no trailing comma, the break move must not run (the comment would swallow the
/// separator), so the appended comma lands on its own line. The file stays valid
/// and the comment survives.
#[test]
fn save_appends_past_a_comment_in_the_last_suffix() {
    let (text, loaded) = run_save(
        "repositories = [\n    \"/a\",\n    \"/b\" # note no comma\n]\n",
        &["/a", "/b", "/c"],
    );
    assert_eq!(
        text,
        "repositories = [\n    \"/a\",\n    \"/b\" # note no comma\n,\n    \"/c\"\n]\n"
    );
    assert_eq!(loaded, ["/a", "/b", "/c"]);
}
