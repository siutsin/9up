# 9up Plan

Status: draft plan for a Rust port of `gitup` 0.5.2. Approval is the merge of this plan's pull request into `master`. No
implementation starts before that merge.

## Goal and Scope

| Item         | Decision                                                                                                   |
|--------------|------------------------------------------------------------------------------------------------------------|
| Problem      | Python `gitup` updates repositories one at a time. You run it often, so the wait adds up                   |
| Milestone 1  | Feature parity with Python `gitup` 0.5.2. Sequential. The same flags and output                            |
| Milestone 2  | Concurrency. Not planned in detail here, but the design must not block it                                  |
| Names        | Repository `9up`. Package `nine-up`, because Cargo rejects a name that starts with a digit. Binary `gitup` |
| Version      | The first version is `0.1.0`                                                                               |
| Out of scope | Releases, Homebrew formula, Windows, new features                                                          |

Source: the upstream tag `v0.5.2` of `https://github.com/earwig/git-repo-updater`, in `gitup/*.py`. It has `cli.py`,
`config.py`, `update.py` and `migrate.py`. The Parity Spec below is the source of truth. The Deliberate Deviations are the
only allowed differences from the Python source. The Python source decides everything else.

## Parity Spec

### Flags

| Flag                          | Meaning                                                                  | Milestone 1          |
|-------------------------------|--------------------------------------------------------------------------|----------------------|
| `path...`                     | Update this repo, or every repo below it                                 | Keep                 |
| `-u`, `--update`              | Also update all bookmarks                                                | Keep                 |
| `-t`, `--depth n`             | Search depth. Default `3`. `0` means no search. Negative means unlimited | Keep                 |
| `-c`, `--current-only`        | Fetch only the remote that the current branch tracks                     | Keep                 |
| `-f`, `--fetch-only`          | Fetch only. Do not fast-forward branches                                 | Keep                 |
| `-p`, `--prune`               | Pass `--prune` to each fetch                                             | Keep                 |
| `-a`, `--add path...`         | Add bookmarks                                                            | Keep                 |
| `-d`, `--delete path...`      | Delete bookmarks                                                         | Keep                 |
| `-l`, `--list`                | List bookmarks                                                           | Keep                 |
| `-n`, `--clean`, `--cleanup`  | Delete bookmarks that point to nothing                                   | Keep                 |
| `-b`, `--bookmark-file path`  | Use another bookmark file                                                | Keep, value required |
| `-e`, `--exec`, `--batch cmd` | Run a command in each repo instead of updating                           | Keep                 |
| `-h`, `--help`                | Print help                                                               | Keep                 |
| `-v`, `--version`             | Print version                                                            | Keep                 |
| `--selftest`                  | Run the Python test suite                                                | Drop                 |

Long options accept any unique prefix, as argparse does. So `--fetch` means `--fetch-only`.

### Behaviour

1. `-h`, `-v` and usage errors exit before any other output. Every other run first prints `gitup: the git-repo-updater` and
   a blank line.
2. Bookmark actions run in this order: add, delete, list, clean. Each one sees the result of the one before.
3. `-e ""` is the same as no `-e`. Any other `-e` value is split into words as Python `shlex.split` does in POSIX mode with
   comments off. A value with no words, or one that cannot be split, is a usage error with exit `2`. The split rules:
   1. Only space, tab, CR and LF separate words. So a value of only these four characters has no words, but `"\f"` is a word.
   2. `#` is a literal character.
   3. Outside quotes, `\` escapes the next character. A trailing `\` is an error.
   4. Inside `'`, every character is literal. Inside `"`, `\` escapes only `"` and `\`. Any other `\` stays, so `"%h\t%s"`
      keeps `\t`. An unclosed quote is an error.
   5. Adjacent quoted and unquoted parts join into one word. So `a"b c"d` is `ab cd`, and `''` is one empty word.

   The split command replaces the update. It runs in each given path. With `-u`, or with no path, it also runs in each
   bookmark. No bookmarks prints nothing.
4. Without `-e`, update each given path. Then update the bookmarks if `-u` is set or no other action ran.
5. An update of the bookmarks with no bookmarks prints the no bookmarks hint.
6. Each path, whether a bookmark or an argument, has `~` expanded. Only a leading `~` alone or `~/` expands, to `HOME`. `~user` and
   `$VAR` stay as typed. An empty path, as in `gitup ""`, prints `doesn't exist!` before any other check. Without this rule
   `Path::new("").join(".git")` is `.git` in the current directory. Every other path resolves in this order:
   1. A repo: use it.
   2. A missing path whose first non-space character is `#`: a comment. Strip leading space, then every leading `#`, then trim.
      Print the rest in cyan. Print nothing if it is empty.
   3. Another missing path: expand it as a glob. No match, or a bad pattern, prints `doesn't exist!`. A path with no glob
      magic matches when its own link exists, as Python `lexists` did. So a broken symlink prints a header with `0 repos`.
   4. An existing path that is not a directory prints `isn't a repository!`. So does a directory when depth is `0`.
   5. A directory: collect the repos below it.
7. The search stops at the first repo on each branch of the tree. It follows symlinked directories and enters hidden ones.
   It skips a directory that it cannot read. Python crashed there with `PermissionError`.
   Depth `n` finds repos up to `n` levels below the path. A glob match counts as level 0, so `-t 0` still checks each match.
8. Each resolved path prints `<base> (N repos):`, or `(1 repo):`. `N` can be `0`. `<base>` is the absolute form of the path,
   with `.`, `..` and trailing slashes removed lexically, as Python `os.path.abspath` does. A glob base keeps its wildcards.
9. Repos follow in name order. A name is the repo path relative to `<base>`. A path that is itself a repo is named by its last
   component. A glob match is named relative to the deepest common parent directory of `<base>` and the repo.
10. Each repo prints its name, then these steps:
    1. Pick the remotes: all of them, or with `-c` the remote of the current branch upstream.
    2. A repo error ends the repo. It prints the error line, with no fetch and no branch step. The checks run in this order:
       with `-c`, a detached HEAD, then with `-c`, no tracked remote, then no remotes.
    3. Fetch each remote in config order. A remote with no `fetch` refspec prints the fetch skip line. On the first failed
       fetch, print the error and stop fetching. The branch step still runs.
    4. Report new branches, new tags and branch updates by name. Report `up to date` when there are none.
    5. Unless `-f`, visit every local branch in name order. This includes `-c` runs.
    6. Skip a branch with no upstream, a missing upstream or no merge base. Report `up to date` when the upstream is an ancestor.
    7. Fast-forward the current branch with `git merge --ff-only`. Skip it on local changes or when that fails.
    8. Move any other branch with `git branch -f`, but only when the branch is an ancestor of its upstream.
