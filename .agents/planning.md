# 9up Plan

Status: the reference for a Rust CLI inspired by `gitup` 0.5.2. The implementation follows it.

9up is a new tool, not a port. It does the jobs `gitup` (the Python `git-repo-updater` by Ben Kurtovic) does, in
the same broad shape, because that shape works. Feature parity is the goal: the same jobs. Its state is its
own, a TOML config file (see Config File), not `gitup`'s bookmark file. Everything else is 9up's own, and
idiomatic Rust wins: where `gitup`'s shape and idiomatic Rust conflict, the plan follows Rust.

## Goal And Scope

| Item | Decision |
| --- | --- |
| Problem | Keep many local git repositories current without visiting each one. |
| Milestone 1 | The same jobs as `gitup` 0.5.2: update repositories under given paths, update the repositories listed in its config file, manage that config file, and run a command in each repository. Runs sequentially. |
| Milestone 2 | Concurrency. Not planned in detail here, and the architecture must not block it. |
| Repository name | `9up` |
| Cargo package name | `nine-up`. Cargo rejects a package name that starts with a digit. |
| Binary name | `9up`. A binary file name can start with a digit. |
| Version | `0.1.0` |
| Out of scope | Releases, a Homebrew formula, Windows support, and any feature `gitup` does not have. |

Two terms run through the plan. A path is a directory passed on the command line, or an entry in the config file. A repository
is a git repository found under a path. For a bare repository, the repository itself is the path's only
repository.

## Feature Spec

This section is the contract. Implementation follows it, and the tests derive from it.

### Commands

Subcommands, the idiomatic clap shape. One verb per run.

| Invocation | Job |
| --- | --- |
| `9up [paths...]` | Update the repositories under the paths, or the configured repositories when no path is given. The bare form of `update`, with the same options. |
| `9up update [paths...]` | The same, spelled out. |
| `9up add <paths...>` | Add paths to the config file. |
| `9up delete <paths...>` | Delete paths from the config file. |
| `9up list` | List the configured paths. |
| `9up clean` | Remove configured paths that no longer exist. |
| `9up exec [paths...] -- <command...>` | Run the command in each repository under the paths, or in the configured repositories when no path is given. Everything after `--` is the command and its arguments, as the shell split them. Takes `--depth`, with the same meaning as on `update`. |

A path named like a command (`list`, `clean`, `add`, `delete`, `exec`, `update`) cannot use the bare form.
Spell it out instead: `9up update list`, or `9up ./list`.

The `update` options:

| Option | Type | Default | Meaning |
| --- | --- | --- | --- |
| `-c`, `--current-only` | bool | `false` | Fetch only the remote that the current branch tracks. |
| `-f`, `--fetch-only` | bool | `false` | Only fetch. Do not update any branch. |
| `-p`, `--prune` | bool | `false` | Pass `--prune` to each fetch. |
| `-t`, `--depth` | value | `3` | Search depth below each path. Negative values search with no limit. The path itself is always checked, at any depth value. |

