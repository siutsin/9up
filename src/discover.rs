use std::fs;
use std::path::{Path, PathBuf};

use glob::MatchOptions;

pub struct Repo {
    pub name: String,
    // Read from Step 2 on, when update runs git in the repository.
    #[allow(dead_code)]
    pub path: PathBuf,
}

pub enum Resolved {
    Repos(Vec<Repo>),
    Comment(String),
    PathError(String),
}

/// Expand a leading `~` or `~/` to the home directory.
pub fn expand_home(path: &str) -> PathBuf {
    let Some(home) = std::env::home_dir() else {
        return PathBuf::from(path);
    };
    if path == "~" {
        return home;
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

/// Resolve one input (a command-line path or a bookmark) to the
/// repositories under it, a comment, or a path error.
pub fn resolve(input: &str, cwd: &Path, depth: i32) -> Resolved {
    if input.is_empty() {
        return Resolved::PathError(format!("{input} doesn't exist."));
    }
    let expanded = expand_home(input);
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
            repos.sort_by(|a, b| a.name.cmp(&b.name));
            Resolved::Repos(repos)
        }
    }
}

/// The comment text of a missing path whose first non-space character is
/// `#`, or None. An empty text is Some(""): it prints nothing.
fn comment_text(input: &str) -> Option<String> {
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
    dir.join("HEAD").is_file()
        && dir.join("refs").is_dir()
        && (dir.join("objects").is_dir() || dir.join("commondir").is_file())
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
    let mut repos = Vec::new();
    collect(dir, depth, header, &mut repos);
    repos
}

fn collect(dir: &Path, remaining: i32, header: &Path, repos: &mut Vec<Repo>) {
    if remaining == 0 {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return; // An unreadable directory is skipped.
    };
    let mut subdirs: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.file_name().is_some_and(|name| name != ".git"))
        .filter(|path| fs::metadata(path).is_ok_and(|meta| meta.is_dir()))
        .collect();
    subdirs.sort();
    for subdir in subdirs {
        if is_repo(&subdir) {
            repos.push(Repo {
                name: name_for(&subdir, header),
                path: subdir,
            });
        } else {
            collect(&subdir, remaining - 1, header, repos);
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_repo(dir: &Path) {
        let git = dir.join(".git");
        fs::create_dir_all(git.join("objects")).unwrap();
        fs::create_dir_all(git.join("refs")).unwrap();
        fs::write(git.join("HEAD"), "ref: refs/heads/main\n").unwrap();
    }

    fn resolved_names(resolved: &Resolved) -> Vec<&str> {
        match resolved {
            Resolved::Repos(repos) => repos.iter().map(|repo| repo.name.as_str()).collect(),
            _ => panic!("not repos"),
        }
    }

    #[test]
    fn a_repository_at_the_path() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        fs::create_dir_all(&repo).unwrap();
        fake_repo(&repo);
        let resolved = resolve(repo.to_str().unwrap(), dir.path(), 3);
        assert_eq!(resolved_names(&resolved), ["repo"]);
    }

    #[test]
    fn repositories_below_the_path_respect_depth() {
        let dir = tempfile::tempdir().unwrap();
        let deep = dir.path().join("a/b/repo");
        fs::create_dir_all(&deep).unwrap();
        fake_repo(&deep);
        let base = dir.path().to_str().unwrap().to_owned();
        assert!(resolved_names(&resolve(&base, dir.path(), 1)).is_empty());
        assert!(resolved_names(&resolve(&base, dir.path(), 2)).is_empty());
        assert_eq!(resolved_names(&resolve(&base, dir.path(), 3)), ["a/b/repo"]);
        assert_eq!(
            resolved_names(&resolve(&base, dir.path(), -1)),
            ["a/b/repo"]
        );
    }

    #[test]
    fn relative_inputs_name_repositories_by_relative_path() {
        let dir = tempfile::tempdir().unwrap();
        let deep = dir.path().join("base/a/repo");
        fs::create_dir_all(&deep).unwrap();
        fake_repo(&deep);
        assert_eq!(resolved_names(&resolve("base", dir.path(), 3)), ["a/repo"]);
        let solo = dir.path().join("solo");
        fs::create_dir_all(&solo).unwrap();
        fake_repo(&solo);
        // "." from inside the repository names it by its directory.
        assert_eq!(resolved_names(&resolve(".", &solo, 3)), ["solo"]);
    }

    #[test]
    fn a_bare_repository_and_a_worktree_count() {
        let dir = tempfile::tempdir().unwrap();
        let bare = dir.path().join("bare.git");
        fs::create_dir_all(bare.join("objects")).unwrap();
        fs::create_dir_all(bare.join("refs")).unwrap();
        fs::write(bare.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        assert_eq!(
            resolved_names(&resolve(bare.to_str().unwrap(), dir.path(), 3)),
            ["bare.git"]
        );

        let admin = dir.path().join("admin");
        fs::create_dir_all(admin.join("refs")).unwrap();
        fs::write(admin.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        fs::write(admin.join("commondir"), "../..\n").unwrap();
        let work = dir.path().join("work");
        fs::create_dir_all(&work).unwrap();
        fs::write(work.join(".git"), format!("gitdir: {}\n", admin.display())).unwrap();
        assert_eq!(
            resolved_names(&resolve(work.to_str().unwrap(), dir.path(), 3)),
            ["work"]
        );
    }

    #[test]
    fn comments_errors_and_missing_paths() {
        let dir = tempfile::tempdir().unwrap();
        match resolve("#  hello ", dir.path(), 3) {
            Resolved::Comment(text) => assert_eq!(text, "hello"),
            _ => panic!("not a comment"),
        }
        match resolve("/nonexistent-9up-test", dir.path(), 3) {
            Resolved::PathError(_) => {}
            _ => panic!("not an error"),
        }
        let file = dir.path().join("file");
        fs::write(&file, "x").unwrap();
        match resolve(file.to_str().unwrap(), dir.path(), 3) {
            Resolved::PathError(message) => assert!(message.contains("isn't a repository")),
            _ => panic!("not an error"),
        }
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
            resolved_names(&resolve(&pattern, dir.path(), 3)),
            ["one", "two"]
        );
        let dot = dir.path().join(".hidden");
        fs::create_dir_all(&dot).unwrap();
        fake_repo(&dot);
        assert_eq!(
            resolved_names(&resolve(&pattern, dir.path(), 3)),
            ["one", "two"]
        );
    }
}
