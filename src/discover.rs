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

#[cfg(test)]
pub(crate) mod tests;