11. The fetch error text is built from stderr in this order, as GitPython and Python `gitup` did:
    1. Keep only lines that start with `error:` or `fatal:`. Join them with a newline.
    2. Collapse each whitespace run to one space. Trim.
    3. Drop one leading `fatal:` and the spaces after it.
    4. Strip every leading and trailing `'`. So `fatal: '/x' does not appear` becomes `/x' does not appear`.
    5. If the text is empty, use the fallback from Message Strings. Otherwise add `.` unless it ends with `.`.
12. `-e` runs without a shell, in the repo workdir. Remove one final `\n` from each stream, as GitPython did. Then split
    each stream like Python `str.splitlines`, which also breaks on `\r`, `\v`, `\f`, `\x1c` to `\x1e`, `\x85`, `\u{2028}`
    and `\u{2029}`. A `\r\n` pair is one line break. Rust `str::lines` does not do this, so a small splitter does it.
    Print stdout lines, then stderr lines. An empty line prints as 8 spaces. Ignore the exit status.
13. Every child process, git and `-e` alike, gets `LC_ALL=C`, `LANGUAGE=C` and a null stdin, as GitPython did.
14. Exit status is `0` after the banner and `2` on a usage error. A failed bookmark write prints `Error: <io message>` to stderr
    and exits `1`.

### Message Strings

Tests assert these texts byte for byte, with colour off. Python `print` adds a space between arguments. So repo names and
bookmark entries have a 4 space indent. Fetch, branch and repo error lines, and `-e` output, have an 8 space indent. Banner,
headers, comments and path errors have none.

| Case             | Exact text                                                                                                                                                |
|------------------|-----------------------------------------------------------------------------------------------------------------------------------------------------------|
| Banner           | `gitup: the git-repo-updater`, then an empty line                                                                                                         |
| Header           | `<base> (N repos):`, `<base> (1 repo):`                                                                                                                   |
| Repo line        | `<name>:`                                                                                                                                                 |
| Fetch result     | `Fetching <remote>: <kinds>.` Kinds are joined by a comma and a space. A kind is `new branch (a)` or `new branches (a, b)`                                |
| Fetch kinds      | `new branch`, `new branches`, `new tag`, `new tags`, `branch update`, `branch updates`, in that order                                                     |
| Fetch none       | `Fetching <remote>: up to date.`                                                                                                                          |
| Fetch skip       | `Fetching <remote>: skipped: no configured refspec.`                                                                                                      |
| Fetch error      | `Fetching <remote>: error: <text>` with a lowercase `error:`                                                                                              |
| Fetch fallback   | `<command> failed with status <N>.`                                                                                                                       |
| Branch result    | `Updating <branch>: up to date.`, `Updating <branch>: done.`                                                                                              |
| Branch skip      | `Updating <branch>: skipped: <reason>.`                                                                                                                   |
| Skip reasons     | `no upstream is tracked`, `upstream does not exist`, `can't find merge base with upstream`, `uncommitted changes`, `not possible to fast-forward`         |
| Branch error     | `Updating <branch>: error: <text>` with a lowercase `error:`. `<text>` is the `git branch -f` stderr, cleaned as in Behaviour 11                          |
| Uncommitted rule | The merge stderr holds both `local changes` and `would be overwritten`                                                                                    |
| Repo errors      | `Error: no remotes configured to fetch.`, `Error: no remote tracked by current branch.`, `Error: --current-only doesn't make sense with a detached HEAD.` |
| Path errors      | `Error: <path> doesn't exist!`, `Error: <path> isn't a repository!`. `<path>` is the `~` expanded input, not the absolute form                            |
| Exec start error | `Error: <argv0>: <io error>`                                                                                                                              |
| No bookmarks     | `You don't have any bookmarks configured! Get help with 'gitup -h'.`                                                                                      |
| Bookmark titles  | `Added bookmarks:`, `Already bookmarked:`, `Deleted bookmarks:`, `Not bookmarked:`, `Current bookmarks:`                                                  |
| Bookmark notes   | `You have no bookmarks to display.`, `You have no bookmarks to clean up.`, `All of your bookmarks are valid.`                                             |

Colour follows Python. colorama 0.4.6 strips ANSI codes when stdout is not a terminal, so the port does the same.

| Style  | Used for                                                                                          |
|--------|---------------------------------------------------------------------------------------------------|
| Bold   | `gitup` in the banner, header base, repo name, remote name, branch name, the path in a path error |
| Red    | `Error:`, fetch `error:`, `Already bookmarked:`, `Not bookmarked:`                                |
| Yellow | `skipped:`, `Added bookmarks:`, `Deleted bookmarks:`, `Current bookmarks:`                        |
| Green  | `done`, each fetch kind                                                                           |
| Blue   | `up to date`                                                                                      |
| Cyan   | Comment text                                                                                      |

Every colour is bold too, because Python adds `Style.BRIGHT` to each one.

### Bookmark File

| Item      | Rule                                                                                                                                                                                                |
|-----------|-----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| Path      | `$XDG_CONFIG_HOME/gitup/bookmarks`, else `~/.config/gitup/bookmarks`. An empty variable counts as unset. The variable is `~` expanded                                                               |
| Override  | `-b path`, with `~` expanded. `-b ""` uses the default path, as Python did                                                                                                                          |
| Read      | Split on `\n`. Trim each line. Drop empty lines. A missing or unreadable file means no bookmarks                                                                                                    |
| Write     | Create the parent directory. Join with `\n`. No final newline                                                                                                                                       |
| Encoding  | UTF-8                                                                                                                                                                                               |
| Add       | Normalise each path. Report new ones as added and known ones as already bookmarked. Always write. In `-a x x` the second is known                                                                   |
| Delete    | Normalise each path. Each argument removes only the first match. Report deleted and not bookmarked                                                                                                  |
| Empty     | `-d` with an empty file reports all as not bookmarked and does not write                                                                                                                            |
| List      | Print entries as stored, or the no bookmarks to display note                                                                                                                                        |
| Clean     | Drop an entry that is not a directory and whose `~` expanded glob has no match. This drops comment lines too. Write only on change                                                                  |
| Normalise | A path that starts with `~`, `~user` included, keeps it and is only normalised lexically. An empty path becomes the absolute current directory, as Python `abspath("")` did. Others become absolute |
| Comments  | A line that starts with `#` prints as a comment when an update or `-e` run reaches it                                                                                                               |

