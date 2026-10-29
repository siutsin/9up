use super::*;
use crate::sys::RealSys;
use std::fs;

fn cwd() -> &'static Path {
    Path::new("/work")
}

/// A system with one in-memory file and a failing write. The
/// file's path does not exist on disk, so a passing read proves
/// the call went through the seam.
struct MemorySys;

impl Sys for MemorySys {
    fn read(&self, _path: &Path) -> io::Result<Vec<u8>> {
        Ok(b"repositories = [\"/a\"]\n".to_vec())
    }

    fn write(&self, _path: &Path, _contents: &[u8]) -> io::Result<()> {
        Err(io::Error::new(io::ErrorKind::PermissionDenied, "denied"))
    }
}

/// A system whose read is denied and whose write would succeed:
/// save must fail rather than overwrite the file unseen.
struct DenyReadSys;

impl Sys for DenyReadSys {
    fn read(&self, _path: &Path) -> io::Result<Vec<u8>> {
        Err(io::Error::new(io::ErrorKind::PermissionDenied, "denied"))
    }

    fn write(&self, _path: &Path, _contents: &[u8]) -> io::Result<()> {
        Ok(())
    }
}

/// A system whose file holds text that is not valid TOML.
struct GarbledSys;

impl Sys for GarbledSys {
    fn read(&self, _path: &Path) -> io::Result<Vec<u8>> {
        Ok(b"repositories = [\n".to_vec())
    }

