use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

/// A Report carries data only, so milestone 2 can collect reports from
/// many threads and main can render them in path order.
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

/// The one child-process choke point. Every git call goes through here:
/// the working directory is set, the locale is pinned to English (the
/// branch logic reads git's words), and standard input is null.
pub fn run(dir: &Path, argv: &[&str]) -> io::Result<Output> {
    Command::new("git")
        .args(argv)
        .current_dir(dir)
        .env("LC_ALL", "C")
        .env("LANGUAGE", "C")
        .stdin(Stdio::null())
        .output()
}

fn git_text(dir: &Path, argv: &[&str]) -> Option<String> {
    let output = run(dir, argv).ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

pub struct RepoInfo {
    // Part of the rev-parse pair the plan prescribes.
    #[allow(dead_code)]
    pub git_dir: PathBuf,
    // Read by exec_repo in Step 3.
    #[allow(dead_code)]
    pub work_dir: PathBuf,
    pub bare: bool,
}

/// Let git answer where the repository lives. A bare repository has no
/// top level, so its working directory is the git directory.
pub fn repo_info(path: &Path) -> Option<RepoInfo> {
    let git_dir = PathBuf::from(git_text(path, &["rev-parse", "--absolute-git-dir"])?);
    let top = git_text(path, &["rev-parse", "--show-toplevel"]);
    let bare = top.is_none();
    let work_dir = top.map(PathBuf::from).unwrap_or_else(|| git_dir.clone());
    Some(RepoInfo {
        git_dir,
        work_dir,
        bare,
    })
}

struct Remote {
    name: String,
    refspecs: Vec<String>,
}

/// The remotes in configuration order, with their fetch refspecs, from
/// one `git config` call.
fn remotes(path: &Path) -> Vec<Remote> {
    let Some(text) = git_text(path, &["config", "--get-regexp", "^remote\\."]) else {
        return Vec::new();
    };
    parse_remotes(&text)
}

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

/// The current branch's full name without the `refs/heads/` prefix.
/// None means a detached HEAD. Never `--short`: a tag with the same
/// name would change the answer.
fn current_branch(path: &Path) -> Option<String> {
    let full = git_text(path, &["symbolic-ref", "-q", "HEAD"])?;
    full.strip_prefix("refs/heads/").map(str::to_owned)
}

/// The remote the branch tracks, from its configuration. Both the
/// remote and the merge ref must be set. Configuration works on an
/// unborn branch, where no refs exist yet.
fn tracked_remote(path: &Path, branch: &str) -> Option<String> {
    let remote = git_text(
        path,
        &["config", "--get", &format!("branch.{branch}.remote")],
    )?;
    git_text(
        path,
        &["config", "--get", &format!("branch.{branch}.merge")],
    )?;
    Some(remote)
}

/// A report that holds one repository error and nothing else.
fn failed(name: &str, message: &str) -> Report {
    Report {
        name: name.to_owned(),
        steps: vec![Step::Error(message.to_owned())],
    }
}

pub fn update_repo(path: &Path, name: &str, options: &Options) -> Report {
    let mut report = Report {
        name: name.to_owned(),
        steps: Vec::new(),
    };
    let all_remotes = remotes(path);
    let picked: Vec<&Remote> = if options.current_only {
        let Some(branch) = current_branch(path) else {
            return failed(name, "HEAD is detached, so no remote is tracked.");
        };
        let Some(remote_name) = tracked_remote(path, &branch) else {
            return failed(name, "The current branch tracks no remote.");
        };
        match all_remotes.iter().find(|remote| remote.name == remote_name) {
            Some(remote) => vec![remote],
            None => {
                if all_remotes.is_empty() {
                    return failed(name, "No remotes.");
                }
                // Not in the configuration at all: same outcome as a
                // refspec-less remote, the fetch skip line. Not a
                // repository error, so the branch step still runs.
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
        let outcome = fetch(path, &remote.name, options.prune);
        let failed = matches!(outcome, FetchOutcome::Error(_));
        report.steps.push(Step::Fetch {
            remote: remote.name.clone(),
            outcome,
        });
        if failed {
            break; // The first failed fetch stops this repository's fetches.
        }
    }
    if !options.fetch_only {
        let bare = repo_info(path).map(|info| info.bare).unwrap_or(false);
        branch_step(path, &current_branch(path), bare, &mut report.steps);
    }
    report
}

/// One branch from the snapshot: its name, its SHA, and the full
/// reference of its upstream (empty when it has none).
struct Branch {
    name: String,
    sha: String,
    upstream: String,
}

fn branch_step(path: &Path, current: &Option<String>, bare: bool, steps: &mut Vec<Step>) {
    let Some(text) = git_text(
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
        let Some(base) = git_text(path, &["merge-base", &branch.sha, &upstream_sha]) else {
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
            match run(path, &["merge", "--ff-only", &upstream_ref]) {
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
            match run(path, &["branch", "-f", "--", &name, &upstream_ref]) {
                Ok(output) if output.status.success() => {
                    push_updated(steps, &branches, &mut shas, index, &upstream_sha);
                }
                Ok(output) => {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    let line = stderr
                        .lines()
                        .map(str::trim)
                        .find(|line| line.starts_with("error:") || line.starts_with("fatal:"))
                        .map(str::to_owned)
                        .unwrap_or_else(|| {
                            format!("git branch -f {name} failed: {}", output.status)
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

fn fetch(path: &Path, remote: &str, prune: bool) -> FetchOutcome {
    let mut argv = vec!["fetch", "--porcelain"];
    if prune {
        argv.push("--prune");
    }
    argv.push("--");
    argv.push(remote);
    let output = match run(path, &argv) {
        Ok(output) => output,
        Err(err) => return FetchOutcome::Error(vec![format!("Could not run git: {err}")]),
    };
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let mut lines: Vec<String> = stderr
            .lines()
            .map(str::trim)
            .filter(|line| line.starts_with("error:") || line.starts_with("fatal:"))
            .map(str::to_owned)
            .collect();
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

/// The changed references in fetch porcelain output, grouped by kind.
/// A `*` flag is a new reference, a line with an old and a new value is
/// an update, and every other flag is not reported.
fn parse_fetch_output(text: &str, remote: &str) -> Vec<(Kind, Vec<String>)> {
    let remote_prefix = format!("refs/remotes/{remote}/");
    let mut groups: Vec<(Kind, Vec<String>)> = Vec::new();
    for line in text.lines() {
        // The line is <flag><space><old> <new> <ref>. The flag of an
        // update line is itself a space, so the fields start after the
        // first two characters.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_remotes_keeps_order_and_refspecs() {
        let text = "remote.origin.url /srv/git.git\nremote.origin.fetch +refs/heads/*:refs/remotes/origin/*\nremote.upstream.url /srv/up.git\nremote.mirror.fetch +refs/*:refs/*\nremote.mirror.url /srv/mirror.git\n";
        let remotes = parse_remotes(text);
        assert_eq!(remotes.len(), 3);
        assert_eq!(remotes[0].name, "origin");
        assert_eq!(remotes[0].refspecs.len(), 1);
        assert!(remotes[1].refspecs.is_empty());
        assert_eq!(remotes[2].name, "mirror");
        assert_eq!(remotes[2].refspecs.len(), 1);
    }

    #[test]
    fn parse_fetch_output_classifies_and_names() {
        let text = "  1111111111111111111111111111111111111111 2222222222222222222222222222222222222222 refs/remotes/origin/main\n* 0000000000000000000000000000000000000000 3333333333333333333333333333333333333333 refs/remotes/origin/feat\n* 0000000000000000000000000000000000000000 4444444444444444444444444444444444444444 refs/tags/v1\n+ 1111111111111111111111111111111111111111 2222222222222222222222222222222222222222 refs/remotes/origin/forced\n";
        let groups = parse_fetch_output(text, "origin");
        assert_eq!(
            groups,
            [
                (Kind::BranchUpdate, vec!["main".to_owned()]),
                (Kind::NewBranch, vec!["feat".to_owned()]),
                (Kind::NewTag, vec!["v1".to_owned()]),
            ]
        );
    }
}
