mod bookmarks;
mod discover;

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
    #[arg(short, long, default_value_t = 3, allow_negative_numbers = true)]
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
        #[arg(short, long, default_value_t = 3, allow_negative_numbers = true)]
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
    let _ = writeln!(out, "9up");
    let _ = writeln!(out);
    match run(&cli, &mut out) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("9up: {err}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: &Cli, out: &mut impl Write) -> io::Result<ExitCode> {
    let path = bookmark_path(cli);
    match &cli.command {
        Some(Command::Add { paths }) => add(&path, paths, out),
        Some(Command::Delete { paths }) => delete(&path, paths, out),
        Some(Command::List) => list(&path, out),
        Some(Command::Clean) => clean(&path, out),
        Some(Command::Update(args)) => update(args, &path, out),
        None => update(&UpdateArgs::default(), &path, out),
        Some(Command::Exec { .. }) => todo!("exec arrives in step 3"),
    }
}

fn add(path: &Path, paths: &[String], out: &mut impl Write) -> io::Result<ExitCode> {
    let cwd = env::current_dir()?;
    let mut entries = bookmarks::load(path);
    for raw in paths {
        let normalized = bookmarks::normalize(raw, &cwd);
        if bookmarks::has(&entries, &normalized) {
            let _ = writeln!(out, "{normalized} is already bookmarked.");
        } else {
            let _ = writeln!(out, "Added {normalized}.");
            entries.push(normalized);
        }
    }
    bookmarks::save(path, &entries)?;
    Ok(ExitCode::SUCCESS)
}

fn delete(path: &Path, paths: &[String], out: &mut impl Write) -> io::Result<ExitCode> {
    let cwd = env::current_dir()?;
    let mut entries = bookmarks::load(path);
    let mut changed = false;
    for raw in paths {
        let normalized = bookmarks::normalize(raw, &cwd);
        if let Some(index) = entries.iter().position(|entry| entry == &normalized) {
            entries.remove(index);
            changed = true;
            let _ = writeln!(out, "Deleted {normalized}.");
        } else {
            let _ = writeln!(out, "{normalized} is not bookmarked.");
        }
    }
    if changed {
        bookmarks::save(path, &entries)?;
    }
    Ok(ExitCode::SUCCESS)
}

fn list(path: &Path, out: &mut impl Write) -> io::Result<ExitCode> {
    let entries = bookmarks::load(path);
    if entries.is_empty() {
        let _ = writeln!(out, "No bookmarks.");
    }
    for entry in &entries {
        let _ = writeln!(out, "{entry}");
    }
    Ok(ExitCode::SUCCESS)
}

const BOLD_WHITE: &str = "\u{1b}[1;37m";
const BOLD_GREEN: &str = "\u{1b}[1;32m";
const RED: &str = "\u{1b}[31m";
const RESET: &str = "\u{1b}[0m";

fn paint(enabled: bool, code: &str, text: &str) -> String {
    if enabled {
        format!("{code}{text}{RESET}")
    } else {
        text.to_owned()
    }
}

/// Step 1 renders discovery only. Step 2 replaces each name with a report.
fn update(args: &UpdateArgs, bookmark_file: &Path, out: &mut impl Write) -> io::Result<ExitCode> {
    let cwd = env::current_dir()?;
    let inputs: Vec<String> = if args.paths.is_empty() {
        let entries = bookmarks::load(bookmark_file);
        if entries.is_empty() {
            let _ = writeln!(out, "No bookmarks. Add one with `9up add <path>`.");
            return Ok(ExitCode::SUCCESS);
        }
        entries
    } else {
        args.paths.clone()
    };
    let colour = io::stdout().is_terminal();
    for input in &inputs {
        match discover::resolve(input, &cwd, args.depth) {
            discover::Resolved::Repos(repos) => {
                let expanded = discover::expand_home(input);
                let header = bookmarks::normalize(&expanded.display().to_string(), &cwd);
                let count = repos.len();
                let noun = if count == 1 {
                    "repository"
                } else {
                    "repositories"
                };
                let _ = writeln!(
                    out,
                    "{} ({count} {noun})",
                    paint(colour, BOLD_WHITE, &header)
                );
                for repo in &repos {
                    let _ = writeln!(out, "  {}", paint(colour, BOLD_GREEN, &repo.name));
                }
            }
            discover::Resolved::Comment(text) => {
                if !text.is_empty() {
                    let _ = writeln!(out, "{text}");
                }
            }
            discover::Resolved::PathError(message) => {
                let _ = writeln!(out, "{}", paint(colour, RED, &message));
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn clean(path: &Path, out: &mut impl Write) -> io::Result<ExitCode> {
    let entries = bookmarks::load(path);
    if entries.is_empty() {
        let _ = writeln!(out, "No bookmarks.");
        return Ok(ExitCode::SUCCESS);
    }
    let mut kept = Vec::new();
    let mut removed = false;
    for entry in &entries {
        let expanded = discover::expand_home(entry);
        let live =
            expanded.is_dir() || !discover::glob_paths(&expanded.display().to_string()).is_empty();
        if live {
            kept.push(entry.clone());
        } else {
            removed = true;
            let _ = writeln!(out, "Removed {entry}.");
        }
    }
    if removed {
        bookmarks::save(path, &kept)?;
    } else {
        let _ = writeln!(out, "Nothing to clean.");
    }
    Ok(ExitCode::SUCCESS)
}

/// The bookmark file for this run: the `-b` value, or the default path
/// under the XDG config directory.
fn bookmark_path(cli: &Cli) -> PathBuf {
    if let Some(path) = cli.bookmark_file.as_deref().filter(|path| !path.is_empty()) {
        return PathBuf::from(path);
    }
    let config = env::var("XDG_CONFIG_HOME")
        .ok()
        .filter(|value| !value.is_empty())
        .map(|value| discover::expand_home(&value))
        .unwrap_or_else(|| home_dir().join(".config"));
    config.join("gitup").join("bookmarks")
}

fn home_dir() -> PathBuf {
    env::home_dir().unwrap_or_default()
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
        assert_eq!(shim(&["9up", "--depth=2"]), ["9up", "update", "--depth=2"]);
        assert_eq!(
            shim(&["9up", "-b", "f", "-c"]),
            ["9up", "update", "-b", "f", "-c"]
        );
        assert_eq!(shim(&["9up", "-b", "f"]), ["9up", "-b", "f"]);
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
