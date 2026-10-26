# 9up

Update many git repositories at once.

## Install

Requires `git` on the `PATH` at version 2.41 or later.

```sh
cargo install --path .
```

## Usage

```sh
9up                          # update every bookmarked repository
9up ~/src ~/work             # update the repositories under these paths
9up update -c ~/src          # fetch only the remote the current branch tracks
9up add ~/src ~/work         # bookmark paths
9up delete ~/src             # remove a bookmark
9up list                     # show the bookmarks
9up clean                    # drop bookmarks whose paths no longer exist
9up exec ~/src -- git status # run a command in each repository under a path
```

Bookmarks live one path per line in `$XDG_CONFIG_HOME/gitup/bookmarks`, or `~/.config/gitup/bookmarks` when
the variable is unset or empty, and a relative value is ignored — the format is `gitup`'s, so an existing file
carries over. Pass `-b <path>` to use a different file, or `-b ""` for the default. Update takes
`--current-only`, `--fetch-only`, `--prune` and
`--depth <n>` (short `-t`; negative means no limit). `exec` takes `--depth` too.

## Development

Run `mise install` for the non-Rust tools. Then check your change:

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
markdownlint-cli2 '**/*.md'
```

## Credit

9up is inspired by [`gitup`](https://github.com/earwig/git-repo-updater) (git-repo-updater) 0.5.2, the Python tool by
Ben Kurtovic.
