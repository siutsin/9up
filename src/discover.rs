use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

use glob::MatchOptions;

pub struct Repo {
    pub name: String,
    pub path: PathBuf,
}

pub enum Resolved {
    Repos(Vec<Repo>),
    Comment(String),
    PathError(String),
}

/// Expand a leading `~` or `~/` to the home directory. With no home
/// directory the path is returned as given.
pub fn expand_home(path: &str, home: Option<&Path>) -> PathBuf {
    let Some(home) = home else {
        return PathBuf::from(path);
    };
    if path == "~" {
        return home.to_path_buf();
    }
    if let Some(rest) = path.strip_prefix("~/") {
        return home.join(rest);
    }
    PathBuf::from(path)
}

/// The existing paths a glob pattern matches. A bad pattern matches
/// nothing. A broken symbolic link counts as existing.
pub fn glob_paths(pattern: &str) -> Vec<PathBuf> {
    let options = MatchOptions {
        require_literal_leading_dot: true,
        ..MatchOptions::default()
    };
    let Ok(paths) = glob::glob_with(pattern, options) else {
        return Vec::new();
    };
    paths
        .filter_map(Result::ok)
        .filter(|path| path.symlink_metadata().is_ok())
        .collect()
}

/// Whether an expanded entry still names something on disk: a
/// directory, or a glob with at least one match. Going through the glob
/// also covers plain files and broken symbolic links, which `is_dir`
/// alone misses. This is the one definition of "exists" for a path.
pub fn path_is_live(expanded: &Path) -> bool {
    expanded.is_dir() || !glob_paths(&expanded.display().to_string()).is_empty()
}

/// Resolve one input (a command-line path or a configured path) to the
/// repositories under it, a comment, or a path error.
pub fn resolve(input: &str, cwd: &Path, home: Option<&Path>, depth: i32) -> Resolved {
    if input.is_empty() {
        return Resolved::PathError("doesn't exist.".to_owned());
    }
    let expanded = expand_home(input, home);
    let display = expanded.display().to_string();
    // All filesystem work goes through the absolute form. `expanded`
    // stays as typed for the path errors, which name the given path.
    let fs_path = cwd.join(&expanded);
    if is_repo(&fs_path) {
        return Resolved::Repos(vec![Repo {
            name: last_component(&fs_path),
            path: fs_path,
        }]);
    }
    match fs_path.symlink_metadata() {
        Ok(_) => {
            let Ok(metadata) = fs::metadata(&fs_path) else {
                // A broken symbolic link: it exists, with no repositories.
                return Resolved::Repos(Vec::new());
            };
            if !metadata.is_dir() || depth == 0 {
                return Resolved::PathError(format!("{display} isn't a repository."));
            }
            let mut repos = search(&fs_path, depth, &fs_path);
            repos.sort_by(|a, b| a.name.cmp(&b.name));
            Resolved::Repos(repos)
        }
        Err(_) => {
            if let Some(comment) = comment_text(input) {
                return Resolved::Comment(comment);
            }
            if !has_glob_chars(input) {
                return Resolved::PathError(format!("{display} doesn't exist."));
            }
            let pattern = fs_path.display().to_string();
            let matches = glob_paths(&pattern);
            if matches.is_empty() {
                return Resolved::PathError(format!("{display} doesn't exist."));
            }
            let prefix = static_prefix(&pattern);
            let mut repos = Vec::new();
            for matched in &matches {
                repos.extend(search(matched, depth, &prefix));
            }
            // A `**` pattern matches both an intermediate directory and
            // the repository below it, so the same path can appear twice.
            let mut seen = std::collections::HashSet::new();
            repos.retain(|repo| seen.insert(repo.path.clone()));
            repos.sort_by(|a, b| a.name.cmp(&b.name));
            Resolved::Repos(repos)
        }
    }
}

/// The comment text of a missing path whose first non-space character is
/// `#`, or None. An empty text is Some(""): it prints nothing.
pub fn comment_text(input: &str) -> Option<String> {
    let trimmed = input.trim_start_matches(' ');
    if !trimmed.starts_with('#') {
        return None;
    }
    let text = trimmed.trim_start_matches('#').trim();
    Some(text.to_owned())
}