### Deliberate Deviations

Each row drops work with no value now or fixes a Python crash. Ask for any row back and it becomes a step.

| Deviation                                                                                         | Reason                                                                                                                                                            |
|---------------------------------------------------------------------------------------------------|-------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| No `--selftest`                                                                                   | It ran the Python suite. The port uses `cargo test`                                                                                                               |
| No migration from `~/.gitup` or `config.ini`                                                      | Only pre 2018 installs need it. A Python 0.5.2 install has already migrated                                                                                       |
| No live fetch counters                                                                            | Python printed them even in a pipe. They break once repos run in parallel                                                                                         |
| Each repo block prints when the repo is done                                                      | Steps return data first. Python printed `Fetching x` before the fetch ran. The final text is the same                                                             |
| No `Stopped by user.` on Ctrl-C                                                                   | It needs a signal crate. The default exit on SIGINT is enough                                                                                                     |
| Help and usage error wording come from clap                                                       | The flags, groups and help texts stay. The layout is clap's                                                                                                       |
| `-v` prints `gitup <version>`                                                                     | Python also printed the Python version                                                                                                                            |
| `-b` needs a value                                                                                | A bare `-b` did nothing in Python                                                                                                                                 |
| The upstream comes from git `%(upstream)`                                                         | Python rebuilt it from config and ignored the refspec. Git's answer is the correct one                                                                            |
| `-c` with an upstream remote that is not configured prints `no remote tracked by current branch.` | Python crashed with `IndexError`                                                                                                                                  |
| `**` in a glob is recursive                                                                       | The `glob` crate rule. Python treated `**` like `*`                                                                                                               |
| A failed `git branch -f` prints `Updating <branch>: error: <text>`                                | Python crashed, for example on a branch checked out in another worktree                                                                                           |
| New refs are classified by their local ref name                                                   | See the porcelain table. Python parsed `[new branch]` text from stderr                                                                                            |
| A ref outside `refs/remotes/<remote>/` keeps its name, so a mirror prints `new branch (feat)`     | Python rebuilt the ref as `refs/remotes/feat` and printed `new branch ()`. Checked on a `--mirror` clone                                                          |
| Fetch does not recurse into submodules                                                            | `--porcelain` turns off config driven submodule recursion. Run `git submodule update` yourself                                                                    |
| The fetch fallback names the Rust command                                                         | It shows the fetch argv from Git Commands Per Repo, not the GitPython argv                                                                                        |
| `-e` whitespace only or unsplittable is a usage error. `-e ""` stays no `-e`                      | Python crashed at the first repo. Treating it as no `-e` would start a full update by mistake. The port exits before the bookmark actions, which Python ran first |
| `-e` start failure prints `Error: <argv0>: <io error>`                                            | Python printed the long GitPython text `Cmd('x') not found due to: ...`                                                                                           |
| Invalid UTF-8 in the bookmark file is replaced                                                    | Python crashed with `UnicodeDecodeError`                                                                                                                          |
| The `-b` help names the default rule, not the resolved path                                       | Clap help text is static. It says `$XDG_CONFIG_HOME/gitup/bookmarks`, else `~/.config/gitup/bookmarks`                                                            |
| The help epilog links to this repository                                                          | Bugs in the port belong here, not upstream                                                                                                                        |
| `[path]...` lists last in the `updating repositories` group                                       | Clap puts positionals after options in a heading. Python listed it first                                                                                          |
| An unreadable directory in the walk is skipped                                                    | Python crashed with `PermissionError` from `os.listdir`                                                                                                           |
| No `$VAR` expansion in paths                                                                      | GitPython expanded `$VAR` when it opened a path, with a deprecation warning. Globs and errors did not                                                             |
| `~user` is not expanded, only `~` and `~/`                                                        | Python `expanduser` also read other users' homes. Behaviour 6 has the rule                                                                                        |
| An empty path argument prints `Error:  doesn't exist!`                                            | GitPython opened the current directory for `""`. If that was not a repo, Python printed `isn't a repository!`                                                     |
| A relative `core.worktree` resolves against the git dir                                           | Git's rule. GitPython ran git in the raw value, so it resolved against the current directory                                                                      |
| A failed bookmark write prints `Error: <io message>` to stderr and exits `1`                      | Python printed a traceback                                                                                                                                        |

## Design Decisions

### Git Backend

The port runs the `git` binary through `std::process::Command`. It does not link `libgit2` or `gix`.

| Reason                                                                           |
|----------------------------------------------------------------------------------|
| Auth works as in your shell: ssh agent, credential helpers, `includeIf`, proxies |
| No C build dependency and no gap in fetch features                               |
| GitPython also shells out for fetch, merge and branch, so behaviour matches      |
| Each call is its own process, so parallel repos in milestone 2 need no locks     |

One function, `run(dir, argv)`, starts every child process. It sets `LC_ALL=C`, `LANGUAGE=C` and a null stdin, and captures
both streams. The locale rule is required, because skip reasons match English git text such as `would be overwritten`.

The port needs git 2.41 or later for `git fetch --porcelain`. A spike on git 2.56 confirmed the format
`<flag> <old> <new> <ref>`. A fetch with no change prints nothing. A failed fetch exits `128`.

| Porcelain line                                       | Reported as                |
|------------------------------------------------------|----------------------------|
| Flag `*`, ref under `refs/heads/` or `refs/remotes/` | `new branch`               |
| Flag `*`, ref under `refs/tags/`                     | `new tag`                  |
| Flag ` `, a space                                    | `branch update`            |
| Flags `+`, `-`, `t`, `!`, `=`, any other `*`         | Not reported, as in Python |

A reported ref is named as GitPython names it. Strip `refs/remotes/<any segment>/`, else drop the first two path parts, which
strips `refs/tags/` or `refs/heads/`. A ref with fewer than three parts keeps its full name. So `refs/remotes/origin/feat/x`
is `feat/x`, and a mirror's `refs/heads/feat` is `feat`.

### Git Commands Per Repo

