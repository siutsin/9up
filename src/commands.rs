use std::io;
use std::process::ExitCode;

use crate::cli::UpdateArgs;
use crate::render::{
    BOLD_GREEN, RED, clean as cleaned, paint, print_header, red_line, render_report,
};
use crate::sys::{App, Sys};
use crate::{config, discover, update};

/// Normalise each path before the check, so no repository is stored twice under two spellings.
/// Report lines are buffered and printed only after the save, so a failed save prints nothing.
pub(crate) fn add(app: &mut App, paths: &[String]) -> io::Result<ExitCode> {
    let mut entries = config::load(&*app.sys, &app.config_file)?;
    let mut lines = String::new();
    for raw in paths {
        let normalized = config::normalize(raw, &app.cwd);
        let shown = cleaned(&normalized);
        if entries.contains(&normalized) {
            lines.push_str(&format!("{shown} is already in the config file.\n"));
        } else {
            lines.push_str(&format!("Added {shown}.\n"));
            entries.push(normalized);
        }
    }
    config::save(&*app.sys, &app.config_file, &entries)?;
    app.out.write_all(lines.as_bytes())?;
    Ok(ExitCode::SUCCESS)
}

/// Save only on a change, so deleting unknown paths leaves the config file untouched.
pub(crate) fn delete(app: &mut App, paths: &[String]) -> io::Result<ExitCode> {
    let mut entries = config::load(&*app.sys, &app.config_file)?;
    let mut lines = String::new();
    let mut changed = false;
    for raw in paths {
        let normalized = config::normalize(raw, &app.cwd);
        let shown = cleaned(&normalized);
        if let Some(index) = entries.iter().position(|entry| entry == &normalized) {
            entries.remove(index);
            changed = true;
            lines.push_str(&format!("Deleted {shown}.\n"));
        } else {
            lines.push_str(&format!("{shown} is not in the config file.\n"));
        }
    }
    if changed {
        config::save(&*app.sys, &app.config_file, &entries)?;
    }
    app.out.write_all(lines.as_bytes())?;
    Ok(ExitCode::SUCCESS)
}

/// Print the config as stored; an empty one prints a message, so blank output is never ambiguous.
pub(crate) fn list(app: &mut App) -> io::Result<ExitCode> {
    let out = &mut *app.out;
    let entries = config::load(&*app.sys, &app.config_file)?;
    if entries.is_empty() {
        writeln!(out, "No repositories configured.")?;
    }
    for entry in &entries {
        writeln!(out, "{}", cleaned(entry))?;
    }
    Ok(ExitCode::SUCCESS)
}

/// With no paths given, use the configured ones; if none exist, point a new user to `9up add`.
pub(crate) fn update(app: &mut App, args: &UpdateArgs) -> io::Result<ExitCode> {
    let inputs: Vec<String> = if args.paths.is_empty() {
        let entries = config::load(&*app.sys, &app.config_file)?;
        if entries.is_empty() {
            writeln!(
                &mut *app.out,
                "No repositories configured. Add one with `9up add <path>`."
            )?;
            return Ok(ExitCode::SUCCESS);
        }
        entries
    } else {
        args.paths.clone()
    };
    let options = update::Options {
        current_only: args.current_only,
        fetch_only: args.fetch_only,
        prune: args.prune,
    };
    for_each_repo(
        app,
        &inputs,
        args.depth,
        |sys, repo| update::update_repo(sys, &repo.path, &repo.name, &options),
        |report, colour, out| render_report(report, colour, out),
    )?;
    Ok(ExitCode::SUCCESS)
}

/// Drop entries whose paths are gone, keeping comment entries, so stale paths do not pile up.
pub(crate) fn clean(app: &mut App) -> io::Result<ExitCode> {
    let out = &mut *app.out;
    let entries = config::load(&*app.sys, &app.config_file)?;
    if entries.is_empty() {
        writeln!(out, "No repositories configured.")?;
        return Ok(ExitCode::SUCCESS);
    }
    let mut kept = Vec::new();
    let mut lines = String::new();
    let mut removed = false;
    for entry in &entries {
        // A comment entry is not a path, so the liveness test cannot judge it.
        if discover::comment_text(entry).is_some() {
            kept.push(entry.clone());
            continue;
        }
        let expanded = discover::expand_home(entry, app.home.as_deref());
        if discover::path_is_live(&expanded) {
            kept.push(entry.clone());
        } else {
            removed = true;
            lines.push_str(&format!("Removed {}.\n", cleaned(entry)));
        }
    }
    if removed {
        config::save(&*app.sys, &app.config_file, &kept)?;
        app.out.write_all(lines.as_bytes())?;
    } else {
        writeln!(out, "Nothing to clean.")?;
    }
    Ok(ExitCode::SUCCESS)
}

