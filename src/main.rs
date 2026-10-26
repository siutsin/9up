mod bookmarks;
mod discover;
mod sys;
mod update;

use sys::{App, RealSys};

use std::env;
use std::ffi::OsString;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};

#[derive(Parser)]
#[command(name = "9up", version)]
struct Cli {
    /// The bookmark file to use. An empty value uses the default path.
    #[arg(short, long, global = true, value_name = "PATH")]
    bookmark_file: Option<String>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Args)]
struct UpdateArgs {
    /// Update the repositories under these paths. No paths means the bookmarks.
    paths: Vec<String>,

    /// Fetch only the remote the current branch tracks.
    #[arg(short, long)]
    current_only: bool,

    /// Only fetch. Do not update any branch.
    #[arg(short, long)]
    fetch_only: bool,

    /// Pass --prune to each fetch.
    #[arg(short, long)]
    prune: bool,

    /// Search depth below each path. A negative value means no limit.
    #[arg(short = 't', long, default_value_t = 3, allow_negative_numbers = true)]
    depth: i32,
}

impl Default for UpdateArgs {
    fn default() -> Self {
        Self {
            paths: Vec::new(),
            current_only: false,
            fetch_only: false,
            prune: false,
            depth: 3,
        }
    }
}

#[derive(Subcommand)]
enum Command {
    /// Update repositories. The default command.
    Update(UpdateArgs),

    /// Add paths to the bookmark file.
    Add {
        #[arg(num_args = 1..)]
        paths: Vec<String>,
    },

    /// Delete paths from the bookmark file.
    Delete {
        #[arg(num_args = 1..)]
        paths: Vec<String>,
    },

    /// List the bookmarks.
    List,

    /// Remove bookmarks whose paths no longer exist.
    Clean,

    /// Run a command in each repository.
    Exec {
        /// Run in the repositories under these paths. No paths means the bookmarks.
        paths: Vec<String>,

        /// Search depth below each path. A negative value means no limit.
        #[arg(short = 't', long, default_value_t = 3, allow_negative_numbers = true)]
        depth: i32,

        /// The command and its arguments, after `--`.
        #[arg(last = true, required = true, num_args = 1..)]
        command: Vec<String>,
    },
}

const COMMANDS: [&str; 7] = ["update", "add", "delete", "list", "clean", "exec", "help"];

/// The bare form: `9up [paths...]` means `9up update [paths...]`. When the
/// first positional argument is not a command name, insert `update` after
/// the program name. A flattened update struct cannot do this: its path
/// list swallows the command name as a path.
fn argv() -> Vec<OsString> {
    insert_update(env::args_os().collect())
}

fn insert_update(mut args: Vec<OsString>) -> Vec<OsString> {
    let mut index = 1;
    let mut positional = None;
    let mut after_separator = false;
    while index < args.len() {
        let token = args[index].to_string_lossy();
        if token == "--" {
            positional = (index + 1 < args.len()).then_some(index + 1);
            after_separator = true;
            break;
        }
        if token == "-b" || token == "--bookmark-file" {
            index += 2;
            continue;
        }
        if token.starts_with('-') {
            index += 1;
            continue;
        }
        positional = Some(index);
        break;
    }
    let insert = match positional {
        // A path after `--` is a path, even when it is named like a command.
        Some(_) if after_separator => true,
        Some(index) => !COMMANDS.contains(&args[index].to_string_lossy().as_ref()),
        // No path: an update option with no command still means update.
        None => args
            .iter()
            .skip(1)
            .any(|arg| is_update_option(&arg.to_string_lossy())),
    };
    if insert {
        args.insert(1, OsString::from("update"));
    }
    args
}

fn is_update_option(token: &str) -> bool {
    if token.starts_with("--") {
        return matches!(
            token,
            "--current-only" | "--fetch-only" | "--prune" | "--depth"
        ) || token.starts_with("--depth=");
    }
    // Attached depth: -t3, -t-1 and -t=3.
    if let Some(rest) = token.strip_prefix("-t")
        && rest
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_digit() || c == '-' || c == '=')
    {
        return true;
    }
    token.len() > 1
        && token.starts_with('-')
        && token[1..]
            .chars()
            .all(|c| matches!(c, 'c' | 'f' | 'p' | 't'))
}

