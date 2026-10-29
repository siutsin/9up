use std::path::{Path, PathBuf};

use crate::sys::Sys;

/// Data only, so milestone 2 can collect reports from many threads and main render them by path.
pub struct Report {
    pub name: String,
    pub steps: Vec<Step>,
}

pub enum Step {
    Fetch {
        remote: String,
        outcome: FetchOutcome,
    },
    Branch {
        name: String,
        outcome: BranchOutcome,
    },
    Error(String),
}

pub enum BranchOutcome {
    Updated { from: String, to: String },
    UpToDate,
    Skipped(String),
    Error(String),
}

pub enum FetchOutcome {
    Changed(Vec<(Kind, Vec<String>)>),
    UpToDate,
    NoRefspec,
    Error(Vec<String>),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    NewBranch,
    NewTag,
    BranchUpdate,
}

#[derive(Clone, Copy)]
pub struct Options {
    pub current_only: bool,
    pub fetch_only: bool,
    pub prune: bool,
}

/// Most git queries need only an answer or its absence, so every failure becomes `None`.
fn git_text(sys: &dyn Sys, dir: &Path, argv: &[&str]) -> Option<String> {
    let output = sys.git(dir, argv).ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

struct RepoInfo {
    work_dir: PathBuf,
    bare: bool,
}

/// Ask git where the repository lives. A bare one has no top level, so its work dir is the git dir.
fn repo_info(sys: &dyn Sys, path: &Path) -> Option<RepoInfo> {
    let git_dir = PathBuf::from(git_text(sys, path, &["rev-parse", "--absolute-git-dir"])?);
    let top = git_text(sys, path, &["rev-parse", "--show-toplevel"]);
    let bare = top.is_none();
    let work_dir = top.map(PathBuf::from).unwrap_or_else(|| git_dir);
    Some(RepoInfo { work_dir, bare })
}

struct Remote {
    name: String,
    refspecs: Vec<String>,
}

/// Remotes in config order with their fetch refspecs, from one `git config` call.
fn remotes(sys: &dyn Sys, path: &Path) -> Vec<Remote> {
    let Some(text) = git_text(sys, path, &["config", "--get-regexp", "^remote\\."]) else {
        return Vec::new();
    };
    parse_remotes(&text)
}

/// Kept apart from `remotes` so config parsing is testable without git.
fn parse_remotes(text: &str) -> Vec<Remote> {
    let mut remotes: Vec<Remote> = Vec::new();
    for line in text.lines() {
        let Some((key, value)) = line.split_once(' ') else {
            continue;
        };
        let Some(rest) = key.strip_prefix("remote.") else {
            continue;
        };
        let Some((name, field)) = rest.rsplit_once('.') else {
            continue;
        };
        let index = match remotes.iter().position(|remote| remote.name == name) {
            Some(index) => index,
            None => {
                remotes.push(Remote {
                    name: name.to_owned(),
                    refspecs: Vec::new(),
                });
                remotes.len() - 1
            }
        };
        if field == "fetch" {
            remotes[index].refspecs.push(value.to_owned());
        }
    }
    remotes
}

/// Full current branch name without `refs/heads/`; `None` on a detached HEAD.
/// Never `--short`: a same-named tag would change the answer.
fn current_branch(sys: &dyn Sys, path: &Path) -> Option<String> {
    let full = git_text(sys, path, &["symbolic-ref", "-q", "HEAD"])?;
    full.strip_prefix("refs/heads/").map(str::to_owned)
}

/// The branch's tracked remote from config; both remote and merge must be set.
/// Config works on an unborn branch, which has no refs yet.
fn tracked_remote(sys: &dyn Sys, path: &Path, branch: &str) -> Option<String> {
    let remote = git_text(
        sys,
        path,
        &["config", "--get", &format!("branch.{branch}.remote")],
    )?;
    git_text(
        sys,
        path,
        &["config", "--get", &format!("branch.{branch}.merge")],
    )?;
    Some(remote)
}

/// A report holding only one repository error.
fn failed(name: &str, message: &str) -> Report {
    Report {
        name: name.to_owned(),
        steps: vec![Step::Error(message.to_owned())],
    }
}

/// Fetch chosen remotes, then fast-forward branches; a `Report` lets only the front end print.
pub fn update_repo(sys: &dyn Sys, path: &Path, name: &str, options: &Options) -> Report {
    let mut report = Report {
        name: name.to_owned(),
        steps: Vec::new(),
    };
    let all_remotes = remotes(sys, path);
    let picked: Vec<&Remote> = if options.current_only {
        let Some(branch) = current_branch(sys, path) else {
            return failed(name, "HEAD is detached, so no remote is tracked.");
        };
        let Some(remote_name) = tracked_remote(sys, path, &branch) else {
            return failed(name, "The current branch tracks no remote.");
        };
        match all_remotes.iter().find(|remote| remote.name == remote_name) {
            Some(remote) => vec![remote],
            None => {
                if all_remotes.is_empty() {
                    return failed(name, "No remotes.");
                }
                // Unconfigured: a fetch skip like a refspec-less remote. Not a repository error,
                // so the branch step still runs.
                report.steps.push(Step::Fetch {
                    remote: remote_name,
                    outcome: FetchOutcome::NoRefspec,
                });
                Vec::new()
            }
        }
    } else {
        if all_remotes.is_empty() {
            return failed(name, "No remotes.");
        }
        all_remotes.iter().collect()
    };
    for remote in picked {
        if remote.refspecs.is_empty() {
            report.steps.push(Step::Fetch {
                remote: remote.name.clone(),
                outcome: FetchOutcome::NoRefspec,
            });
            continue;
        }
        let outcome = fetch(sys, path, &remote.name, options.prune);
        let failed = matches!(outcome, FetchOutcome::Error(_));
        report.steps.push(Step::Fetch {
            remote: remote.name.clone(),
            outcome,
        });
        if failed {
            break;
        }
    }
    if !options.fetch_only {
        let bare = repo_info(sys, path).map(|info| info.bare).unwrap_or(false);
        branch_step(
            sys,
            path,
            &current_branch(sys, path),
            bare,
            &mut report.steps,
        );
    }
    report
}

/// A snapshot branch: name, SHA and full upstream reference, empty when it has none.
struct Branch {
    name: String,
    sha: String,
    upstream: String,
}

/// Fast-forward branches to upstream; merge only the checked-out one so its work tree keeps up.
fn branch_step(
    sys: &dyn Sys,
    path: &Path,
    current: &Option<String>,
    bare: bool,
    steps: &mut Vec<Step>,
) {
    let Some(text) = git_text(
        sys,
        path,
        &[
            "for-each-ref",
            "--format=%(refname)%00%(objectname)%00%(upstream)",
            "refs/heads",
            "refs/remotes",
        ],
    ) else {
        return;
    };
    let mut branches: Vec<Branch> = Vec::new();
    let mut shas: Vec<(String, String)> = Vec::new();
    for line in text.lines() {
        let mut parts = line.split('\0');
        let (Some(reference), Some(sha), upstream) = (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        shas.push((reference.to_owned(), sha.to_owned()));
        if let Some(name) = reference.strip_prefix("refs/heads/") {
            branches.push(Branch {
                name: name.to_owned(),
                sha: sha.to_owned(),
                upstream: upstream.unwrap_or("").to_owned(),
            });
        }
    }
    for index in 0..branches.len() {
        let branch = &branches[index];
        if branch.upstream.is_empty() {
            continue; // No upstream: skipped silently.
        }
        let Some(upstream_sha) = shas
            .iter()
            .find(|(reference, _)| *reference == branch.upstream)
            .map(|(_, sha)| sha.clone())
        else {
            continue; // The upstream reference is missing locally.
        };
        let Some(base) = git_text(sys, path, &["merge-base", &branch.sha, &upstream_sha]) else {
            continue; // No merge base: skipped silently.
        };
        let name = branch.name.clone();
        let upstream_ref = branch.upstream.clone();
        if base == upstream_sha {
            steps.push(Step::Branch {
                name,
                outcome: BranchOutcome::UpToDate,
            });
        } else if base != branch.sha {
            steps.push(Step::Branch {
                name,
                outcome: BranchOutcome::Skipped("a fast-forward is not possible".to_owned()),
            });
        } else if !bare && current.as_deref() == Some(name.as_str()) {
            match sys.git(path, &["merge", "--ff-only", &upstream_ref]) {
                Ok(output) if output.status.success() => {
                    push_updated(steps, &branches, &mut shas, index, &upstream_sha);
                }
                Ok(output) => {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    let reason = if stderr.contains("local changes")
                        && stderr.contains("would be overwritten")
                    {
                        "uncommitted changes"
                    } else {
                        "merge failed"
                    };
                    steps.push(Step::Branch {
                        name,
                        outcome: BranchOutcome::Skipped(reason.to_owned()),
                    });
                }
                Err(err) => steps.push(Step::Branch {
                    name,
                    outcome: BranchOutcome::Skipped(format!("Could not run git: {err}")),
                }),
            }
        } else {
            match sys.git(path, &["branch", "-f", "--", &name, &upstream_ref]) {
                Ok(output) if output.status.success() => {
                    push_updated(steps, &branches, &mut shas, index, &upstream_sha);
                }
                Ok(output) => {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    let line = git_error_lines(&stderr)
                        .into_iter()
                        .next()
                        .unwrap_or_else(|| {
                            format!("git branch -f {name} failed with {}.", output.status)
                        });
                    steps.push(Step::Branch {
                        name,
                        outcome: BranchOutcome::Error(line),
                    });
                }
                Err(err) => steps.push(Step::Branch {
                    name,
                    outcome: BranchOutcome::Error(format!("Could not run git: {err}")),
                }),
            }
        }
    }
}

/// Record a moved branch and update the snapshot, so later branches tracking it see the new SHA.
fn push_updated(
    steps: &mut Vec<Step>,
    branches: &[Branch],
    shas: &mut [(String, String)],
    index: usize,
    upstream_sha: &str,
) {
    let branch = &branches[index];
    // A later branch may track this one: its snapshot SHA moves too.
    let reference = format!("refs/heads/{}", branch.name);
    if let Some((_, sha)) = shas.iter_mut().find(|(r, _)| *r == reference) {
        *sha = upstream_sha.to_owned();
    }
    steps.push(Step::Branch {
        name: branch.name.clone(),
        outcome: BranchOutcome::Updated {
            from: branch.sha.clone(),
            to: upstream_sha.to_owned(),
        },
    });
}

/// The trimmed git stderr lines that carry the failure: those starting `error:` or `fatal:`.
fn git_error_lines(stderr: &str) -> Vec<String> {
    stderr
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("error:") || line.starts_with("fatal:"))
        .map(str::to_owned)
        .collect()
}

/// Fetch one remote with `--porcelain`, so changes are parsed, not scraped from human text.
fn fetch(sys: &dyn Sys, path: &Path, remote: &str, prune: bool) -> FetchOutcome {
    let mut argv = vec!["fetch", "--porcelain"];
    if prune {
        argv.push("--prune");
    }
    argv.push("--");
    argv.push(remote);
    let output = match sys.git(path, &argv) {
        Ok(output) => output,
        Err(err) => return FetchOutcome::Error(vec![format!("Could not run git: {err}")]),
    };
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let mut lines = git_error_lines(&stderr);
        if lines.is_empty() {
            lines.push(format!("git fetch {remote} failed with {}.", output.status));
        }
        return FetchOutcome::Error(lines);
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let changed = parse_fetch_output(&stdout, remote);
    if changed.is_empty() {
        FetchOutcome::UpToDate
    } else {
        FetchOutcome::Changed(changed)
    }
}

/// Changed references in fetch porcelain output, grouped by kind. A `*` flag is a new reference,
/// a line with old and new values is an update, and other flags are not reported.
fn parse_fetch_output(text: &str, remote: &str) -> Vec<(Kind, Vec<String>)> {
    let remote_prefix = format!("refs/remotes/{remote}/");
    let mut groups: Vec<(Kind, Vec<String>)> = Vec::new();
    for line in text.lines() {
        // The line is <flag><space><old> <new> <ref>. An update's flag is itself a space,
        // so the fields start after the first two characters.
        let mut chars = line.chars();
        let Some(flag) = chars.next() else { continue };
        let rest = chars.as_str().strip_prefix(' ').unwrap_or(chars.as_str());
        let Some(reference) = rest.split(' ').nth(2) else {
            continue;
        };
        let kind = match flag {
            '*' if reference.starts_with("refs/tags/") => Kind::NewTag,
            '*' => Kind::NewBranch,
            ' ' => Kind::BranchUpdate,
            _ => continue,
        };
        let name = if let Some(short) = reference.strip_prefix(&remote_prefix) {
            short.to_owned()
        } else {
            // Drop the first two parts: refs/tags/, refs/heads/, and so on.
            reference
                .splitn(3, '/')
                .nth(2)
                .unwrap_or(reference)
                .to_owned()
        };
        match groups
            .iter_mut()
            .find(|(group_kind, _)| *group_kind == kind)
        {
            Some((_, names)) => names.push(name),
            None => groups.push((kind, vec![name])),
        }
    }
    groups
}

/// One repository's exec result: raw stdout and stderr, or the error line if the command
/// could not start. main splits the text into lines.
pub struct ExecReport {
    pub name: String,
    pub stdout: String,
    pub stderr: String,
    pub error: Option<String>,
}

/// Run the command without a shell in the work dir, or the repository dir if bare.
/// The exit status is ignored.
pub fn exec_repo(sys: &dyn Sys, path: &Path, name: &str, command: &[String]) -> ExecReport {
    let blank = |error: Option<String>| ExecReport {
        name: name.to_owned(),
        stdout: String::new(),
        stderr: String::new(),
        error,
    };
    let Some(info) = repo_info(sys, path) else {
        return blank(Some("Could not read the repository.".to_owned()));
    };
    match sys.exec(&info.work_dir, command) {
        Ok(output) => ExecReport {
            name: name.to_owned(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            error: None,
        },
        Err(err) => blank(Some(format!("Could not run {}: {err}", command[0]))),
    }
}

#[cfg(test)]
mod tests;