| Step           | Command                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
|----------------|----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| Detect a repo  | No git call. A std check in `discover.rs`, as GitPython did. A git dir has a `HEAD` file and `objects` and `refs` directories. A path is a repo when it is a git dir, when `path/.git` is one, or when `path/.git` is a file whose `gitdir:` target is one. A relative target resolves against the `.git` file's directory. A linked worktree target holds a `commondir` file, which also counts                                                                                                                                                                                                                                                                                                                                                               |
| Workdir        | `Repo` keeps the git dir and a separate workdir. For `path/.git` and a `.git` file, the workdir is `path`. A git dir that holds `gitdir` and `commondir` files is a linked worktree admin dir, as in GitPython. Its workdir is the parent of the path in `gitdir`, resolved against the git dir when relative. It is never bare. For any other path that is itself a git dir, `git config --local --get --bool core.bare` and `git config --local --get core.worktree` run in the git dir. `--local` reads only the repo config, as GitPython did, so global config never sets `core.worktree`. A bare repo uses the git dir. Else `core.worktree`, resolved against the git dir, or the parent of the git dir. Every git and `-e` command runs in the workdir |
| Current branch | `git symbolic-ref -q HEAD`, then strip `refs/heads/`. Exit `1` means a detached HEAD. Do not use `--short`, which prints `heads/main` when a tag `main` exists                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| Remotes        | `git config --local --includes --name-only --get-regexp '^remote\..+\.[^.]+$'`. Exit `1` means no remotes. The name sits between the first and last dot. Keep config order and drop repeats. A remote without a `.fetch` key prints the fetch skip line                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| `-c` remote    | `git config --get branch.<b>.remote` and `git config --get branch.<b>.merge`, before the fetch. Both must be set, as GitPython `tracking_branch` required. Config works on an unborn branch, where `for-each-ref` lists nothing                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| Refs           | `git for-each-ref --format='%(refname)%00%(objectname)%00%(upstream)' refs/heads refs/remotes`, run once after the fetch. It serves only the branch step. A missing upstream is a failed lookup in its rows                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| Fetch          | `git fetch --porcelain <remote>`, or `git fetch --porcelain --prune <remote>` when `-p` is set                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| Merge base     | `git merge-base <branch sha> <upstream sha>`. Exit `1` means no merge base. Base equal to upstream means up to date. Base equal to branch means a fast-forward is possible                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| Update current | `git merge --ff-only <upstream full ref>`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| Update another | `git branch -f <branch> <upstream full ref>`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |

The quotes in this table are for display. `run` passes each argv element as is, with no shell and no shell quotes.

Pass SHAs or full ref names to git, never short names. A tag with the same short name would win otherwise.

After each successful `merge` or `branch -f`, set that branch's SHA in the snapshot to its upstream SHA. Python read each
ref fresh, so a branch that tracks a local branch must see the move made earlier in the loop.

### Dependencies

Ponytail rule: std first. Each crate must earn its place.

| Crate           | Use                      | Why not std                                                                        |
|-----------------|--------------------------|------------------------------------------------------------------------------------|
| `clap` derive   | Parsing, help, version   | Groups, aliases, prefix matching and `nargs` parity                                |
| `glob`          | Path and bookmark globs  | Python used `glob`. Set `require_literal_leading_dot`, as Python skips dot entries |
| `tempfile`, dev | Temporary repos in tests | Safe cleanup of temporary directories                                              |

`glob` 0.3.4 checks a literal pattern part with `fs::symlink_metadata`, so a broken symlink matches as Python `lexists` did.
This was checked in a scratch crate. No extra `lexists` check is needed in `discover::resolve` or `bookmarks::clean`.

| Need            | Std answer                                                                                                                         |
|-----------------|------------------------------------------------------------------------------------------------------------------------------------|
| Split `-e`      | About 40 lines in `main.rs`: POSIX `shlex.split` with comments off, with the rules in Behaviour 3                                  |
| Colour          | Six ANSI `const` strings and a reset, used only when `std::io::IsTerminal` is true                                                 |
| Errors          | `std::io::Result` and `?` for the bookmark file, stdout and `run`. Every git failure is report text. No `Box<dyn Error>`           |
| Home and config | `std::env::var_os` for `HOME` and `XDG_CONFIG_HOME`, read only in `main.rs`. See Data Flow                                         |
| Path normalise  | A lexical `..` collapse. A path that starts with `~` gets only this. Others first get `std::path::absolute`, with `""` read as `.` |
| Depth walk      | `std::fs::read_dir` recursion                                                                                                      |
| Child processes | `std::process::Command` with `current_dir`, `env` and `Stdio::null()`                                                              |

Not added: `tokio`, because git is the slow part and runs as a process. Also not `rayon`, `anyhow`, `thiserror`, `serde`,
`walkdir`, `dirs`, `anstream` or `assert_cmd`. Not the `shlex` crate either. Version 2.0.1 treats `#` as a comment with no
option to turn it off: a scratch test gave `["echo"]` for `echo # x`, where Python gives `["echo", "#", "x"]`.

### Concurrency Readiness

Milestone 1 is sequential. These rules keep milestone 2 a small change:

1. `update_repo(&Repo, &Cli) -> Report` returns data and never prints. So does `exec_repo`.
2. Only `main.rs` writes to stdout. It prints each `Report` in sorted repo order.
3. No global mutable state. `Cli` and `Repo` are plain data, so both are `Sync`.
4. Every child process goes through `run(dir, argv)`. A timeout or a job limit attaches there later.
5. Dispatch collects a `Vec<Repo>` per path and then maps it to reports. Milestone 2 swaps the map for
   `std::thread::scope` with a worker limit.

## CLI Design

One command and no subcommands, so every Python invocation keeps working. One clap derive struct, `Cli`, has one field per
flag. `help_heading` reproduces the four Python groups with their lowercase titles. Each help text is the Python text.

### Help Screen

This is clap 4.6.7 output from a scratch crate with the real texts. Do not add the `wrap_help` feature. Without it clap does
not wrap, and it aligns each heading on its own. The `-t` and `-b` lines are wrapped here only to fit the line limit.

