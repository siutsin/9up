use std::fs;
use std::io;
use std::path::Path;

/// The bookmark file's entries, in file order.
///
/// A missing or unreadable file means no bookmarks. Lines are trimmed and
/// empty lines dropped. Invalid UTF-8 becomes the replacement character,
/// and the next save stores the replaced text.
pub fn load(path: &Path) -> Vec<String> {
    let Ok(bytes) = fs::read(path) else {
        return Vec::new();
    };
    String::from_utf8_lossy(&bytes)
        .split('\n')
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Write the entries, one per line, with no final newline.
pub fn save(path: &Path, entries: &[String]) -> io::Result<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, entries.join("\n"))
}

pub fn has(entries: &[String], normalized: &str) -> bool {
    entries.iter().any(|entry| entry == normalized)
}

/// The stored form of a path: absolute, normalised with no filesystem
/// access. A leading `~` stays, and only the rest is normalised.
pub fn normalize(path: &str, cwd: &Path) -> String {
    if path.is_empty() {
        return cwd.to_string_lossy().into_owned();
    }
    let full = if path == "~" || path.starts_with("~/") || Path::new(path).is_absolute() {
        path.to_owned()
    } else {
        cwd.join(path).to_string_lossy().into_owned()
    };
    let absolute = full.starts_with('/');
    let mut parts: Vec<&str> = Vec::new();
    for part in full.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if parts.last().is_some_and(|last| *last != "~") {
                    parts.pop();
                }
            }
            part => parts.push(part),
        }
    }
    let joined = parts.join("/");
    if absolute {
        format!("/{joined}")
    } else {
        joined
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cwd() -> &'static Path {
        Path::new("/work")
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
    fn normalize_keeps_a_leading_tilde() {
        assert_eq!(normalize("~", cwd()), "~");
        assert_eq!(normalize("~/x/../y", cwd()), "~/y");
        assert_eq!(normalize("~/..", cwd()), "~");
    }

    #[test]
    fn load_trims_and_drops_empty_lines() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("bookmarks");
        fs::write(&file, " /a \n\n# note\n/b\n").unwrap();
        assert_eq!(load(&file), ["/a", "# note", "/b"]);
    }

    #[test]
    fn load_of_a_missing_file_is_empty() {
        assert!(load(Path::new("/nonexistent/bookmarks")).is_empty());
    }

    #[test]
    fn save_and_load_round_trip_without_a_final_newline() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("deep/bookmarks");
        let entries = vec!["/a".to_owned(), "/b".to_owned()];
        save(&file, &entries).unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap(), "/a\n/b");
        assert_eq!(load(&file), entries);
    }
}
