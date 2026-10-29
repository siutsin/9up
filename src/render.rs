use std::io::{self, Write};
use std::path::Path;

use crate::update;
use crate::{config, discover};

const BOLD_WHITE: &str = "\u{1b}[1;37m";
pub(crate) const BOLD_GREEN: &str = "\u{1b}[1;32m";
pub(crate) const RED: &str = "\u{1b}[31m";
const CYAN: &str = "\u{1b}[36m";
const RESET: &str = "\u{1b}[0m";

/// Colour text only when asked, so piped output stays free of escape codes.
pub(crate) fn paint(enabled: bool, code: &str, text: &str) -> String {
    if enabled {
        format!("{code}{text}{RESET}")
    } else {
        text.to_owned()
    }
}

/// Show each input as an absolute path with its repository count, clear whatever the user typed.
pub(crate) fn print_header(
    out: &mut dyn Write,
    colour: bool,
    input: &str,
    home: Option<&Path>,
    cwd: &Path,
    count: usize,
) -> io::Result<()> {
    let expanded = discover::expand_home(input, home);
    let header = config::normalize(&expanded.display().to_string(), cwd);
    let noun = if count == 1 {
        "repository"
    } else {
        "repositories"
    };
    writeln!(
        out,
        "{} ({count} {noun})",
        paint(colour, BOLD_WHITE, &header)
    )?;
    Ok(())
}

/// One style for step errors and skip lines under a repository, so they look alike.
pub(crate) fn red_line(out: &mut dyn Write, colour: bool, text: &str) -> io::Result<()> {
    writeln!(out, "    {}", paint(colour, RED, text))?;
    Ok(())
}

/// Render a `Report` here: `update_repo` only returns data and the front end owns output.
pub(crate) fn render_report(
    report: &update::Report,
    colour: bool,
    out: &mut dyn Write,
) -> io::Result<()> {
    writeln!(out, "  {}", paint(colour, BOLD_GREEN, &report.name))?;
    for step in &report.steps {
        match step {
            update::Step::Fetch { remote, outcome } => match outcome {
                update::FetchOutcome::Changed(groups) => {
                    // Groups print in a fixed order, not fetch order.
                    let order = [
                        update::Kind::NewBranch,
                        update::Kind::NewTag,
                        update::Kind::BranchUpdate,
                    ];
                    let parts: Vec<String> = order
                        .iter()
                        .filter_map(|kind| groups.iter().find(|(group_kind, _)| group_kind == kind))
                        .map(|(kind, names)| {
                            let label = match kind {
                                update::Kind::NewBranch => "new branches",
                                update::Kind::NewTag => "new tags",
                                update::Kind::BranchUpdate => "updated references",
                            };
                            format!("{label}: {}", names.join(", "))
                        })
                        .collect();
                    let line = format!("Fetched {remote}: {}.", parts.join("; "));
                    writeln!(out, "    {}", paint(colour, CYAN, &line))?;
                }
                update::FetchOutcome::UpToDate => {
                    writeln!(out, "    {remote} is up to date.")?;
                }
                update::FetchOutcome::NoRefspec => {
                    red_line(
                        out,
                        colour,
                        &format!("{remote} has no fetch refspec. Skipped."),
                    )?;
                }
                update::FetchOutcome::Error(lines) => {
                    for line in lines {
                        red_line(out, colour, line)?;
                    }
                }
            },
            update::Step::Branch { name, outcome } => match outcome {
                update::BranchOutcome::Updated { from, to } => {
                    let line = format!("{name} updated: {}..{}", &from[..7], &to[..7]);
                    writeln!(out, "    {}", paint(colour, CYAN, &line))?;
                }
                update::BranchOutcome::UpToDate => {
                    writeln!(out, "    {name} is up to date.")?;
                }
                update::BranchOutcome::Skipped(reason) => {
                    red_line(out, colour, &format!("{name} skipped: {reason}."))?;
                }
                update::BranchOutcome::Error(message) => {
                    red_line(out, colour, &format!("{name}: {message}"))?;
                }
            },
            update::Step::Error(message) => red_line(out, colour, message)?,
        }
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests;