fn has_glob_chars(input: &str) -> bool {
    input.chars().any(|c| matches!(c, '*' | '?' | '['))
}

/// A git directory: a HEAD file, a refs directory, and an objects
/// directory or a commondir file (a linked worktree's admin directory).
fn is_git_dir(dir: &Path) -> bool {
    // A linked worktree's admin directory holds HEAD and commondir but
    // no refs directory (real git creates none), so commondir alone is
    // enough; a normal or bare directory pairs refs with objects.
    dir.join("HEAD").is_file()
        && (dir.join("commondir").is_file()
            || (dir.join("refs").is_dir() && dir.join("objects").is_dir()))
}

/// Whether a path is a repository, by filesystem test only. Discovery
/// runs no git.
fn is_repo(path: &Path) -> bool {
    if is_git_dir(path) {
        return true;
    }
    let dot_git = path.join(".git");
    if is_git_dir(&dot_git) {
        return true;
    }
    // A .git file (a linked worktree or a submodule) points at the git
    // directory with a `gitdir:` line.
    if dot_git.is_file()
        && let Ok(content) = fs::read_to_string(&dot_git)
        && let Some(target) = content
            .lines()
            .next()
            .and_then(|line| line.strip_prefix("gitdir: "))
    {
        let target = target.trim();
        return is_git_dir(&path.join(target));
    }
    false
}

/// The repositories at and below a directory, down to `depth` levels
/// below it (a negative depth has no limit). Names are relative to
/// `header`, except a repository at the directory itself, which takes
/// its last component.
fn search(dir: &Path, depth: i32, header: &Path) -> Vec<Repo> {
    if is_repo(dir) {
        return vec![Repo {
            name: name_for(dir, header),
            path: dir.to_path_buf(),
        }];
    }
    let walk = WalkDir::new(dir)
        .follow_links(true)
        .min_depth(1)
        // A negative depth becomes the unlimited case.
        .max_depth(depth.try_into().unwrap_or(usize::MAX));
    let mut repos = Vec::new();
    let mut iter = walk.into_iter();
    while let Some(entry) = iter.next() {
        // An unreadable directory (or a symbolic-link loop, which
        // walkdir reports as an error) is skipped.
        let Ok(entry) = entry else { continue };
        if !entry.file_type().is_dir() {
            continue;
        }
        let path = entry.path();
        if path.file_name().is_some_and(|name| name == ".git") {
            iter.skip_current_dir();
            continue;
        }
        if is_repo(path) {
            repos.push(Repo {
                name: name_for(path, header),
                path: entry.into_path(),
            });
            iter.skip_current_dir(); // A found repository is not descended into.
        }
    }
    repos
}

