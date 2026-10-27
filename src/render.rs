use std::io::{self, Write};
use std::path::Path;

use crate::update;
use crate::{config, discover};

const BOLD_WHITE: &str = "\u{1b}[1;37m";
pub(crate) const BOLD_GREEN: &str = "\u{1b}[1;32m";
pub(crate) const RED: &str = "\u{1b}[31m";
const CYAN: &str = "\u{1b}[36m";
const RESET: &str = "\u{1b}[0m";

pub(crate) fn paint(enabled: bool, code: &str, text: &str) -> String {
    if enabled {
        format!("{code}{text}{RESET}")
    } else {
        text.to_owned()
    }
}

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

pub(crate) fn red_line(out: &mut dyn Write, colour: bool, text: &str) -> io::Result<()> {
    writeln!(out, "    {}", paint(colour, RED, text))?;
    Ok(())
}

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

/// A writer whose writes always fail, shared by the crate's tests.
#[cfg(test)]
pub(crate) struct FailingWriter;

#[cfg(test)]
impl Write for FailingWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let _ = buf;
        Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed"))
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn print_header_shows_the_absolute_path_and_count() {
        let mut out: Vec<u8> = Vec::new();
        print_header(&mut out, false, "base", None, Path::new("/work"), 1).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "/work/base (1 repository)\n"
        );
        let mut out: Vec<u8> = Vec::new();
        print_header(&mut out, false, "/srv/repos", None, Path::new("/work"), 2).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "/srv/repos (2 repositories)\n"
        );
        let mut out: Vec<u8> = Vec::new();
        print_header(
            &mut out,
            false,
            "~/repos",
            Some(Path::new("/home/a")),
            Path::new("/work"),
            0,
        )
        .unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "/home/a/repos (0 repositories)\n"
        );
    }

    #[test]
    fn print_header_reports_a_failed_write() {
        let mut out = FailingWriter;
        assert!(print_header(&mut out, false, "/x", None, Path::new("/"), 1).is_err());
    }

    #[test]
    fn paint_wraps_the_text_in_the_codes_when_enabled() {
        assert_eq!(paint(true, RED, "x"), format!("{RED}x{RESET}"));
        assert_eq!(paint(false, RED, "x"), "x");
    }

    /// A writer that refuses the fetch-skip line, so a render fails
    /// midway rather than on its first line.
    struct SkipLineRefusingWriter;

    impl Write for SkipLineRefusingWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            if String::from_utf8_lossy(buf).contains("refspec") {
                return Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed"));
            }
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn the_test_writers_flush_successfully() {
        assert!(FailingWriter.flush().is_ok());
        assert!(SkipLineRefusingWriter.flush().is_ok());
    }

    #[test]
    fn render_report_prints_fetch_and_branch_lines() {
        let report = update::Report {
            name: "repo".to_owned(),
            steps: vec![
                update::Step::Fetch {
                    remote: "origin".to_owned(),
                    outcome: update::FetchOutcome::Changed(vec![(
                        update::Kind::NewBranch,
                        vec!["feat".to_owned()],
                    )]),
                },
                update::Step::Fetch {
                    remote: "upstream".to_owned(),
                    outcome: update::FetchOutcome::UpToDate,
                },
                update::Step::Branch {
                    name: "main".to_owned(),
                    outcome: update::BranchOutcome::Updated {
                        from: "1111111111111111111111111111111111111111".to_owned(),
                        to: "2222222222222222222222222222222222222222".to_owned(),
                    },
                },
                update::Step::Branch {
                    name: "side".to_owned(),
                    outcome: update::BranchOutcome::UpToDate,
                },
                update::Step::Branch {
                    name: "old".to_owned(),
                    outcome: update::BranchOutcome::Skipped(
                        "a fast-forward is not possible".to_owned(),
                    ),
                },
                update::Step::Error("No remotes.".to_owned()),
            ],
        };
        let mut out: Vec<u8> = Vec::new();
        render_report(&report, false, &mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("  repo\n"), "{text}");
        assert!(
            text.contains("Fetched origin: new branches: feat."),
            "{text}"
        );
        assert!(text.contains("upstream is up to date."), "{text}");
        assert!(text.contains("main updated: 1111111..2222222"), "{text}");
        assert!(text.contains("side is up to date."), "{text}");
        assert!(
            text.contains("old skipped: a fast-forward is not possible."),
            "{text}"
        );
        assert!(text.contains("No remotes."), "{text}");
    }

    #[test]
    fn render_report_prints_fetch_skip_and_error_lines() {
        let report = update::Report {
            name: "repo".to_owned(),
            steps: vec![
                update::Step::Fetch {
                    remote: "empty".to_owned(),
                    outcome: update::FetchOutcome::NoRefspec,
                },
                update::Step::Fetch {
                    remote: "origin".to_owned(),
                    outcome: update::FetchOutcome::Error(vec!["fatal: boom".to_owned()]),
                },
                update::Step::Branch {
                    name: "main".to_owned(),
                    outcome: update::BranchOutcome::Error("error: failed".to_owned()),
                },
            ],
        };
        let mut out: Vec<u8> = Vec::new();
        render_report(&report, false, &mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(
            text.contains("empty has no fetch refspec. Skipped."),
            "{text}"
        );
        assert!(text.contains("fatal: boom"), "{text}");
        assert!(text.contains("main: error: failed"), "{text}");
    }

    /// A writer that refuses any write containing `needle`, so a
    /// render fails on one chosen line.
    struct LineRefusingWriter {
        needle: &'static str,
    }

    impl Write for LineRefusingWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            if String::from_utf8_lossy(buf).contains(self.needle) {
                return Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed"));
            }
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn render_report_reports_a_failure_on_each_line() {
        let report = update::Report {
            name: "repo".to_owned(),
            steps: vec![
                update::Step::Fetch {
                    remote: "origin".to_owned(),
                    outcome: update::FetchOutcome::Changed(vec![(
                        update::Kind::NewBranch,
                        vec!["feat".to_owned()],
                    )]),
                },
                update::Step::Fetch {
                    remote: "upstream".to_owned(),
                    outcome: update::FetchOutcome::UpToDate,
                },
                update::Step::Branch {
                    name: "main".to_owned(),
                    outcome: update::BranchOutcome::Updated {
                        from: "1111111111111111111111111111111111111111".to_owned(),
                        to: "2222222222222222222222222222222222222222".to_owned(),
                    },
                },
                update::Step::Branch {
                    name: "side".to_owned(),
                    outcome: update::BranchOutcome::UpToDate,
                },
                update::Step::Branch {
                    name: "old".to_owned(),
                    outcome: update::BranchOutcome::Skipped(
                        "a fast-forward is not possible".to_owned(),
                    ),
                },
                update::Step::Error("No remotes.".to_owned()),
            ],
        };
        // Each needle sits inside one write of its line: a formatted
        // line goes out in pieces, so a phrase spanning two pieces
        // never appears in a single write.
        for needle in [
            "repo",
            "new branches: feat.",
            "upstream",
            "main",
            "side",
            "old",
            "No remotes.",
        ] {
            let mut out = LineRefusingWriter { needle };
            assert!(
                render_report(&report, false, &mut out).is_err(),
                "needle: {needle}"
            );
        }
        let report = update::Report {
            name: "repo".to_owned(),
            steps: vec![
                update::Step::Fetch {
                    remote: "origin".to_owned(),
                    outcome: update::FetchOutcome::Error(vec!["fatal: boom".to_owned()]),
                },
                update::Step::Branch {
                    name: "main".to_owned(),
                    outcome: update::BranchOutcome::Error("error: failed".to_owned()),
                },
            ],
        };
        for needle in ["fatal: boom", "error: failed"] {
            let mut out = LineRefusingWriter { needle };
            assert!(
                render_report(&report, false, &mut out).is_err(),
                "needle: {needle}"
            );
        }
    }

    #[test]
    fn render_report_reports_a_failed_write() {
        let report = update::Report {
            name: "repo".to_owned(),
            steps: vec![update::Step::Fetch {
                remote: "empty".to_owned(),
                outcome: update::FetchOutcome::NoRefspec,
            }],
        };
        let mut out = SkipLineRefusingWriter;
        assert!(render_report(&report, false, &mut out).is_err());
    }
}
