use super::*;

/// Run the shim on string slices, so each case reads as plain argv in, argv out.
fn shim(args: &[&str]) -> Vec<String> {
    insert_update(args.iter().map(OsString::from).collect())
        .iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect()
}

/// Guards the most used bare `9up <paths>` form; a flag value like `-1` must not stop the scan.
#[test]
fn bare_paths_gain_the_update_command() {
    assert_eq!(shim(&["9up", "a", "b"]), ["9up", "update", "a", "b"]);
    assert_eq!(shim(&["9up", "-c", "a"]), ["9up", "update", "-c", "a"]);
    assert_eq!(
        shim(&["9up", "-t", "-1", "a"]),
        ["9up", "update", "-t", "-1", "a"]
    );
}

/// Guards that commands, `--config` values and `--version` pass unrewritten; rewriting breaks them.
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

/// Guards `9up -c` and attached depth forms; clap rejects them without the inserted `update`.
#[test]
fn bare_options_without_a_path_mean_update() {
    assert_eq!(shim(&["9up", "-c"]), ["9up", "update", "-c"]);
    assert_eq!(shim(&["9up", "-p", "-f"]), ["9up", "update", "-p", "-f"]);
    assert_eq!(shim(&["9up", "-t3"]), ["9up", "update", "-t3"]);
    assert_eq!(shim(&["9up", "-t-1"]), ["9up", "update", "-t-1"]);
    assert_eq!(shim(&["9up", "-t=3"]), ["9up", "update", "-t=3"]);
    assert_eq!(shim(&["9up", "--depth=2"]), ["9up", "update", "--depth=2"]);
    assert_eq!(shim(&["9up", "-j"]), ["9up", "update", "-j"]);
    assert_eq!(shim(&["9up", "-j2"]), ["9up", "update", "-j2"]);
    assert_eq!(shim(&["9up", "-j=2"]), ["9up", "update", "-j=2"]);
    assert_eq!(shim(&["9up", "--jobs=2"]), ["9up", "update", "--jobs=2"]);
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

/// Guards the long update flags; they take a separate code path from short flag clusters.
#[test]
fn a_bare_long_option_means_update() {
    assert_eq!(
        shim(&["9up", "--fetch-only"]),
        ["9up", "update", "--fetch-only"]
    );
    assert_eq!(shim(&["9up", "--prune"]), ["9up", "update", "--prune"]);
}

/// Guards that a depth value is skipped like a config value; otherwise `list` as a depth
/// value reads as the list command and clap blames the wrong thing.
#[test]
fn a_depth_value_is_not_a_command_word() {
    assert_eq!(
        shim(&["9up", "-C", "f", "--depth", "list"]),
        ["9up", "update", "-C", "f", "--depth", "list"]
    );
    assert_eq!(
        shim(&["9up", "-t", "list"]),
        ["9up", "update", "-t", "list"]
    );
}

/// Guards that a jobs value is skipped like a depth value; otherwise `list` as a jobs
/// value reads as the list command and clap blames the wrong thing.
#[test]
fn a_jobs_value_is_not_a_command_word() {
    assert_eq!(
        shim(&["9up", "-j", "list"]),
        ["9up", "update", "-j", "list"]
    );
    assert_eq!(
        shim(&["9up", "--jobs", "list"]),
        ["9up", "update", "--jobs", "list"]
    );
    assert_eq!(
        shim(&["9up", "-j", "2", "a"]),
        ["9up", "update", "-j", "2", "a"]
    );
}

/// Pull the jobs value out of a parsed CLI, so the parse cases read as plain values.
fn parsed_jobs(args: &[&str]) -> NonZeroUsize {
    match Cli::try_parse_from(args).unwrap().command {
        Some(Command::Update(args)) => args.jobs,
        Some(Command::Exec { jobs, .. }) => jobs,
        _ => panic!("expected update or exec"),
    }
}

/// Guards that jobs parses for both pooled commands, defaults to the bound, and
/// that the struct default carries the same bound for the bare form.
#[test]
fn jobs_parses_and_defaults_for_update_and_exec() {
    assert_eq!(parsed_jobs(&["9up", "update"]), DEFAULT_JOBS);
    assert_eq!(parsed_jobs(&["9up", "update", "-j2"]).get(), 2);
    assert_eq!(
        parsed_jobs(&["9up", "exec", "--jobs", "3", "--", "true"]).get(),
        3
    );
    assert_eq!(parsed_jobs(&["9up", "exec", "--", "true"]), DEFAULT_JOBS);
    assert_eq!(UpdateArgs::default().jobs, DEFAULT_JOBS);
}

/// Guards that zero jobs is a usage error on both pooled commands; a zero bound
/// could never run a repository.
#[test]
fn jobs_rejects_zero() {
    assert!(Cli::try_parse_from(["9up", "update", "--jobs", "0"]).is_err());
    assert!(Cli::try_parse_from(["9up", "exec", "-j", "0", "--", "true"]).is_err());
}

/// Guards a directory named like a command; after `--` it is still updated as a path.
#[test]
fn a_path_after_the_separator_is_a_path() {
    assert_eq!(
        shim(&["9up", "--", "list"]),
        ["9up", "update", "--", "list"]
    );
    assert_eq!(shim(&["9up", "--", "a"]), ["9up", "update", "--", "a"]);
}
