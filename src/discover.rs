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

/// Expand a leading `~` or `~/` to the home directory; with no home, return the path as given.
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

/// The existing paths a glob matches. A bad pattern matches none; a broken symbolic link counts.
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

/// The one definition of "exists" for an expanded entry: a directory, or a glob with a match.
/// The glob also covers plain files and broken symbolic links, which `is_dir` alone misses.
pub fn path_is_live(expanded: &Path) -> bool {
    expanded.is_dir() || !glob_paths(&expanded.display().to_string()).is_empty()
}

/// Resolve a command-line or configured path to its repositories, a comment, or a path error.
pub fn resolve(input: &str, cwd: &Path, home: Option<&Path>, depth: i32) -> Resolved {
    if input.is_empty() {
        return Resolved::PathError("doesn't exist.".to_owned());
    }
    let expanded = expand_home(input, home);
    let display = expanded.display().to_string();
    // Filesystem work uses the absolute form; `expanded` stays as typed for path errors.
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
            // Only a missing path with these characters is tried as a glob, so a typo stays a missing path.
            if !input.chars().any(|c| matches!(c, '*' | '?' | '[')) {
                return Resolved::PathError(format!("{display} doesn't exist."));
            }
            let pattern = fs_path.display().to_string();
            let matches = glob_paths(&pattern);
            if matches.is_empty() {
                return Resolved::PathError(format!("{display} doesn't exist."));
            }
            let prefix = {
                let mut prefix = PathBuf::new();
                for part in Path::new(&pattern).components() {
                    let text = part.as_os_str().to_string_lossy();
                    if text.chars().any(|c| matches!(c, '*' | '?' | '[')) {
                        break;
                    }
                    prefix.push(part.as_os_str());
                }
                prefix
            };
            let mut repos = Vec::new();
            for matched in &matches {
                repos.extend(search(matched, depth, &prefix));
            }
            // `**` matches a parent directory and the repository below it, so a path can repeat.
            let mut seen = std::collections::HashSet::new();
            repos.retain(|repo| seen.insert(canonical_key(&repo.path)));
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

/// A git directory: a HEAD file, a refs directory, and an objects
/// directory or a commondir file (a linked worktree's admin directory).
fn is_git_dir(dir: &Path) -> bool {
    // A linked worktree's admin directory has HEAD and commondir but no refs (git creates none),
    // so commondir suffices; a normal or bare one pairs refs with objects.
    dir.join("HEAD").is_file()
        && (dir.join("commondir").is_file()
            || (dir.join("refs").is_dir() && dir.join("objects").is_dir()))
}

/// Whether a path is a repository, by filesystem test only: discovery runs no git.
fn is_repo(path: &Path) -> bool {
    if is_git_dir(path) {
        return true;
    }
    let dot_git = path.join(".git");
    if is_git_dir(&dot_git) {
        return true;
    }
    // A `.git` file (linked worktree or submodule) points at the git directory with `gitdir:`.
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

/// The key two spellings of one directory share: its canonical path, or the path itself
/// when that cannot be resolved.
fn canonical_key(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Repositories at and below `dir`, down to `depth` levels (negative means no limit). Names are
/// relative to `header`, except a repository at `dir` itself, which takes its last component.
fn search(dir: &Path, depth: i32, header: &Path) -> Vec<Repo> {
    if is_repo(dir) {
        return vec![Repo {
            name: name_for(dir, header),
            path: dir.to_path_buf(),
        }];
    }
    // A negative depth becomes the unlimited case.
    let max_depth = depth.try_into().unwrap_or(usize::MAX);
    let walk = WalkDir::new(dir)
        .follow_links(true)
        .min_depth(1)
        .max_depth(max_depth);
    let mut repos = Vec::new();
    // A link can reach a directory under a second name; walk each physical directory once,
    // or an aliased repository is reported once per spelling.
    let mut visited = std::collections::HashSet::from([canonical_key(dir)]);
    let mut iter = walk.into_iter();
    while let Some(entry) = iter.next() {
        // Skip an unreadable directory or a symbolic link loop, which walkdir reports as an error.
        let Ok(entry) = entry else { continue };
        if !entry.file_type().is_dir() {
            continue;
        }
        let path = entry.path();
        if path.file_name().is_some_and(|name| name == ".git") {
            iter.skip_current_dir();
            continue;
        }
        let key = canonical_key(path);
        if visited.contains(&key) {
            iter.skip_current_dir();
            continue;
        }
        if is_repo(path) {
            visited.insert(key);
            repos.push(Repo {
                name: name_for(path, header),
                path: entry.into_path(),
            });
            iter.skip_current_dir(); // A found repository is not descended into.
        } else if entry.depth() < max_depth {
            // A directory at the depth limit is never descended, so marking it visited
            // would block a shallower spelling of it that still can be.
            visited.insert(key);
        }
    }
    repos
}

/// Name a repository relative to the input, so output shows where it lives under it.
fn name_for(repo: &Path, header: &Path) -> String {
    if repo == header {
        return last_component(repo);
    }
    match repo.strip_prefix(header) {
        Ok(relative) => relative.display().to_string(),
        Err(_) => last_component(repo),
    }
}

/// A path that is itself a repository is named by its last component, per the naming rule.
fn last_component(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

#[cfg(test)]
pub(crate) mod tests;