fn main() -> ExitCode {
    let cli = Cli::parse_from(argv());
    let stdout = io::stdout();
    let mut out = stdout.lock();
    if let Err(err) = writeln!(out, "9up").and_then(|()| writeln!(out)) {
        return if err.kind() == io::ErrorKind::BrokenPipe {
            ExitCode::SUCCESS
        } else {
            eprintln!("9up: {err}");
            ExitCode::FAILURE
        };
    }
    // The composition root: every service and resolved value is built
    // here and passed down in the context.
    // `list` and `clean` never need the cwd (bookmarks are absolute),
    // so a deleted or unreadable cwd must not fail them.
    let cwd = match env::current_dir() {
        Ok(cwd) => cwd,
        Err(err) => match &cli.command {
            Some(Command::List) | Some(Command::Clean) => PathBuf::from("/"),
            _ => {
                eprintln!("9up: {err}");
                return ExitCode::FAILURE;
            }
        },
    };
    let config_dir = dirs::config_dir();
    let bookmark_file = bookmark_path(&cli, config_dir.as_deref());
    let mut app = App {
        sys: Box::new(RealSys),
        cwd,
        home: dirs::home_dir(),
        bookmark_file,
        colour: io::stdout().is_terminal(),
        out: &mut out,
    };
    match run(&mut app, &cli) {
        Ok(code) => code,
        // A closed pipe ends the run quietly, with the success code.
        Err(err) if err.kind() == io::ErrorKind::BrokenPipe => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("9up: {err}");
            ExitCode::FAILURE
        }
    }
}

fn run(app: &mut App, cli: &Cli) -> io::Result<ExitCode> {
    match &cli.command {
        Some(Command::Add { paths }) => add(app, paths),
        Some(Command::Delete { paths }) => delete(app, paths),
        Some(Command::List) => list(app),
        Some(Command::Clean) => clean(app),
        Some(Command::Update(args)) => update(app, args),
        None => update(app, &UpdateArgs::default()),
        Some(Command::Exec {
            paths,
            depth,
            command,
        }) => exec(app, paths, *depth, command),
    }
}

fn add(app: &mut App, paths: &[String]) -> io::Result<ExitCode> {
    let out = &mut *app.out;
    let mut entries = bookmarks::load(&*app.sys, &app.bookmark_file);
    for raw in paths {
        let normalized = bookmarks::normalize(raw, &app.cwd);
        if bookmarks::has(&entries, &normalized) {
            writeln!(out, "{normalized} is already bookmarked.")?;
        } else {
            writeln!(out, "Added {normalized}.")?;
            entries.push(normalized);
        }
    }
    bookmarks::save(&*app.sys, &app.bookmark_file, &entries)?;
    Ok(ExitCode::SUCCESS)
}

fn delete(app: &mut App, paths: &[String]) -> io::Result<ExitCode> {
    let out = &mut *app.out;
    let mut entries = bookmarks::load(&*app.sys, &app.bookmark_file);
    let mut changed = false;
    for raw in paths {
        let normalized = bookmarks::normalize(raw, &app.cwd);
        if let Some(index) = entries.iter().position(|entry| entry == &normalized) {
            entries.remove(index);
            changed = true;
            writeln!(out, "Deleted {normalized}.")?;
        } else {
            writeln!(out, "{normalized} is not bookmarked.")?;
        }
    }
    if changed {
        bookmarks::save(&*app.sys, &app.bookmark_file, &entries)?;
    }
    Ok(ExitCode::SUCCESS)
}