/// Print each repository's output under its name, so one command's results stay readable.
pub(crate) fn exec(
    app: &mut App,
    paths: &[String],
    depth: i32,
    command: &[String],
) -> io::Result<ExitCode> {
    let inputs: Vec<String> = if paths.is_empty() {
        // No hint here: an exec run with no configured paths prints nothing.
        config::load(&*app.sys, &app.config_file)?
    } else {
        paths.to_vec()
    };
    for_each_repo(
        app,
        &inputs,
        depth,
        |sys, repo| update::exec_repo(sys, &repo.path, &repo.name, command),
        |report, colour, out| {
            writeln!(out, "  {}", paint(colour, BOLD_GREEN, &report.name))?;
            if let Some(error) = &report.error {
                red_line(out, colour, error)
            } else {
                for line in report.stdout.lines().chain(report.stderr.lines()) {
                    writeln!(out, "    {line}")?;
                }
                Ok(())
            }
        },
    )?;
    Ok(ExitCode::SUCCESS)
}

/// The most repositories one batch runs at once. The work is subprocess and network
/// waiting, not CPU work, so the bound does not come from the core count.
const MAX_CONCURRENT: usize = 8;

/// Shared by `update` and `exec`: resolve each input in turn — only after the group
/// before it has rendered — then run its repositories in batches, rendering each
/// report on this thread. A comment input prints as its own line; a bad path in red.
fn for_each_repo<T>(
    app: &mut App,
    inputs: &[String],
    depth: i32,
    work: impl Fn(&dyn Sys, &discover::Repo) -> T + Sync,
    mut render: impl FnMut(&T, bool, &mut dyn io::Write) -> io::Result<()>,
) -> io::Result<()>
where
    T: Send,
{
    for input in inputs {
        match discover::resolve(input, &app.cwd, app.home.as_deref(), depth) {
            discover::Resolved::Repos(repos) => {
                print_header(
                    &mut *app.out,
                    app.colour,
                    input,
                    app.home.as_deref(),
                    &app.cwd,
                    repos.len(),
                )?;
                run_group(app, &repos, &work, &mut render)?;
            }
            discover::Resolved::Comment(text) => {
                if !text.is_empty() {
                    writeln!(&mut *app.out, "{}", cleaned(&text))?;
                }
            }
            discover::Resolved::PathError(message) => {
                writeln!(&mut *app.out, "{}", paint(app.colour, RED, &message))?;
            }
        }
    }
    Ok(())
}

/// Run a path's repositories in consecutive batches: a batch spawns only when the batch
/// before it is fully joined and rendered.
fn run_group<T>(
    app: &mut App,
    repos: &[discover::Repo],
    work: &(impl Fn(&dyn Sys, &discover::Repo) -> T + Sync),
    render: &mut impl FnMut(&T, bool, &mut dyn io::Write) -> io::Result<()>,
) -> io::Result<()>
where
    T: Send,
{
    for batch in repos.chunks(MAX_CONCURRENT) {
        run_batch(app, batch, work, render)?;
    }
    Ok(())
}

/// What ended a batch early: a render failure, or the first worker panic in join order.
/// Whichever comes first in join order wins; the other is discarded.
enum Failure {
    Render(io::Error),
    Panic(Box<dyn std::any::Any + Send>),
}

/// Run one batch on scoped threads, joining in name order and rendering each report
/// as its join returns. Every handle is joined before this returns, so a worker
/// panic surfaces only as a join result.
fn run_batch<T>(
    app: &mut App,
    batch: &[discover::Repo],
    work: &(impl Fn(&dyn Sys, &discover::Repo) -> T + Sync),
    render: &mut impl FnMut(&T, bool, &mut dyn io::Write) -> io::Result<()>,
) -> io::Result<()>
where
    T: Send,
{
    let sys: &dyn Sys = &*app.sys;
    let colour = app.colour;
    let out: &mut dyn io::Write = &mut *app.out;
    let mut failure: Option<Failure> = None;
    std::thread::scope(|scope| {
        let handles: Vec<_> = batch
            .iter()
            .map(|repo| scope.spawn(|| work(sys, repo)))
            .collect();
        for handle in handles {
            match handle.join() {
                Ok(report) => {
                    if failure.is_none()
                        && let Err(err) = render(&report, colour, &mut *out)
                    {
                        failure = Some(Failure::Render(err));
                    }
                }
                Err(payload) => {
                    if failure.is_none() {
                        failure = Some(Failure::Panic(payload));
                    }
                }
            }
        }
    });
    match failure {
        None => Ok(()),
        Some(Failure::Render(err)) => Err(err),
        // The saved payload resumes only now, with every handle in the batch joined.
        Some(Failure::Panic(payload)) => std::panic::resume_unwind(payload),
    }
}

#[cfg(test)]
mod tests;
