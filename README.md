# 9up

Update many git repositories at once.

## Install

Requires `git` on the `PATH` at version 2.41 or later.

```sh
cargo install --path .
```

## Usage

```sh
9up                          # update every configured repository
9up ~/src ~/work             # update the repositories under these paths
9up update -c ~/src          # fetch only the remote the current branch tracks
9up add ~/src ~/work         # add paths to the config file
9up delete ~/src             # remove a path from the config file
9up list                     # show the configured paths
9up clean                    # drop configured paths that no longer exist
9up exec ~/src -- git status # run a command in each repository under a path
```

The config file is TOML. It holds one key, `repositories`, an array of path strings:

```toml
repositories = [
    "/home/you/src",
    "/home/you/work"
]
```

### Config file

The default is `$XDG_CONFIG_HOME/9up/config.toml`, else `~/.config/9up/config.toml`, on every platform,
macOS included. A relative `XDG_CONFIG_HOME` is ignored with a warning, and an empty one counts as unset.

- `-C`/`--config <path>` uses a different file. `-C ""` means the default.
- Other keys in the file are left alone.

### Options

| Command  | Options                                                                                          |
|----------|--------------------------------------------------------------------------------------------------|
| `update` | `--current-only`, `--fetch-only`, `--prune`, `--depth <n>` (short `-t`; negative means no limit) |
| `exec`   | `--depth <n>`                                                                                    |

## Development

Run `mise install` for the non-Rust tools. Then check your change:

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
mise run coverage
markdownlint-cli2 '**/*.md'
```

## Credit

9up is inspired by [`gitup`](https://github.com/earwig/git-repo-updater) (git-repo-updater), the Python tool by
Ben Kurtovic.