```text
Easily update multiple git repositories at once.

Usage: gitup [OPTIONS] [path]...

updating repositories:
  -u, --update        update all bookmarks (default behavior when called without arguments)
  -t, --depth <n>     max recursion depth when searching for repos in subdirectories (default: 3; use 0 for no recursion,
                      or -1 for unlimited)
  -c, --current-only  only fetch the remote tracked by the current branch instead of all remotes
  -f, --fetch-only    only fetch remotes, don't try to fast-forward any branches
  -p, --prune         after fetching, delete remote-tracking branches that no longer exist on their remote
  [path]...           update this repository, or all repositories it contains (if not a repo directly)

bookmarking:
  -a, --add <path>...         add directory(s) as bookmarks
  -d, --delete <path>...      delete bookmark(s) (leaves actual directories alone)
  -l, --list                  list current bookmarks
  -n, --clean                 delete any bookmarks that don't exist [alias: --cleanup]
  -b, --bookmark-file <path>  use a specific bookmark config file (default: $XDG_CONFIG_HOME/gitup/bookmarks,
                              else ~/.config/gitup/bookmarks)

advanced:
  -e, --exec <command>  run a shell command on all repos [alias: --batch]

miscellaneous:
  -h, --help     show this help message and exit
  -v, --version  show program's version number and exit

Both relative and absolute paths are accepted by all arguments.
Direct bug reports and feature requests to https://github.com/siutsin/9up.
```

### Sample Output

The bookmarks are `# work`, `~/projects` and `~/gone`. Colour is stripped here, as in a pipe.

```text
gitup: the git-repo-updater

work
/home/me/projects (3 repos):
    9up:
        Fetching origin: new branch (feat), branch update (main).
        Updating feat: skipped: no upstream is tracked.
        Updating main: done.
    notes:
        Error: no remotes configured to fetch.
    tools/cli:
        Fetching origin: up to date.
        Fetching fork: error: Could not read from remote repository.
        Updating main: skipped: uncommitted changes.
Error: /home/me/gone doesn't exist!
```

### Clap Notes

1. `#[command(name = "gitup", version, about, after_help = EPILOG, disable_version_flag = true, disable_help_flag = true,
   infer_long_args = true)]`. Without `name`, clap prints `nine-up` from `CARGO_PKG_NAME`. `infer_long_args` gives the
   argparse prefix match. `about` reads `description = "Easily update multiple git repositories at once."` from
   `Cargo.toml`. `EPILOG` is a `const` with the two epilog lines, joined by `\n`.
2. `-h` is `_help: Option<bool>` with `ArgAction::Help` and `help_heading = "miscellaneous"`. The built in flag would sit
   under a default heading instead.
3. `-v` is `_version: Option<bool>` with `ArgAction::Version`. It prints `gitup <version>`. Clap panics on this action
   without `version` on the command.
4. `Option<bool>` keeps both out of `struct_excessive_bools`. The `_` prefix keeps `dead_code` quiet, as clap reads them.
5. `-a` and `-d` are `Vec<String>` with `num_args = 1..`, as `nargs="+"` was. `bookmark_file` is `Option<String>` and
   `paths` is `Vec<String>`. The clap `PathBuf` parser rejects an empty value, and `-b ""` and `gitup ""` must parse.
6. `-t` is `i32` with `default_value_t = 3` and `allow_negative_numbers = true`, so `-t -1` parses. It also sets
   `hide_default_value = true`, because the help text already says `default: 3`.
7. `-e` is `Option<String>` with one value. `--batch` is a visible alias, and so is `--cleanup` for `-n`.
8. Before the banner, a rejected `-e` value from Behaviour 3 runs
   `let mut cmd = Cli::command(); cmd.error(ErrorKind::ValueValidation, "...").exit()`, with `clap::CommandFactory` in
   scope. It prints a usage error and exits `2`.
9. `help_heading` on the `path` positional puts `[path]...` in `updating repositories`. A scratch crate confirmed it.

## Step 1: Foundation and Build Infra

Goal: an empty but fully gated crate. The four checks in the `README.md` Development section pass locally and in CI,
before any feature code exists. Repo conventions come from the repos listed under Files. None of them is Rust, so the Rust parts
follow the Cargo book and the clippy book.

Run `rustup toolchain install` in the repo. It installs the toolchain named in `rust-toolchain.toml`.

### Verified Versions

Checked on 25 Oct 2026. Renovate keeps every pin current after Step 1.

| Item        | Pin      | Note                                                              |
|-------------|----------|-------------------------------------------------------------------|
| Rust stable | `1.99.1` | Released 2026-10-13                                               |
| Edition     | `2024`   | The latest edition                                                |
| `clap`      | `4.6.7`  | Feature `derive` only                                             |
| `glob`      | `0.3.4`  |                                                                   |
| `tempfile`  | `3.27.0` | Dev dependency                                                    |
| git         | `2.41`   | Minimum for `fetch --porcelain`. Both CI runners ship a newer one |

### Files

Sources are `https://github.com/siutsin/heartbeats`, `https://github.com/siutsin/images` and
`https://github.com/siutsin/telegram-jung2-bot`, read on 25 Oct 2026. An empty Source cell means a new file.

| File                            | Content                                                                                                                                                              | Source                                                                                                                                                     |
|---------------------------------|----------------------------------------------------------------------------------------------------------------------------------------------------------------------|------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `Cargo.toml`                    | Package `nine-up`, `edition = "2024"`, `description`, `[[bin]] name = "gitup"` with `path = "src/main.rs"`, the lint policy below                                    |                                                                                                                                                            |
| `Cargo.lock`                    | Committed, because this is a binary                                                                                                                                  |                                                                                                                                                            |
| `rust-toolchain.toml`           | Exists already. `channel = "1.99.1"`, `profile = "minimal"`, `components = ["clippy", "rustfmt"]`                                                                    |                                                                                                                                                            |
| `clippy.toml`                   | `allow-unwrap-in-tests = true`, `allow-expect-in-tests = true`                                                                                                       |                                                                                                                                                            |
| `src/main.rs`                   | `Cli` with `_help` and `_version` only, and a `main` that parses it                                                                                                  |                                                                                                                                                            |
| `tests/cli.rs`                  | One test that runs `env!("CARGO_BIN_EXE_gitup")` with `-v` and checks the output                                                                                     |                                                                                                                                                            |
| `mise.toml`                     | Exists already. It pins tools only, with no tasks. `zizmor` is pinned for an optional local `zizmor .github/workflows` run. It is not one of the four checks         |                                                                                                                                                            |
| `.editorconfig`                 | See Copied Text                                                                                                                                                      | `telegram-jung2-bot/.editorconfig`, `images/.editorconfig`                                                                                                 |
| `.gitignore`                    | `/target`, `.scratchpad/`, `.idea/`, `.DS_Store`                                                                                                                     |                                                                                                                                                            |
| `.markdownlint-cli2.jsonc`      | Exists already: line length 140, tables exempt, `**/.scratchpad/**` and `target/**` ignored                                                                          | `heartbeats/.markdownlint-cli2.jsonc`, with line length 140, not 180                                                                                       |
| `renovate.json5`                | Exists already. Its content is final                                                                                                                                 | `heartbeats/renovate.json5`. Go, Docker and Helm rules dropped. The `ci` commit type rule waits for release-please                                         |
| `.github/workflows/lint.yaml`   | See CI Workflows                                                                                                                                                     | `heartbeats/.github/workflows/lint.yaml`: the `on` block, one job per check, the checkout step. `mise-action` replaces `setup-node`. Go and Docker dropped |
| `.github/workflows/test.yaml`   | See CI Workflows                                                                                                                                                     | `heartbeats/.github/workflows/test.yaml`: the `on` block and permissions. Go setup swapped for `rustup` and `rust-cache`. Matrix adds macOS                |
| `.github/workflows/zizmor.yaml` | See CI Workflows                                                                                                                                                     | `heartbeats/.github/workflows/zizmor.yaml`, as is                                                                                                          |
| `AGENTS.md`                     | Rules, a pointer to `README.md` and one verify line with the four checks                                                                                             | Exists already. `Rules` and `Verify` from `heartbeats/AGENTS.md`. Go rules swapped for Rust ones                                                           |
| `CLAUDE.md`                     | `# Claude Instructions`, a blank line, then `@AGENTS.md`                                                                                                             | `images/CLAUDE.md`                                                                                                                                         |
| `README.md`                     | Exists already. The Install section, marked as planned, the Development checks and a one line Credit section that links `https://github.com/earwig/git-repo-updater` |                                                                                                                                                            |
| `LICENSE`                       | Exists already. MIT, with Ben Kurtovic's upstream copyright line beside the port's line, because logic and strings are ported                                        |                                                                                                                                                            |

