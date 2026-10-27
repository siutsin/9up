mod config;
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
    /// The config file to use. An empty value uses the default path.
    #[arg(short = 'C', long = "config", global = true, value_name = "PATH")]
    config_file: Option<String>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Args, Default)]
struct UpdateArgs {
    /// Update the repositories under these paths. No paths means the configured repositories.
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

#[derive(Subcommand)]
enum Command {
    /// Update repositories. The default command.
    Update(UpdateArgs),

    /// Add paths to the config file.
    Add {
        #[arg(num_args = 1..)]
        paths: Vec<String>,
    },

    /// Delete paths from the config file.
    Delete {
        #[arg(num_args = 1..)]
        paths: Vec<String>,
    },

    /// List the configured paths.
    List,

    /// Remove configured paths that no longer exist.
    Clean,

    /// Run a command in each repository.
    Exec {
        /// Run in the repositories under these paths. No paths means the configured repositories.
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
        if token == "-C" || token == "--config" {
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
    // `list` and `clean` never need the cwd (configured paths are
    // absolute), so a deleted or unreadable cwd must not fail them.
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
    let config_file = config::file_path(cli.config_file.as_deref(), config_dir.as_deref());
    let mut app = App {
        sys: Box::new(RealSys),
        cwd,
        home: dirs::home_dir(),
        config_file,
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
        None => update(
            app,
            &UpdateArgs {
                depth: 3,
                ..Default::default()
            },
        ),
        Some(Command::Exec {
            paths,
            depth,
            command,
        }) => exec(app, paths, *depth, command),
    }
}

fn add(app: &mut App, paths: &[String]) -> io::Result<ExitCode> {
    let out = &mut *app.out;
    let mut entries = config::load(&*app.sys, &app.config_file)?;
    for raw in paths {
        let normalized = config::normalize(raw, &app.cwd);
        if entries.contains(&normalized) {
            writeln!(out, "{normalized} is already in the config file.")?;
        } else {
            writeln!(out, "Added {normalized}.")?;
            entries.push(normalized);
        }
    }
    config::save(&*app.sys, &app.config_file, &entries)?;
    Ok(ExitCode::SUCCESS)
}

fn delete(app: &mut App, paths: &[String]) -> io::Result<ExitCode> {
    let out = &mut *app.out;
    let mut entries = config::load(&*app.sys, &app.config_file)?;
    let mut changed = false;
    for raw in paths {
        let normalized = config::normalize(raw, &app.cwd);
        if let Some(index) = entries.iter().position(|entry| entry == &normalized) {
            entries.remove(index);
            changed = true;
            writeln!(out, "Deleted {normalized}.")?;
        } else {
            writeln!(out, "{normalized} is not in the config file.")?;
        }
    }
    if changed {
        config::save(&*app.sys, &app.config_file, &entries)?;
    }
    Ok(ExitCode::SUCCESS)
}

fn list(app: &mut App) -> io::Result<ExitCode> {
    let out = &mut *app.out;
    let entries = config::load(&*app.sys, &app.config_file)?;
    if entries.is_empty() {
        writeln!(out, "No repositories configured.")?;
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
    for_each_repo(app, &inputs, args.depth, |app, repo| {
        let report = update::update_repo(&*app.sys, &repo.path, &repo.name, &options);
        render_report(&report, app.colour, &mut *app.out)
    })?;
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
    let entries = config::load(&*app.sys, &app.config_file)?;
    if entries.is_empty() {
        writeln!(out, "No repositories configured.")?;
        return Ok(ExitCode::SUCCESS);
    }
    let mut kept = Vec::new();
    let mut removed = false;
    for entry in &entries {
        // A comment entry is not a path, so the liveness test below
        // cannot judge it. The plan keeps it.
        if discover::comment_text(entry).is_some() {
            kept.push(entry.clone());
            continue;
        }
        let expanded = discover::expand_home(entry, app.home.as_deref());
        if discover::path_is_live(&expanded) {
            kept.push(entry.clone());
        } else {
            removed = true;
            writeln!(out, "Removed {entry}.")?;
        }
    }
    if removed {
        config::save(&*app.sys, &app.config_file, &kept)?;
    } else {
        writeln!(out, "Nothing to clean.")?;
    }
    Ok(ExitCode::SUCCESS)
}

fn exec(app: &mut App, paths: &[String], depth: i32, command: &[String]) -> io::Result<ExitCode> {
    let inputs: Vec<String> = if paths.is_empty() {
        // No hint here: an exec run with no configured paths prints nothing.
        config::load(&*app.sys, &app.config_file)?
    } else {
        paths.to_vec()
    };
    for_each_repo(app, &inputs, depth, |app, repo| {
        let report = update::exec_repo(&*app.sys, &repo.path, &repo.name, command);
        let out = &mut *app.out;
        writeln!(out, "  {}", paint(app.colour, BOLD_GREEN, &report.name))?;
        if let Some(error) = &report.error {
            red_line(out, app.colour, error)
        } else {
            for line in report.stdout.lines().chain(report.stderr.lines()) {
                writeln!(out, "    {line}")?;
            }
            Ok(())
        }
    })?;
    Ok(ExitCode::SUCCESS)
}

/// The walk `update` and `exec` share: resolve each input in turn, print
/// its group header, and run `op` on every repository found. A comment
/// input prints as a line of its own; a bad path prints in red.
fn for_each_repo(
    app: &mut App,
    inputs: &[String],
    depth: i32,
    mut op: impl FnMut(&mut App, &discover::Repo) -> io::Result<()>,
) -> io::Result<()> {
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
                for repo in &repos {
                    op(app, repo)?;
                }
            }
            discover::Resolved::Comment(text) => {
                if !text.is_empty() {
                    writeln!(&mut *app.out, "{text}")?;
                }
            }
            discover::Resolved::PathError(message) => {
                writeln!(&mut *app.out, "{}", paint(app.colour, RED, &message))?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