    fn write(&self, _path: &Path, _contents: &[u8]) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn save_fails_on_an_unparseable_existing_file() {
    assert!(save(&GarbledSys, Path::new("/x/config.toml"), &[]).is_err());
}

#[test]
fn join_reuses_the_indent_of_the_comment_head() {
    assert_eq!(join("  # note", "rest"), "  # note\n  rest");
    assert_eq!(join("# note", "rest"), "# note\n    rest");
}

#[test]
fn save_moves_a_removed_comment_back_to_the_previous_entry() {
    let (text, kept) = run_save(
        "repositories = [\n    \"/a\",\n    \"/b\", # note\n]\n",
        &["/a"],
    );
    assert_eq!(kept, ["/a"]);
    assert_eq!(text, "repositories = [\n    \"/a\", # note\n]\n");
}

#[test]
fn save_does_not_overwrite_a_file_it_cannot_read() {
    assert!(save(&DenyReadSys, Path::new("/x/config.toml"), &[]).is_err());
}

#[test]
fn the_deny_read_fixture_would_let_a_write_through() {
    // The fixture's point: only its read fails, so the test above
    // proves save stops at the read.
    assert!(DenyReadSys.write(Path::new("/x"), b"").is_ok());
}

#[test]
fn load_reads_through_the_seam_and_a_failed_write_is_an_error() {
    assert_eq!(
        load(&MemorySys, Path::new("/x/config.toml")).unwrap(),
        ["/a"]
    );
    assert!(save(&MemorySys, Path::new("/x/config.toml"), &["/a".to_owned()]).is_err());
}

#[test]
fn load_of_a_missing_file_is_empty() {
    assert!(
        load(&RealSys, Path::new("/nonexistent/config.toml"))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn load_without_the_key_is_empty() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("config.toml");
    fs::write(&file, "other = 1\n").unwrap();
    assert!(load(&RealSys, &file).unwrap().is_empty());
}

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

/// Save `entries` into a fresh file holding `seed`, and return the
/// resulting text and the entries a load reads back.
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

#[test]
fn save_keeps_comments_inside_the_array() {
    let seed = "repositories = [\n    # the main one\n    \"/a\", # trailing note\n    \"/b\",\n    \"/c\",\n]\n";
    // Delete the last entry and append one: every comment stays.
    let (text, loaded) = run_save(seed, &["/a", "/b", "/d"]);
    assert_eq!(text.matches("# the main one").count(), 1, "{text}");
    assert!(text.contains("# trailing note"), "{text}");
    assert_eq!(loaded, ["/a", "/b", "/d"]);

    // Delete the middle entry: the note trailing /a is stored on
    // /b, and must move rather than vanish.
    let (text, loaded) = run_save(seed, &["/a", "/c"]);
    assert_eq!(text.matches("# trailing note").count(), 1, "{text}");
    assert_eq!(loaded, ["/a", "/c"]);

    // Delete the first entry: its leading comment stays in place.
    let (text, loaded) = run_save(seed, &["/b", "/c"]);
    assert_eq!(text.matches("# the main one").count(), 1, "{text}");
    assert_eq!(loaded, ["/b", "/c"]);
}

#[test]
fn save_keeps_comments_around_removed_entries() {
    let cases: [(&str, &[&str], &str); 4] = [
        // A comment after the last entry, before its missing comma.
        (
            "repositories = [\n    \"/a\",\n    \"/b\" # tail note\n]\n",
            &["/a"],
            "# tail note",
        ),
        // A comment after a middle entry, its comma on the next line.
        (
            "repositories = [\n    \"/a\" # mid note\n    ,\n    \"/b\",\n]\n",
            &["/b"],
            "# mid note",
        ),
        // The last entry has a comment above it and one after it.
        (
            "repositories = [\n    \"/a\",\n    # above third\n    \"/c\", # trailing gamma\n]\n",
            &["/a"],
            "# above third",
        ),
        // Appending must not move the last entry's trailing comment.
        (
            "repositories = [\n    \"/a\", # trailing a\n]\n",
            &["/a", "/b"],
            "# trailing a",
        ),
    ];
    for (seed, entries, comment) in cases {
        let (text, loaded) = run_save(seed, entries);
        assert_eq!(text.matches(comment).count(), 1, "{text}");
        assert_eq!(loaded, entries, "{text}");
    }
    // The third case carries a second comment; check it separately.
    let (text, _) = run_save(
        "repositories = [\n    \"/a\",\n    # above third\n    \"/c\", # trailing gamma\n]\n",
        &["/a"],
    );
    assert_eq!(text.matches("# trailing gamma").count(), 1, "{text}");
    assert!(
        text.contains("# above third\n    # trailing gamma"),
        "{text}"
    );
}

#[test]
fn save_keeps_comments_when_the_last_entry_goes() {
    // No trailing comma, a comment above the removed entry.
    let (text, loaded) = run_save(
        "repositories = [\n    \"/a\",\n    # above b\n    \"/b\"\n]\n",
        &["/a"],
    );
    assert!(text.contains("# above b"), "{text}");
    assert_eq!(loaded, ["/a"]);

    // A comment on its own line before the closing bracket.
    let (text, loaded) = run_save(
        "repositories = [\n    \"/a\",\n    \"/b\"\n    # before bracket\n]\n",
        &["/a"],
    );
    assert!(text.contains("# before bracket"), "{text}");
    assert_eq!(loaded, ["/a"]);

    // The only entry goes; its comment stays in the empty array.
    let (text, loaded) = run_save(
        "repositories = [\n    # above only\n    \"/only\"\n]\n",
        &[],
    );
    assert!(text.contains("# above only"), "{text}");
    assert!(loaded.is_empty());
}

#[test]
fn save_removes_the_first_of_interleaved_duplicates() {
    let (_, loaded) = run_save("repositories = [\"/a\", \"/b\", \"/a\"]\n", &["/b", "/a"]);
    assert_eq!(loaded, ["/b", "/a"]);
}

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

#[test]
fn save_adds_the_trailing_newline_an_existing_file_lacks() {
    let (text, loaded) = run_save("repositories = [\"/a\"]\nother = 1", &["/a"]);
    assert!(text.ends_with('\n'), "{text}");
    assert!(text.contains("other = 1"), "{text}");
    assert_eq!(loaded, ["/a"]);
    // A file ending in a bare comment renders without a newline;
    // the guard in save adds it.
    let (text, _) = run_save("repositories = [\"/a\"]\n# end comment", &["/a"]);
    assert!(text.ends_with('\n'), "{text}");
    assert!(text.contains("# end comment"), "{text}");
}

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

#[test]
fn normalize_makes_relative_paths_absolute() {
    assert_eq!(normalize("a/b", cwd()), "/work/a/b");
    assert_eq!(normalize("a/", cwd()), "/work/a");
    assert_eq!(normalize("", cwd()), "/work");
}

#[test]
fn normalize_is_lexical() {
    assert_eq!(normalize("/a//b/./c/../d", cwd()), "/a/b/d");
    assert_eq!(normalize("/", cwd()), "/");
    assert_eq!(normalize("/../a", cwd()), "/a");
}

#[test]
fn normalize_resolves_a_mid_path_tilde() {
    assert_eq!(normalize("a/~/../b", cwd()), "/work/a/b");
}

#[test]
fn normalize_keeps_a_leading_tilde() {
    assert_eq!(normalize("~", cwd()), "~");
    assert_eq!(normalize("~/x/../y", cwd()), "~/y");
    assert_eq!(normalize("~/..", cwd()), "~");
}

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
