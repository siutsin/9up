use std::env;
use std::ffi::OsString;

use clap::{Args, Parser, Subcommand};

#[derive(Parser)]
#[command(name = "9up", version)]
pub struct Cli {
    /// The config file to use. An empty value uses the default path.
    #[arg(short = 'C', long = "config", global = true, value_name = "PATH")]
    pub config_file: Option<String>,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Args, Default)]
pub struct UpdateArgs {
    /// Update the repositories under these paths. No paths means the configured repositories.
    pub paths: Vec<String>,

    /// Fetch only the remote the current branch tracks.
    #[arg(short, long)]
    pub current_only: bool,

    /// Only fetch. Do not update any branch.
    #[arg(short, long)]
    pub fetch_only: bool,

    /// Pass --prune to each fetch.
    #[arg(short, long)]
    pub prune: bool,

    /// Search depth below each path. A negative value means no limit.
    #[arg(short = 't', long, default_value_t = 3, allow_negative_numbers = true)]
    pub depth: i32,
}

#[derive(Subcommand)]
pub enum Command {
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
pub fn argv() -> Vec<OsString> {
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
            shim(&["9up", "-C", "f", "list"]),
            ["9up", "-C", "f", "list"]
        );
        assert_eq!(
            shim(&["9up", "--config", "f", "list"]),
            ["9up", "--config", "f", "list"]
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
            shim(&["9up", "-C", "f", "-c"]),
            ["9up", "update", "-C", "f", "-c"]
        );
        assert_eq!(
            shim(&["9up", "--config", "f", "-c"]),
            ["9up", "update", "--config", "f", "-c"]
        );
        assert_eq!(shim(&["9up", "-C", "f"]), ["9up", "-C", "f"]);
    }

    #[test]
    fn a_bare_long_option_means_update() {
        assert_eq!(
            shim(&["9up", "--fetch-only"]),
            ["9up", "update", "--fetch-only"]
        );
        assert_eq!(shim(&["9up", "--prune"]), ["9up", "update", "--prune"]);
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