Not added: `rustfmt.toml`, `deny.toml`, `.cargo/config.toml`, `build.rs`, and a workspace.
Add any other file only on a real need.

### Cargo.toml Policy

```toml
[lints.rust]
unsafe_code = "forbid"

[lints.clippy]
all = { level = "warn", priority = -1 }
pedantic = { level = "warn", priority = -1 }
unwrap_used = "warn"
expect_used = "warn"
dbg_macro = "warn"
print_stdout = "warn"
print_stderr = "warn"
allow_attributes = "warn"
allow_attributes_without_reason = "warn"
```

1. Lints are `warn` in the file and fail the gate through `-D warnings`. A local `cargo build` stays usable while you edit.
2. Use `#[expect(lint, reason = "...")]`, never `#[allow]`. The last two lints enforce it.
3. Nothing uses `print!`, `println!` or `eprintln!`. `main.rs` writes with `writeln!` to a locked stdout. Rendering
   functions return `io::Result<()>` and use `?` on each write. A closed pipe then ends the run quietly, where `println!`
   would panic.
4. `main` returns `std::process::ExitCode`, not `Result`, because a `Result` from `main` prints its own `Error: <Debug>`
   text. A stdout write error gives `ExitCode::SUCCESS`, as in item 3. A failed bookmark write gives `ExitCode::FAILURE`
   after its stderr line. Only clap's `exit()` ends the process early.
5. `pedantic` includes `struct_excessive_bools`, which fires above 3 `bool` fields. `Cli` gets
   `#[expect(clippy::struct_excessive_bools, reason = "one field per Python flag")]` in the step that adds the fourth bool.
   An earlier `expect` is unfulfilled, and that fails the gate too.
6. Dead code and never read fields fail `-D warnings`. So each step adds only what it reads. See Build Order.
7. No `rust-version`. The toolchain file pins the compiler.

### Local Checks

Cargo is the Rust command runner, so a `Makefile` is not idiomatic here. `mise.toml` pins tools only and has no tasks.
All Rust commands are plain cargo. The four checks are listed once, in the `README.md` Development section. They run as
plain commands, locally and in CI. `markdownlint-cli2` comes from the mise tool pin.

Other work uses cargo directly: `cargo fmt`, `cargo build --locked --release` and `cargo install --locked --path .`.

### CI Workflows

Every action pins a full commit SHA with a version comment. Every checkout sets `persist-credentials: false`. Every job
grants `contents: read`, and the zizmor job adds `security-events: write`. Lint and test run on push and pull request to
`master`. Zizmor keeps its own workflow path filter.

| Workflow      | Jobs                                                                                                                              |
|---------------|-----------------------------------------------------------------------------------------------------------------------------------|
| `lint.yaml`   | `rust`: `cargo fmt --check`, then `cargo clippy --locked --all-targets -- -D warnings`. `markdown`: `markdownlint-cli2 '**/*.md'` |
| `test.yaml`   | `test`: `cargo test --locked` on a matrix of `ubuntu-latest` and `macos-latest`                                                   |
| `zizmor.yaml` | `zizmor` with the SARIF upload. See Copied Text                                                                                   |

Each Rust job runs `rustup toolchain install` with no argument, which reads `rust-toolchain.toml`. Then it runs
`Swatinem/rust-cache`, pinned by SHA. Look up each SHA from the action's release tag when you write the file. The Rust jobs
call cargo directly and do not install mise. Only the `markdown` job uses mise, through `jdx/mise-action`. It installs the
tools in `mise.toml`, Node included. Then the job runs `markdownlint-cli2 '**/*.md'`.

Deferred until milestone 1 ships, because they depend on each other: `commit.yaml`, release-please, release binaries,
`claude.yaml`, `codeql.yaml` and the Homebrew formula.

### Copied Text

The `.editorconfig`: `[*]` and YAML from `telegram-jung2-bot`, `[*.md]` for this repo.

```ini
root = true

[*]
indent_style = space
indent_size = 4
end_of_line = lf
charset = utf-8
trim_trailing_whitespace = true
insert_final_newline = true

[*.{yml,yaml}]
indent_size = 2

[*.md]
max_line_length = 140
```

The `renovate.json5` exists already in the repo root. Read it there.

Renovate's `cargo` and `rust-toolchain` managers are built in, so no custom manager is needed. The `rust-toolchain` manager
matches `rust-toolchain.toml`.

The `zizmor.yaml`: copy `.github/workflows/zizmor.yaml` from `https://github.com/siutsin/heartbeats` as is.