One global option, accepted by every command: `-C`, `--config <path>`, the config file to use. (`-c` is
`update`'s `--current-only`, so the config option takes `-C`.) The value is required. An empty value uses the
default path (see Config File).

One verb per run is the one combined-invocation loss against `gitup`: add-then-update takes two runs. Not
included: `gitup`'s `--selftest` (`cargo test` covers it) and its combined flag actions.

### Behaviour

1. A usage error exits with code 2 before any other output. Help and version print and exit with code 0.
   Every other run prints the banner first: `9up`, then a blank line.
2. Path resolution, for command-line paths and configured paths alike, in this order:
   - `~` at the start expands to the home directory. Only a leading `~` alone or `~/` expands. `~user` does
     not expand, and `$VAR` never expands.
   - An empty path prints a "doesn't exist" error.
   - A path that is a repository is its own only repository, whatever the depth.
   - A missing path whose first non-space character is `#` is a comment. Strip the leading spaces, then every
     leading `#`, then trim. The rest is the comment text. An empty rest prints nothing.
   - Any other missing path expands as a glob. No match, or a bad pattern, prints a "doesn't exist" error. A
     path with no glob characters matches when its own link exists, so a broken symbolic link prints a header
     with a count of 0.
   - An existing path that is not a directory prints an "isn't a repository" error. So does a directory when
     the depth is 0.
   - Any other directory is searched. The search follows symbolic links to directories and enters hidden
     directories. It skips a directory it cannot read, and a symbolic-link cycle ends that branch of the search
     (`walkdir` reports the loop as an error entry).
   - Depth: the path itself is depth 0. A repository directly below the path is depth 1. Depth `d` finds
     repositories down to `d` levels below the path. The search does not descend into a repository it found.
   - A directory with no repositories below it, including an empty directory, prints its header with a count
     of 0.
   - A path error names the path as given, after `~` expansion. It does not use the absolute form the header
     uses.
3. When an update or an `exec` run reaches the configured paths and the config file lists none, an update
   prints a hint line that says so. An `exec` run prints nothing.
4. A repository updates like this:
   - Pick the remotes: all of them, or with `--current-only` only the remote that the current branch tracks.
     That remote comes from the branch's configuration, so it works on an unborn branch.
   - A repository error ends the repository: it prints an error line, with no fetch and no branch step. The
     checks run in this order: with `--current-only`, a detached HEAD; then with `--current-only`, no tracked
     remote; then no remotes at all.
   - Fetch each picked remote in configuration order, and report what the fetch brought in: new branches,
     new tags, updated references, or up to date. A remote with no reference lines in its configuration
     prints a skip line for that remote.
   - The first failed fetch stops the fetches for that repository. The branch step still runs.
   - A fetch failure shows the `error:` and `fatal:` lines from git's standard error, with leading and
     trailing whitespace removed. When git prints no such line, the report shows the command and its exit
     status instead.
5. Branch step. It runs per repository after the fetches, unless `--fetch-only` is set, and covers every
   local branch in name order. `--current-only` does not change it.
   - A branch with no upstream is skipped silently and prints nothing. So is a branch whose upstream reference
     is missing locally, and a branch whose upstream has no merge base with it. Only a branch that cannot
     fast-forward and a branch blocked by uncommitted changes are skipped with a printed reason.
   - A branch whose upstream is an ancestor of it is up to date. A branch that can fast-forward: the current
     branch updates with `git merge --ff-only`. If the merge fails and its standard error holds both
     `local changes` and `would be overwritten`, the branch is skipped with the uncommitted-changes reason.
     If it fails for another reason, the branch is skipped with a plain failure reason. Any other branch
     updates with `git branch -f`, and a failure shows an error. In a bare repository there is no work
     tree to merge into, so the current branch updates with `git branch -f` like any other (probed against
     git 2.51: `git branch -f` on a bare repository's HEAD branch works).
   - A branch that cannot fast-forward is skipped, with the reason that a fast-forward is not possible.
6. `exec` runs its command without a shell, in the repository's working directory. A bare repository has no
   working directory, so it uses the repository directory itself. The command's standard output prints
   first, then its standard error, each split into lines the way `str::lines` splits. The command's exit
   status is ignored. A command that cannot start prints an error line. An `exec` with no command after
   `--` is a usage error (exit 2), and it runs before anything else, so it never touches the config file.
7. Every `git` child process runs through the `Sys` seam (see Git Backend), which pins the locale to English.
   The skip reasons in item 5 match English git output, so the pin is part of the design, not an accident.
8. Exit codes: 0 for a normal run, 2 for a usage error, and 1 when a config write fails or the config file
   cannot be parsed. Both also print an error to standard error. An unreadable or deleted current directory also exits 1, with
   the error on standard error, for every command except `list` and `clean`, which do not need it.

### Output

Output groups by path. Each path prints a header, then one block per repository in name order. The header
shows the path in its absolute form, normalised lexically (`.` and `..` parts resolved, no trailing slash),
with the repository count. A glob path keeps its wildcards in the header. A repository's name is its path
relative to the header path. A path that is itself a repository is named by its last component. A glob match
is named relative to the deepest common parent directory of the header path and the repository.
The wording is 9up's own. Keep it close to `gitup`'s where `gitup`'s words are already clear, and prefer one
short line per fact.

Terminal output uses colour. A colour is a fixed escape code per role: bold white for a path header, bold
green for a repository name, red for an error or a skip, cyan for new or updated references and branches, no
colour for up-to-date lines. Standard output that is not a terminal gets no colour codes. Standard error never
gets colour codes.

### Config File

9up keeps its state in one TOML file of its own; `gitup`'s bookmark file is not read. The document holds one
key, `repositories`: an array of path strings, in order. Any other keys are left alone, for settings that may
come later.

The default path is `$XDG_CONFIG_HOME/9up/config.toml`, or `~/.config/9up/config.toml` when the variable is
unset or empty, on every platform, macOS included (`main` reads the variable, not the `dirs` crate: its macOS
directory is `~/Library/Application Support`, which the owner chose not to use). A relative value is ignored,
as the XDG specification requires. A non-UTF-8 absolute `XDG_CONFIG_HOME` is accepted. `-C ""` also uses
the default path. With no home directory at all, the fallback is a relative `.config/9up/config.toml` path.

- Read: parse the file as TOML (the `toml_edit` crate). A missing or unreadable file means no repositories.
  A file that is not valid TOML, a file whose content is not valid UTF-8, or a `repositories` value that
  is not an array of strings, cannot be parsed: the run prints an error to standard error and exits 1. A
  missing `repositories` key means no repositories.
- Write: edit the parsed document in place and save it, so comments and any other keys stay as they are;
  only the `repositories` array changes. A write creates the parent directory. When the file does not
  exist, the new document is the one key, with each entry on its own line.
- A path is stored as its absolute form, normalised without a filesystem access: duplicate separators
  collapse, `.` parts drop, `..` parts resolve against the parts before them, and a trailing separator
  drops. A leading `~` (alone or as `~/`) keeps it, and the path is only normalised lexically. An empty
  path becomes the current directory.
- A relative `add` value resolves against the current directory first. An added path does not need to
  exist.
- Add reports each path as added or already present, and always writes. In `add x x`, the second `x` is
  already present.
- Delete matches a stored path exactly, after the same normalisation. Each argument removes only the
  first match. Delete reports each path as deleted or not present. Delete writes only when it removes an
  entry, so it does not write when there are no repositories or when no argument matches.
- List prints the entries as stored, or a note when there are none.
- `clean` removes an entry when it is not a directory and its glob has no match. A comment entry (see
  Behaviour item 2) is never removed. `clean` writes only when it removes an entry. When it removes
  nothing, it prints a note that says so.

### Differences From gitup

Feature parity allows these differences. The Feature Spec above is the whole contract: where it is silent,
the implementer chooses, and `gitup` is a guide, not a rule.

Where Python crashed or printed a traceback, 9up prints an error line instead: an unconfigured
`--current-only` remote, a failed `git branch -f`, a command that cannot start, and a failed config
write (which also exits 1).

| Difference | Reason |
| --- | --- |
| The interface is subcommands, not flags. One verb per run. | The idiomatic clap shape. |
| The state file is 9up's own TOML config; `gitup`'s bookmark file is not read. | A new tool keeps its own state, in its own format. |
| No migration from `~/.gitup` or the old XDG `config.ini`. | One default path, no legacy state. |
| Long options must be written in full. Argparse accepted any unique prefix. | Clap has no prefix matching. |
| An exec command is separate arguments after `--`, split by the shell. Python took one string and split it. | No re-splitting what the shell already split. |
| The current branch's upstream comes from git's own `%(upstream)` value. | One source instead of a reconstruction. |
| A `**` pattern in a configured path is recursive. | The `glob` crate's semantics. |
| New references after a fetch are classified by the local reference name. | Clearer than the remote-side name. |
| A symbolic-link cycle in the search ends that branch instead of looping. | `walkdir` detects the loop; `gitup` does not terminate on one. |
| In a bare repository, the HEAD branch updates like any other. | `gitup` skips it as "not possible to fast-forward" and leaves it stale (probed against 0.5.2); `git branch -f` on a bare HEAD branch works, so the update is safe. |
| A fetch does not recurse into submodules. | One less flag until a user needs it. |
| `$VAR` never expands, in any path. | The expansion was a side effect, not a feature. |
| Output prints per repository when the repository is done. No live fetch counters. | Milestone 2 concurrency needs this shape anyway. |
| Ctrl-C stops the process. There is no "Stopped by user." message. | The default signal behaviour is enough. |

## Design Decisions

### Git Backend

9up runs the `git` binary as a child process. It does not use `libgit2` (the `git2` crate) and does not parse
pack files. The binary honours the user's own git setup (aliases, credentials, signing, hooks, `includeIf`
rules) because real `git` runs, its behaviour matches the git the user runs by hand, and its failure text is
git's own words, which the user can search for.

`git` must be on the `PATH` at version 2.41 or later. Porcelain output (machine-readable output that does not
change between releases) only, and never the human progress output. Every call goes through the `Sys`
seam (see Module Breakdown): `RealSys::git`, with its working directory set to `dir`, `LC_ALL=C` and
`LANGUAGE=C` in the environment, and a null standard input. No shell, anywhere.

### Git Commands Per Repository

| Need | Command |
| --- | --- |
| Detect a repository (in `discover.rs`, no git call) | A filesystem test only. A git directory has a `HEAD` file and `objects` and `refs` directories. A linked worktree's admin directory has no `objects`: it holds a `commondir` file instead, and that also counts. A path is a repository when it is a git directory, when its `.git` entry is one, or when its `.git` file's `gitdir:` target is one. A relative `gitdir:` target resolves against the `.git` file's directory. |
| Working directory and git directory | `git rev-parse --absolute-git-dir` and `git rev-parse --show-toplevel`, run in the repository by `update.rs`. A bare repository has no top level, so its working directory is the git directory. Let git answer these questions. Do not resolve `commondir` files or `core.worktree` by hand. |
| Fetch | `git fetch --porcelain [--prune] -- <remote>`, one call per remote, with `--prune` added when `--prune` is set. Never `--quiet`: it silences the porcelain lines too (probed against git 2.51). The porcelain lines classify by the local reference name: a line with the `*` flag is a new reference, and a line that shows an old and a new value is an update. Lines with the flags `+`, `-`, `t`, `!` or `=` are not reported. A reported name comes from the local reference: strip `refs/remotes/<remote>/` from the front, else drop the first two path parts (which strips `refs/tags/` or `refs/heads/`). A reference with fewer than three parts keeps its full name. |
| Remotes, URLs and reference lines | One call per repository: `git config --get-regexp ^remote\.`, parsed in memory. It gives each remote's name, URL and `fetch` lines, in configuration order. A remote with no `fetch` lines prints the fetch skip line. |
| Current branch | `git symbolic-ref -q HEAD`, then strip the `refs/heads/` prefix. A failure means a detached HEAD. Never use `--short`, which prints `heads/main` when a tag `main` exists. |
| The current-only remote | `git config --get branch.<branch>.remote` and `git config --get branch.<branch>.merge`. Both must be set. Configuration works on an unborn branch, where `for-each-ref` lists nothing. `git config` matches the key as a pattern, so a branch name with pattern characters (a dot, say) can collide with a sibling key; accepted, as this is git's own lookup shape. |
| Branch snapshot | One call after the fetch: `git for-each-ref --format=%(refname)%00%(objectname)%00%(upstream) refs/heads refs/remotes`. The branch step reads upstreams and SHAs from this snapshot. After each successful `merge` or `branch -f`, set that branch's snapshot SHA to its upstream SHA, so a branch that tracks a local branch sees the move made earlier in the loop. |
| Update the current branch | `git merge --ff-only <upstream full ref>`. |
| Update any other branch | `git branch -f -- <branch> <upstream full ref>`. |
| Ancestor test | `git merge-base <branch SHA> <upstream SHA>`, from the snapshot. Exit 1 means no merge base. A base equal to the upstream SHA means up to date. A base equal to the branch SHA means a fast-forward is possible. |

Pass SHAs or full reference names to git, never short names. A tag with the same short name would win
otherwise.

### Dependencies

Five crates, plus one for tests. The standard library covers the rest. Every version is pinned exactly (`=` in `Cargo.toml`), so each
update arrives as a Renovate pull request instead of drifting inside a range. A bump that changes a behaviour
this plan states gets a plan update in the same pull request.

| Crate | Use |
| --- | --- |
| `clap` (derive feature) | Parse the command line: a `Parser` struct with a `Subcommand` enum. |
| `glob` | Expand the glob patterns that a configured path can contain, with `require_literal_leading_dot` set, as Python skipped dot entries. |
| `walkdir` | The depth-limited directory search in `discover.rs`: symbolic links followed, loop detection. Order comes from `repos.sort_by(name)` in `resolve()`, not from `WalkDir`, which is unsorted. |
| `dirs` | The home directory: the lookup behind `~` expansion and the `~/.config` default. |
| `toml_edit` | The config file: parse the TOML document and edit it in place, changing only the `repositories` array. |
| `tempfile` (development only) | Temporary directories in integration tests. |

What the standard library covers here: the colour constants (a few string constants, written by hand), the
error type (`io::Error` and `io::Result`), the lexical path normalisation (a short function over path
parts), and the child process (`std::process::Command`).

Not added: a colour crate, an error-type crate, a test-framework crate, a splitting
crate for the exec command (the shell already split it).

### Command Line

- Derive API. `Cli` is a `Parser` struct with the global config option and an optional subcommand.
  The commands are a `Subcommand` enum: `Update`, `Add`, `Delete`, `List`, `Clean`, `Exec`.
- The bare form is an argv shim in `cli.rs`: when the first positional argument is not a command name,
  `update` is inserted after the program name before clap parses. A flattened update struct cannot do
  this, because its path list swallows the command name as a path.
- The binary name comes from `#[command(name = "9up")]`, because the package name is `nine-up`. Help and
  version use clap's built-in flags, so `--version` prints `9up <version>` from the name alone.
- `update`'s `--depth` needs `allow_negative_numbers`, or clap reads `-1` as a flag. `exec` shares the
  depth argument.
- `add` and `delete` take `num_args(1..)` path lists. `exec` takes optional paths, then
  a required `--`: a `last = true` argument collects everything after it as the command's argv, untouched.
- Path and value fields are `String`, not `PathBuf`: an empty `-C ""` must parse, and it means the default
  path. `PathBuf` rejects empty values at parse time.

### Lints

- `Cargo.toml` sets `all` plus `unwrap_used`, `expect_used` and `dbg_macro` to `warn`, and `unsafe_code` to `forbid`.
  `-D warnings` on the command line turns each warning into an error, so the local check and CI match.
- Where a lint cannot hold, use `#[allow(...)]` with a comment that gives the reason.

### Output And Errors

- `main` returns `std::process::ExitCode`, so no `process::exit` call exists outside it.
- Render functions take a writer (`impl Write`). `main` passes a locked standard output and ignores
  `BrokenPipe`, so a closed pipe ends the run quietly with the success code. Unit tests pass a `Vec<u8>`.

## Module Breakdown

Eight modules in `src/`, one crate, no `lib.rs`. The architecture is a composition root (owner's
direction, 26 Oct 2026): `main` builds one `App` context in `sys.rs` — the `Sys` services, the working
directory, the home directory, the config file,
the colour flag and the output — and passes it down.
Nothing below `main` reads the environment, calls `dirs`, or builds a service. Path probes (metadata,
the `.git` layout, the `walkdir` search) work on path values and need no service.

| File | Contents |
| --- | --- |
| `main.rs` | The composition root and dispatch. Builds the `App` and hands each command to `commands`. |
| `cli.rs` | `Cli`, `UpdateArgs`, the command enum and the bare-path argv shim. |
| `commands.rs` | The add, delete, list, clean, update and exec flows, and the repository walk they share. Writes through the `App` output. |
| `render.rs` | `paint`, the path headers, and rendering of every report. |
| `sys.rs` | The `App` context, the `Sys` trait (git, exec, file read/write) and `RealSys`. Tests substitute a stub `Sys` to force failures. |
| `config.rs` | The config file's entries, plus `load`, `save`, `normalize`, `has`. No output. |
| `discover.rs` | `expand_home`, `glob_paths`, `resolve` and the depth-limited search. No `git` calls. |
| `update.rs` | `repo_info`, `update_repo`, `exec_repo`, all over the `Sys` seam. No output. |

A `Report` carries data only. Milestone 2 needs no type change: run `update_repo` on many threads, collect the
`Report` values, render in order.

Each module's unit tests sit in `src/<module>/tests.rs`, a child module declared with `#[cfg(test)] mod tests;`.
It is `pub(crate)` in `render` and `discover`, which share test helpers.

## Tests

Tests assert behaviour. Output assertions check the facts a line carries (the repository name, the outcome,
the reason), never the exact wording, so a wording change does not rewrite the suite.

Three layers:

- Unit tests in `src/<module>/tests.rs`, for the pure parts: path normalisation, the reference-line reading in
  `update.rs`, and report rendering in `render.rs`.
- Integration tests in `tests/cli.rs`, which run the compiled binary in temporary directories with real `git`
  repositories. The fixtures use local file-path remotes, so the suite needs no network.
- `tests/cli.rs` sets `HOME` to a temporary directory and passes `-C` with an explicit path, so no test reads
  the developer's real config file. Colour is off in tests (not a terminal). The fixtures run `git init -b main`, and the commit
  helper sets `GIT_CONFIG_NOSYSTEM=1` and `GIT_CONFIG_GLOBAL=/dev/null`, so a developer's git configuration
  (signing, hooks) cannot change a result.

Coverage is a gate. `mise run coverage` runs `cargo llvm-cov` with the `tests.rs` files filtered out by name and
fails under 100 percent line coverage of the production code. Replace the filter with `#[coverage(off)]` on the
test modules once that attribute is stable.

Behaviour tables, one row per case:

| Area | Cases |
| --- | --- |
| Commands | Bare `9up` updates the configured paths and nothing else. Bare `9up <path>` updates only that path. `update` with no paths updates the configured paths. `add x` adds x and updates nothing. `delete`, `list` and `clean` each do only their own job. `-C` works on every command. `-C ""` uses the default path. |
| Config | Add stores the absolute, normalised path. Add of an existing path reports it as already present. A relative add resolves against the current directory. Delete matches exactly, after normalisation. Delete removes only the first match. Delete of a missing path reports it as not present, and a delete that removes nothing leaves the file unchanged. List prints the paths in file order. Clean removes a missing path and keeps an existing one. Clean keeps a comment entry, and a clean that removes nothing leaves the file unchanged. A TOML comment and an unknown key survive add and delete, unchanged. A file that is not valid TOML, a file that is not valid UTF-8, and one whose `repositories` is not an array of strings, fail with an error and exit 1. A missing `repositories` key means no repositories. The default path is `$XDG_CONFIG_HOME/9up/config.toml` on Linux, `~/Library/Application Support/9up/config.toml` on macOS, and the relative `.config/9up/config.toml` fallback when no config directory is found. A `gitup` bookmark file at the old path is ignored. |
| Discovery | A repository at the path. Bare repositories: at the path, below the path, and at depth 0. A repository below the path, at each depth from 0 up. A `.git` file (a linked worktree) counts. A directory with no repositories, and an empty directory: header with a count of 0. A file given as a path. A symbolic link to a directory is followed. A broken symbolic link as a path: header with a count of 0. A comment line: never searched, and its text prints with the `#` characters stripped. A glob entry: matches expand; `**` recurses; no match means missing. A `*` pattern does not match a dot-directory. The search does not descend into a found repository. Repositories print in name order. Display names: a repository below the base is named by its path relative to the base. A path that is itself a repository is named by its last component. A glob match is named relative to the deepest common parent of the base and the repository. |
| Update | Up to date. Behind: fast-forward. Ahead. Diverged: skipped, fast-forward not possible. No upstream: skipped silently, prints nothing. Missing upstream reference: skipped silently, prints nothing. No merge base: skipped silently, prints nothing. Local changes block the current branch: skipped with the uncommitted-changes reason. `--current-only`: only the tracked remote fetches, and the branch step still visits every branch. An unborn branch with `--current-only`: the tracked remote fetches, and the branch step visits nothing. `--fetch-only`: no branch step. `--prune`: a branch deleted on the remote disappears locally. Two remotes: both fetch. First fetch fails: the second remote never fetches, and the branch step still runs. A remote with no reference lines: skip line. A new branch and a new tag arrive, named by the naming rule. A detached HEAD. A bare repository. |
| Exec | The command runs in the working directory. Standard output prints before standard error. The exit status is ignored. A missing command prints the error line. Arguments after `--` reach the command untouched, quotes included. A `--depth` on exec limits the search the way update's does. `exec` with no command is a usage error, and the config file is unchanged. With no configured paths and no paths given, `exec` prints nothing. |

The clippy configuration allows `unwrap` and `expect` in tests only. Test helper functions can use both,
with the reason in the `clippy.toml` comment.