fn list(app: &mut App) -> io::Result<ExitCode> {
    let out = &mut *app.out;
    let entries = bookmarks::load(&*app.sys, &app.bookmark_file);
    if entries.is_empty() {
        writeln!(out, "No bookmarks.")?;
    }
    for entry in &entries {
        writeln!(out, "{entry}")?;
    }
    Ok(ExitCode::SUCCESS)
}

const BOLD_WHITE: &str = "\u{1b}[1;37m";
const BOLD_GREEN: &str = "\u{1b}[1;32m";
const RED: &str = "\u{1b}[31m";
const CYAN: &str = "\u{1b}[36m";
const RESET: &str = "\u{1b}[0m";

fn paint(enabled: bool, code: &str, text: &str) -> String {
    if enabled {
        format!("{code}{text}{RESET}")
    } else {
        text.to_owned()
    }
}

fn update(app: &mut App, args: &UpdateArgs) -> io::Result<ExitCode> {
    let out = &mut *app.out;
    let inputs: Vec<String> = if args.paths.is_empty() {
        let entries = bookmarks::load(&*app.sys, &app.bookmark_file);
        if entries.is_empty() {
            writeln!(out, "No bookmarks. Add one with `9up add <path>`.")?;
            return Ok(ExitCode::SUCCESS);
        }
        entries
    } else {
        args.paths.clone()
    };
    let colour = app.colour;
    for input in &inputs {
        match discover::resolve(input, &app.cwd, app.home.as_deref(), args.depth) {
            discover::Resolved::Repos(repos) => {
                print_header(
                    out,
                    colour,
                    input,
                    app.home.as_deref(),
                    &app.cwd,
                    repos.len(),
                )?;
                let options = update::Options {
                    current_only: args.current_only,
                    fetch_only: args.fetch_only,
                    prune: args.prune,
                };
                for repo in &repos {
                    let report = update::update_repo(&*app.sys, &repo.path, &repo.name, &options);
                    render_report(&report, colour, out)?;
                }
            }
            discover::Resolved::Comment(text) => {
                if !text.is_empty() {
                    writeln!(out, "{text}")?;
                }
            }
            discover::Resolved::PathError(message) => {
                writeln!(out, "{}", paint(colour, RED, &message))?;
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn print_header(
    out: &mut dyn Write,
    colour: bool,
    input: &str,
    home: Option<&Path>,
    cwd: &Path,
    count: usize,
) -> io::Result<()> {
    let expanded = discover::expand_home(input, home);
    let header = bookmarks::normalize(&expanded.display().to_string(), cwd);
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

fn red_line(out: &mut dyn Write, colour: bool, text: &str) -> io::Result<()> {
    writeln!(out, "    {}", paint(colour, RED, text))?;
    Ok(())
}

fn render_report(report: &update::Report, colour: bool, out: &mut dyn Write) -> io::Result<()> {
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

fn clean(app: &mut App) -> io::Result<ExitCode> {
    let out = &mut *app.out;
    let entries = bookmarks::load(&*app.sys, &app.bookmark_file);
    if entries.is_empty() {
        writeln!(out, "No bookmarks.")?;
        return Ok(ExitCode::SUCCESS);
    }
    let mut kept = Vec::new();
    let mut removed = false;
    for entry in &entries {
        let expanded = discover::expand_home(entry, app.home.as_deref());
        let live =
            expanded.is_dir() || !discover::glob_paths(&expanded.display().to_string()).is_empty();
        if live {
            kept.push(entry.clone());
        } else {
            removed = true;
            writeln!(out, "Removed {entry}.")?;
        }
    }
    if removed {
        bookmarks::save(&*app.sys, &app.bookmark_file, &kept)?;
    } else {
        writeln!(out, "Nothing to clean.")?;
    }
    Ok(ExitCode::SUCCESS)
}

/// The bookmark file for this run: the `-b` value, or the default path
/// under the XDG config directory.
fn bookmark_path(cli: &Cli, config_dir: Option<&Path>) -> PathBuf {
    if let Some(path) = cli.bookmark_file.as_deref().filter(|path| !path.is_empty()) {
        return PathBuf::from(path);
    }
    // The config directory comes from `dirs` (see main): on Linux a
    // relative XDG_CONFIG_HOME is ignored, as the specification
    // requires. No config directory at all falls back to `.config`.
    let config = config_dir
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from(".config"));
    config.join("gitup").join("bookmarks")
}

fn exec(app: &mut App, paths: &[String], depth: i32, command: &[String]) -> io::Result<ExitCode> {
    let out = &mut *app.out;
    let inputs: Vec<String> = if paths.is_empty() {
        // No hint here: an exec run with no bookmarks prints nothing.
        bookmarks::load(&*app.sys, &app.bookmark_file)
    } else {
        paths.to_vec()
    };
    let colour = app.colour;
    for input in &inputs {
        match discover::resolve(input, &app.cwd, app.home.as_deref(), depth) {
            discover::Resolved::Repos(repos) => {
                print_header(
                    out,
                    colour,
                    input,
                    app.home.as_deref(),
                    &app.cwd,
                    repos.len(),
                )?;
                for repo in &repos {
                    let report = update::exec_repo(&*app.sys, &repo.path, &repo.name, command);
                    writeln!(out, "  {}", paint(colour, BOLD_GREEN, &report.name))?;
                    if let Some(error) = &report.error {
                        red_line(out, colour, error)?;
                    } else {
                        for line in report.stdout.lines().chain(report.stderr.lines()) {
                            writeln!(out, "    {line}")?;
                        }
                    }
                }
            }
            discover::Resolved::Comment(text) => {
                if !text.is_empty() {
                    writeln!(out, "{text}")?;
                }
            }
            discover::Resolved::PathError(message) => {
                writeln!(out, "{}", paint(colour, RED, &message))?;
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shim(args: &[&str]) -> Vec<String> {
        insert_update(args.iter().map(OsString::from).collect())
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn bare_paths_gain_the_update_command() {
        assert_eq!(shim(&["9up", "a", "b"]), ["9up", "update", "a", "b"]);
        assert_eq!(shim(&["9up", "-c", "a"]), ["9up", "update", "-c", "a"]);
        assert_eq!(
            shim(&["9up", "-t", "-1", "a"]),
            ["9up", "update", "-t", "-1", "a"]
        );
    }

    #[test]
    fn commands_and_flags_pass_through() {
        assert_eq!(shim(&["9up"]), ["9up"]);
        assert_eq!(shim(&["9up", "list"]), ["9up", "list"]);
        assert_eq!(
            shim(&["9up", "-b", "f", "list"]),
            ["9up", "-b", "f", "list"]
        );
        assert_eq!(shim(&["9up", "--version"]), ["9up", "--version"]);
        assert_eq!(shim(&["9up", "help"]), ["9up", "help"]);
    }

    #[test]
    fn bare_options_without_a_path_mean_update() {
        assert_eq!(shim(&["9up", "-c"]), ["9up", "update", "-c"]);
        assert_eq!(shim(&["9up", "-p", "-f"]), ["9up", "update", "-p", "-f"]);
        assert_eq!(shim(&["9up", "-t3"]), ["9up", "update", "-t3"]);
        assert_eq!(shim(&["9up", "-t-1"]), ["9up", "update", "-t-1"]);
        assert_eq!(shim(&["9up", "-t=3"]), ["9up", "update", "-t=3"]);
        assert_eq!(shim(&["9up", "--depth=2"]), ["9up", "update", "--depth=2"]);
        assert_eq!(
            shim(&["9up", "-b", "f", "-c"]),
            ["9up", "update", "-b", "f", "-c"]
        );
        assert_eq!(shim(&["9up", "-b", "f"]), ["9up", "-b", "f"]);
    }

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

    #[test]
    fn a_path_after_the_separator_is_a_path() {
        assert_eq!(
            shim(&["9up", "--", "list"]),
            ["9up", "update", "--", "list"]
        );
        assert_eq!(shim(&["9up", "--", "a"]), ["9up", "update", "--", "a"]);
    }
}