`lint.yaml` and `test.yaml` reuse its checkout step, `actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1`
with `persist-credentials: false`. The `markdown` job adds
`jdx/mise-action@8418fedb1110381a0d3ba51a7a415d7c1b884550 # v5.2.2` before `markdownlint-cli2 '**/*.md'`. This tag was checked on
25 Oct 2026. It is the newest one past the 3 day Renovate age.

### Done When

1. The four checks in the `README.md` Development section are green locally.
2. The four checks are green in CI on a pull request. `cargo test` is green on both operating systems.
3. `cargo run -- -v` prints `gitup 0.1.0`.
4. `README.md` has the Install section, marked as planned, the Development checks and the Credit section.
5. The first Renovate run shows `rust-toolchain.toml` and the `Cargo.toml` crates as tracked.

## Module Breakdown

Rule: one flat binary crate with four source files. No `lib.rs`, no workspace, no traits. A file splits only when it has a
second reason to change. Unit tests sit inline. Integration tests run the built binary.

| File           | Owns                                                                                              | Surface                                                  | Lines |
|----------------|---------------------------------------------------------------------------------------------------|----------------------------------------------------------|-------|
| `main.rs`      | `Cli`, the `-e` split, the action order, every write to stdout, colour                            | `main`                                                   | ~260  |
| `bookmarks.rs` | File path, load, save, and add, delete and clean on a `Vec<String>`. No printing                  | `default_path`, `load`, `save`, `add`, `delete`, `clean` | ~80   |
| `discover.rs`  | One path to repos: `~` expansion, normalise, repo check, comment, glob, depth walk, names. No git | `Repo`, `Resolved`, `resolve`, `expand`, `normalise`     | ~120  |
| `update.rs`    | `run`, workdir, git queries, porcelain parse, fetch error clean, branch logic, `-e` output split  | `Report`, `Step`, `Outcome`, `update_repo`, `exec_repo`  | ~280  |

Split `update.rs` only if it grows past about 400 lines.

### Data Flow

```text
Cli -> paths and bookmarks -> discover::resolve -> Resolved -> update_repo or exec_repo -> Report -> main renders
```

1. `Repo` is `{ name: String, git_dir: PathBuf, workdir: Option<PathBuf> }`. `discover.rs` sets `workdir` for the `.git`
   cases and a linked worktree admin dir. `None` means the path is another git dir, and `update.rs` asks git for the
   workdir, as in Git Commands Per Repo.
2. `Resolved` is `Repos { base, repos }`, `Comment(String)`, `Missing(PathBuf)` or `NotRepo(PathBuf)`.
3. `Report` is `{ name, steps: Vec<Step> }`.
4. `Step` is `Fetch(remote, Outcome)`, `Update(branch, Outcome)`, `Error(String)` or `Output(String)`.
5. `Outcome` is `Fetched(Vec<(Kind, Vec<String>)>)`, `UpToDate`, `Done`, `Skipped(&'static str)` or `Failed(String)`.
6. `Kind` is `NewBranch`, `NewTag` or `BranchUpdate`. `main.rs` picks the singular or plural label from the count.
7. Bookmark functions return the lists to print, such as added and already bookmarked. `main.rs` saves, then prints.
8. `bookmarks::default_path` and `discover::expand` take the `HOME` and `XDG_CONFIG_HOME` values as arguments, and
   `discover::resolve` passes `HOME` on. Only `main.rs` reads them, with `std::env::var_os`. Edition 2024 makes
   `env::set_var` unsafe and `unsafe_code = "forbid"` bans it, so tests pass values instead.

### Rust Conventions

A scratch crate on Rust 1.99.1, edition 2024 and clap 4.6.7 checked items 3, 5 and 6 under the lint policy.

1. Arguments borrow: `&Path`, `&str` and slices. Return values own: `PathBuf`, `String` and `Vec`. No `Cow` and no clone
   to satisfy the borrow checker.
2. `run` is `fn run<S: AsRef<OsStr>>(dir: &Path, argv: &[S]) -> io::Result<Output>`. It takes the program from
   `argv.split_first()` and calls `Command::new`, `args`, `current_dir`, `env`, `stdin(Stdio::null())` and `output()`.
   So every git call starts its argv with `"git"`, as in `&["git", "fetch", "--porcelain", remote]`, and `-e` passes its
   `Vec<String>` as is. Name the `split_first` rest binding `rest`, because pedantic `similar_names` rejects `args` beside `argv`.
3. Use `let else` for early returns and `if let` chains where they read better than nested `if let`. Edition 2024 allows
   both.
4. Use iterator chains, `split_first` and slice patterns, not index loops. The branch loop iterates the sorted names and
   updates the snapshot map, so it does not borrow the map while it walks it.
5. Modules are private `mod` items. Items that another module uses are `pub(crate)`. Pedantic `must_use_candidate` and
   `missing_errors_doc` do not fire on these items in a binary crate.
6. Each module has a `//!` line, and each `pub(crate)` item has a `///` line. `Report` has `#[must_use]`, because a dropped
   report loses output.
7. Each data type derives `Debug`. Types that a test compares also derive `PartialEq` and `Eq`. `Kind` also derives
   `Clone` and `Copy`. No other derive until a caller needs it.
8. Unit tests sit in `#[cfg(test)] mod tests` at the end of each file. Integration tests live in `tests/`.

### Not Added

- `report.rs`, `cli.rs`, `git.rs` and `exec.rs`: each has one caller and changes with it. Printing in one file is the
  concurrency rule.
- A `GitBackend` trait: one implementation. Tests use real temporary repos.
- A `config` module, a `util` module, `Repo` methods and error enums: the only setting is a `Cli` field, the only shared
  helpers are `discover::expand` and `discover::normalise`, and git failures are report text.
- `impl Display` for `Step` and `Outcome`: colour is a runtime choice, and `Display` has no way to receive it. One render
  function in `main.rs` takes the colour flag and uses `write!`. `Kind` gives its label through a `match`.

### Tests

