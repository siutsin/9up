use std::env;
use std::ffi::OsString;
use std::num::NonZeroUsize;

use clap::{Args, Parser, Subcommand};

/// The default job bound: the work waits on subprocesses and the network,
/// not the CPU, so the bound is not the core count.
pub const DEFAULT_JOBS: NonZeroUsize = NonZeroUsize::new(8).unwrap();

#[derive(Parser)]
#[command(name = "9up", version)]
pub struct Cli {
    /// The config file to use. An empty value uses the default path.
    #[arg(short = 'C', long = "config", global = true, value_name = "PATH")]
    pub config_file: Option<String>,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Args)]
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

    /// Run at most this many repositories at once, across all paths.
    #[arg(short, long, default_value_t = DEFAULT_JOBS)]
    pub jobs: NonZeroUsize,
}

impl Default for UpdateArgs {
    /// Give the bare form the default job bound, so it pools like an explicit update.
    fn default() -> Self {
        Self {
            paths: Vec::new(),
            current_only: false,
            fetch_only: false,
            prune: false,
            depth: 0,
            jobs: DEFAULT_JOBS,
        }
    }
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

        /// Run at most this many repositories at once, across all paths.
        #[arg(short, long, default_value_t = DEFAULT_JOBS)]
        jobs: NonZeroUsize,

        /// The command and its arguments, after `--`.
        #[arg(last = true, required = true, num_args = 1..)]
        command: Vec<String>,
    },
}

const COMMANDS: [&str; 7] = ["update", "add", "delete", "list", "clean", "exec", "help"];

/// Make `9up [paths...]` mean `9up update [paths...]`: insert `update` after the program name
/// when the first positional is not a command. A flattened update struct cannot: its path list
/// swallows the command name as a path.
pub fn argv() -> Vec<OsString> {
    insert_update(env::args_os().collect())
}

/// Split from `argv` so tests can feed the shim any argument list, not the real process ones.
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
        if matches!(
            &*token,
            "-C" | "--config" | "-t" | "--depth" | "-j" | "--jobs"
        ) {
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
        Some(_) if after_separator => true,
        Some(index) => !COMMANDS.contains(&args[index].to_string_lossy().as_ref()),
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

/// Spot pathless update flags, so `9up -p` runs an update instead of failing in clap.
fn is_update_option(token: &str) -> bool {
    if token.starts_with("--") {
        return matches!(
            token,
            "--current-only" | "--fetch-only" | "--prune" | "--depth" | "--jobs"
        ) || token.starts_with("--depth=")
            || token.starts_with("--jobs=");
    }
    // Attached values: -t3, -t-1, -t=3 and -j3, -j=3.
    if let Some(rest) = token
        .strip_prefix("-t")
        .or_else(|| token.strip_prefix("-j"))
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
            .all(|c| matches!(c, 'c' | 'f' | 'p' | 't' | 'j'))
}

#[cfg(test)]
mod tests;
