use super::*;

/// A writer whose writes always fail, shared by the crate's tests.
pub(crate) struct FailingWriter;

impl Write for FailingWriter {
    /// Fail every write, to hit the error path on the first line.
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let _ = buf;
        Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed"))
    }

    /// Flush succeeds, so a failure always comes from a write.
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Guards that relative, absolute and `~` inputs print as absolute paths with the right noun.
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

/// Guards that a header write error propagates, so a closed pipe stops the run.
#[test]
fn print_header_reports_a_failed_write() {
    let mut out = FailingWriter;
    assert!(print_header(&mut out, false, "/x", None, Path::new("/"), 1).is_err());
}

/// Guards that disabled colour adds nothing; piped output carries no escape codes.
#[test]
fn paint_wraps_the_text_in_the_codes_when_enabled() {
    assert_eq!(paint(true, RED, "x"), format!("{RED}x{RESET}"));
    assert_eq!(paint(false, RED, "x"), "x");
}

/// Guards the wording of the fetch, branch and error lines; users read them every run.
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

/// Guards the wording of the refspec skip and git error lines; failures stay visible to the user.
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

/// A writer refusing any write containing `needle`, so a render fails on one chosen line.
struct LineRefusingWriter {
    needle: &'static str,
}

impl Write for LineRefusingWriter {
    /// Fail only on writes containing the needle, so each line's error path is hit alone.
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if String::from_utf8_lossy(buf).contains(self.needle) {
            return Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed"));
        }
        Ok(buf.len())
    }

    /// Flush succeeds, so a failure always comes from a write.
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Guards that every write in `render_report` propagates; no line drops an I/O failure.
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
    // Each needle fits one write of its line: a formatted line goes out in pieces, so a phrase
    // spanning two pieces never appears in a single write.
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

/// Guards that a write error midway through a report, not only on the first line, propagates.
#[test]
fn render_report_reports_a_failed_write() {
    let report = update::Report {
        name: "repo".to_owned(),
        steps: vec![update::Step::Fetch {
            remote: "empty".to_owned(),
            outcome: update::FetchOutcome::NoRefspec,
        }],
    };
    let mut out = LineRefusingWriter { needle: "refspec" };
    assert!(render_report(&report, false, &mut out).is_err());
}