| Layer         | Where                 | Method                                                                                                                                                                                                                                                                                                                                                                 |
|---------------|-----------------------|------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| Unit          | Inline `#[cfg(test)]` | Table driven with named cases for pure functions. Porcelain parse and names: `refs/remotes/origin/feat/x` is `feat/x`, a mirror `refs/heads/feat` is `feat`, a `*` line under `refs/heads/` is a new branch. Fetch error clean. Normalise. Repo names                                                                                                                  |
| `-e` split    | `main.rs` tests       | `echo # x` gives `["echo", "#", "x"]`. `a#b` gives `["a#b"]`. `a 'b` fails. `" "` gives no words. `"a\tb"` keeps the backslash. `a"b c"d` gives `["ab cd"]`. `''` gives `[""]`. `a\` fails. `"\f"` gives `["\f"]`                                                                                                                                                      |
| `-e` output   | `update.rs` tests     | `a\n` gives `["a"]`. `a\n\nb\n` gives `["a", "", "b"]`. `a\rb` gives `["a", "b"]`. `a\r\nb` gives `["a", "b"]`. An empty line renders as 8 spaces                                                                                                                                                                                                                      |
| Bookmarks     | `bookmarks.rs` tests  | `-a x x` adds one and reports the second as already bookmarked. `-d x` on `[x, x]` leaves `[x]`. `-b ""` uses the default path. `default_path` with an XDG value of `~/cfg` expands the `~`. `clean` keeps a broken symlink. `-a ""` adds the current directory. `-a ~/a/../b` stores `~/b`. `-a ~user/x` stores `~user/x`. Invalid UTF-8 in the file is replaced      |
| Discovery     | `discover.rs` tests   | `tempfile` trees: depth `0`, `3` and `-1`, a glob, a comment, a missing path, a symlinked dir, a broken symlink with `0 repos`, a linked worktree, a linked worktree admin dir, a submodule `.git` file with a relative `gitdir`, a bare repo, an empty path, a `chmod 000` directory that the walk skips. `expand` keeps `~user` and `$HOME` as typed                 |
| Repo errors   | `tests/cli.rs`        | One case each: `-c` with a detached HEAD, `-c` with no tracked remote, `-c` with `branch.<b>.remote` naming an unconfigured remote, no remotes. Each prints only its error line, with no fetch and no branch line                                                                                                                                                      |
| Fetch table   | `tests/cli.rs`        | A bare repo runs in its git dir. A remote with no fetch refspec prints `Fetching <remote>: skipped: no configured refspec.` A second remote fails after one worked. `-p` prunes a deleted branch. A `--mirror` clone. `-c` on an unborn branch with an upstream in config. A git dir with a relative `core.worktree` runs in the worktree resolved against the git dir |
| Branch table  | `tests/cli.rs`        | No upstream, up to date, fast-forward, diverged, unrelated history, dirty tree, detached HEAD, gone upstream, local upstream that moved earlier, a tag named like the branch. A branch checked out in another worktree prints `Updating <branch>: error: ...`                                                                                                          |
| Exec and exit | `tests/cli.rs`        | `-e ""` runs a normal update. `-e " "` and `-e "a 'b"` exit `2` with no banner. `-h` and `-v` print no banner. A bad bookmark write exits `1`. `-e nosuchcmd` prints `Error: nosuchcmd: <io error>`. `-a x -e " "` exits `2` and does not write the bookmark file                                                                                                      |
| Parity        | Manual, Step 5        | One fixture cannot serve two runs, because the first run updates it. So make two copies of one fixture with local file remotes. Run Python `gitup` on one copy and `target/release/gitup` on the other, both through a pipe, then diff the output. No diff outside the Deliberate Deviations, after removing Python's progress counter text                            |

No test changes the process environment. Unit tests pass `HOME` and `XDG_CONFIG_HOME` values as arguments. Each
integration test runs `env!("CARGO_BIN_EXE_gitup")` on real git repos in `tempfile` dirs, and sets the child environment
with `Command::env`. `HOME` and `XDG_CONFIG_HOME` point to its temporary dir. It also sets `GIT_CONFIG_GLOBAL=/dev/null`,
`GIT_CONFIG_NOSYSTEM=1` and fixed author and committer variables. No network. The suite must run in under 10 seconds.
Split `tests/cli.rs` only if it grows past about 400 lines.

`allow-unwrap-in-tests` covers only `#[test]` functions and `#[cfg(test)]` code. A plain helper in `tests/cli.rs` that
calls `unwrap` fails clippy. So the helpers, such as the repo builder and the run wrapper, sit in a `#[cfg(test)] mod`
inside `tests/cli.rs`.

## Build Order

Work starts when this plan is merged into `master`. Its Module Breakdown is the contract for Steps 2 to 4. Each step is
one pull request that ends green on the four checks in the `README.md` Development section. Each step wires its code into
`main.rs`. It adds only the `Cli` fields, struct fields, variants and constants that it reads. Anything unread fails
`-D warnings`. Each crate arrives in the step that first uses it: `clap` in 1, `glob` and `tempfile` in 2. The order puts
the least git heavy code first.

| Step | Scope                                                                                                                                                                                                                                                                                                                                                                                                                                                      | Done when                                                                                                           |
|------|------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------|---------------------------------------------------------------------------------------------------------------------|
| 1    | Foundation and build infra, as above                                                                                                                                                                                                                                                                                                                                                                                                                       | Step 1 Done When                                                                                                    |
| 2    | Bookmarks and discovery, which share `expand`, `normalise` and `glob`. `Cli` gains `paths`, `add`, `delete`, `list`, `clean`, `bookmark_file` and `depth`. `bookmarks.rs`. `discover.rs` with `resolve` and `Repo` with `name` only. Banner and colour in `main.rs`. `main.rs` resolves each given path, then the bookmarks when no other action ran. It prints comments, headers, names, path errors and the no bookmarks hint. Add `glob` and `tempfile` | Each bookmark message and every discovery header and name matches Python                                            |
| 3    | `Cli` gains `current_only`, `fetch_only`, `prune` and the `struct_excessive_bools` expect. `Repo` gains `git_dir` and `workdir`. `update.rs` with `run`, workdir, fetch and branch logic. `main.rs` renders reports                                                                                                                                                                                                                                        | The repo error, fetch and branch tables pass on real temporary repos                                                |
| 4    | `Cli` gains `exec` and `update`. `exec_repo`, the `-e` split and the full action order                                                                                                                                                                                                                                                                                                                                                                     | Every Behaviour item has a test                                                                                     |
| 5    | Parity run on two copies of one fixture, as in the Tests Parity row. A separate timing run of both tools on a real bookmarks file. README usage and `cargo install --locked --path .`. Update the `README.md` Install text. Install Python `gitup` 0.5.2 from the upstream tag `v0.5.2`, for example with `pipx install git+https://github.com/earwig/git-repo-updater@v0.5.2`                                                                             | No diff outside the Deliberate Deviations, after removing Python's progress counter text. Both timings are recorded |

The Step 5 timing of Python `gitup` is the baseline for milestone 2.
