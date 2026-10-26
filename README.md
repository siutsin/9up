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

The default file is `$XDG_CONFIG_HOME/9up/config.toml`, or `~/.config/9up/config.toml` when the variable is
unset or empty, and a relative value is ignored. On macOS the default is
`~/Library/Application Support/9up/config.toml`. Pass `-C`/`--config <path>` to use a different file, or
`-C ""` for the default. Other keys in the file are left alone. Update takes `--current-only`, `--fetch-only`, `--prune` and
`--depth <n>` (short `-t`; negative means no limit). `exec` takes `--depth` too.

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

9up is inspired by [`gitup`](https://github.com/earwig/git-repo-updater) (git-repo-updater) 0.5.2, the Python tool by
Ben Kurtovic.