fn name_for(repo: &Path, header: &Path) -> String {
    if repo == header {
        return last_component(repo);
    }
    match repo.strip_prefix(header) {
        Ok(relative) => relative
            .components()
            .map(|part| part.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/"),
        Err(_) => last_component(repo),
    }
}

fn last_component(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// The part of a glob pattern before its first wildcard component.
fn static_prefix(pattern: &str) -> PathBuf {
    let mut prefix = PathBuf::new();
    for part in Path::new(pattern).components() {
        let text = part.as_os_str().to_string_lossy();
        if text.chars().any(|c| matches!(c, '*' | '?' | '[')) {
            break;
        }
        prefix.push(part.as_os_str());
    }
    prefix
}

/// A minimal fake repository for tests across the crate: the `.git`
/// layout `is_git_dir` accepts, without running git.
#[cfg(test)]
pub(crate) fn fake_repo(dir: &Path) {
    let git = dir.join(".git");
    fs::create_dir_all(git.join("refs")).unwrap();
    fs::create_dir_all(git.join("objects")).unwrap();
    fs::write(git.join("HEAD"), "ref: refs/heads/main\n").unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The message of a path error, or a panic when the resolve was
    /// something else.
    fn expect_path_error(resolved: &Resolved) -> &str {
        match resolved {
            Resolved::PathError(message) => message,
            _ => panic!("not an error"),
        }
    }

    fn resolved_names(resolved: &Resolved) -> Vec<&str> {
        match resolved {
            Resolved::Repos(repos) => repos.iter().map(|repo| repo.name.as_str()).collect(),
            _ => panic!("not repos"),
        }
    }

    #[test]
    fn expand_home_uses_the_given_home() {
        assert_eq!(
            expand_home("~/x", Some(Path::new("/home/a"))),
            PathBuf::from("/home/a/x")
        );
        assert_eq!(expand_home("~/x", None), PathBuf::from("~/x"));
        assert_eq!(
            expand_home("~", Some(Path::new("/home/a"))),
            PathBuf::from("/home/a")
        );
    }

    #[test]
    fn expand_home_leaves_a_plain_path_alone() {
        assert_eq!(
            expand_home("plain/dir", Some(Path::new("/home/a"))),
            PathBuf::from("plain/dir")
        );
    }

    #[test]
    fn last_component_falls_back_to_the_whole_path() {
        assert_eq!(last_component(Path::new("/")), "/");
        assert_eq!(last_component(Path::new("/a/b")), "b");
    }

    #[test]
    #[should_panic(expected = "not repos")]
    fn resolved_names_rejects_a_non_repository_result() {
        resolved_names(&Resolved::Comment("x".to_owned()));
    }

    #[test]
    fn a_repository_outside_the_header_takes_its_last_component() {
        assert_eq!(
            name_for(Path::new("/elsewhere/repo"), Path::new("/base")),
            "repo"
        );
    }

    #[test]
    fn a_repository_at_the_path() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        fs::create_dir_all(&repo).unwrap();
        fake_repo(&repo);
        let resolved = resolve(repo.to_str().unwrap(), dir.path(), None, 3);
        assert_eq!(resolved_names(&resolved), ["repo"]);
    }

    #[test]
    fn repositories_below_the_path_respect_depth() {
        let dir = tempfile::tempdir().unwrap();
        let deep = dir.path().join("a/b/repo");
        fs::create_dir_all(&deep).unwrap();
        fake_repo(&deep);
        let base = dir.path().to_str().unwrap().to_owned();
        assert!(resolved_names(&resolve(&base, dir.path(), None, 1)).is_empty());
        assert!(resolved_names(&resolve(&base, dir.path(), None, 2)).is_empty());
        assert_eq!(
            resolved_names(&resolve(&base, dir.path(), None, 3)),
            ["a/b/repo"]
        );
        assert_eq!(
            resolved_names(&resolve(&base, dir.path(), None, -1)),
            ["a/b/repo"]
        );
    }

    #[test]
    fn relative_inputs_name_repositories_by_relative_path() {
        let dir = tempfile::tempdir().unwrap();
        let deep = dir.path().join("base/a/repo");
        fs::create_dir_all(&deep).unwrap();
        fake_repo(&deep);
        assert_eq!(
            resolved_names(&resolve("base", dir.path(), None, 3)),
            ["a/repo"]
        );
        let solo = dir.path().join("solo");
        fs::create_dir_all(&solo).unwrap();
        fake_repo(&solo);
        // "." from inside the repository names it by its directory.
        assert_eq!(resolved_names(&resolve(".", &solo, None, 3)), ["solo"]);
    }

    #[test]
    fn a_bare_repository_and_a_worktree_count() {
        let dir = tempfile::tempdir().unwrap();
        let bare = dir.path().join("bare.git");
        fs::create_dir_all(bare.join("objects")).unwrap();
        fs::create_dir_all(bare.join("refs")).unwrap();
        fs::write(bare.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        assert_eq!(
            resolved_names(&resolve(bare.to_str().unwrap(), dir.path(), None, 3)),
            ["bare.git"]
        );

        let admin = dir.path().join("admin");
        fs::create_dir_all(&admin).unwrap();
        fs::write(admin.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        fs::write(admin.join("commondir"), "../..\n").unwrap();
        let work = dir.path().join("work");
        fs::create_dir_all(&work).unwrap();
        fs::write(work.join(".git"), format!("gitdir: {}\n", admin.display())).unwrap();
        assert_eq!(
            resolved_names(&resolve(work.to_str().unwrap(), dir.path(), None, 3)),
            ["work"]
        );
    }

    #[test]
    fn comments_errors_and_missing_paths() {
        let dir = tempfile::tempdir().unwrap();
        match resolve("#  hello ", dir.path(), None, 3) {
            Resolved::Comment(text) => assert_eq!(text, "hello"),
            _ => panic!("not a comment"),
        }
        expect_path_error(&resolve("/nonexistent-9up-test", dir.path(), None, 3));
        let file = dir.path().join("file");
        fs::write(&file, "x").unwrap();
        assert!(
            expect_path_error(&resolve(file.to_str().unwrap(), dir.path(), None, 3))
                .contains("isn't a repository")
        );
        expect_path_error(&resolve("", dir.path(), None, 3));
        expect_path_error(&resolve("/nonexistent-9up-test-*", dir.path(), None, 3));
        expect_path_error(&resolve("[", dir.path(), None, 3));
    }

    #[cfg(unix)]
    #[test]
    fn a_broken_symbolic_link_has_no_repositories() {
        let dir = tempfile::tempdir().unwrap();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(dir.path().join("missing"), &link).unwrap();
        match resolve(link.to_str().unwrap(), dir.path(), None, 3) {
            Resolved::Repos(repos) => assert!(repos.is_empty()),
            _ => panic!("not repos"),
        }
    }

    #[test]
    fn search_skips_files_and_git_directories_and_names_the_root() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        fs::create_dir_all(&repo).unwrap();
        fake_repo(&repo);
        fs::write(dir.path().join("stray"), "x").unwrap();
        let inner = dir.path().join(".git").join("inner");
        fs::create_dir_all(&inner).unwrap();
        fake_repo(&inner);
        let resolved = resolve(dir.path().to_str().unwrap(), dir.path(), None, 3);
        assert_eq!(resolved_names(&resolved), ["repo"]);
        let found = search(&repo, 3, &repo);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "repo");
    }

    #[test]
    fn a_double_star_glob_lists_each_repository_once() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["a/one", "b/two"] {
            let repo = dir.path().join("base").join(name);
            fs::create_dir_all(&repo).unwrap();
            fake_repo(&repo);
        }
        let pattern = format!("{}/base/**", dir.path().display());
        let resolved = resolve(&pattern, dir.path(), None, -1);
        let names = resolved_names(&resolved);
        assert_eq!(names.len(), 2, "{names:?}");
        assert_eq!(names, ["a/one", "b/two"]);
    }

    #[test]
    fn a_symbolic_link_cycle_terminates() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().join("base");
        let repo = base.join("repo");
        fs::create_dir_all(&repo).unwrap();
        fake_repo(&repo);
        // A repository only reachable by following a symbolic link:
        // without follow_links(true) it is never found.
        let outside = dir.path().join("outside");
        fs::create_dir_all(&outside).unwrap();
        fake_repo(&outside);
        // A cycle: base/loop points back at base. walkdir reports the
        // loop as an error entry, which search() skips.
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&base, base.join("loop")).unwrap();
            std::os::unix::fs::symlink(&outside, base.join("linked")).unwrap();
        }
        let resolved = resolve(base.to_str().unwrap(), dir.path(), None, -1);
        assert_eq!(resolved_names(&resolved), ["linked", "repo"]);
    }

    #[test]
    fn a_glob_finds_repositories() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["one", "two"] {
            let repo = dir.path().join(name);
            fs::create_dir_all(&repo).unwrap();
            fake_repo(&repo);
        }
        let pattern = format!("{}/*", dir.path().display());
        assert_eq!(
            resolved_names(&resolve(&pattern, dir.path(), None, 3)),
            ["one", "two"]
        );
        let dot = dir.path().join(".hidden");
        fs::create_dir_all(&dot).unwrap();
        fake_repo(&dot);
        assert_eq!(
            resolved_names(&resolve(&pattern, dir.path(), None, 3)),
            ["one", "two"]
        );
    }

    #[test]
    fn path_is_live_covers_directories_files_and_broken_links() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        assert!(path_is_live(root));
        let file = root.join("file.txt");
        fs::write(&file, "").unwrap();
        assert!(path_is_live(&file));
        assert!(!path_is_live(&root.join("missing")));
        #[cfg(unix)]
        {
            let link = root.join("link");
            std::os::unix::fs::symlink(root.join("gone"), &link).unwrap();
            assert!(path_is_live(&link));
        }
    }
}
