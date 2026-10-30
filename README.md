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

### Releasing

Releases run from git tags: cargo-dist builds the binaries, creates the GitHub Release and
publishes a Homebrew formula to the `siutsin/homebrew-tap` tap.

One-time setup:

- Create the `siutsin/homebrew-tap` repository on GitHub.
- Add a `HOMEBREW_TAP_TOKEN` secret to this repository, holding a token with write access to
  the tap. The workflow's own token cannot write to another repository, so the formula
  publish step uses this one.

The first release is `v0.1.0`, the version already in `Cargo.toml`. Tag it directly once the
setup above is done:

```sh
git tag v0.1.0
git push origin v0.1.0
```

Every later release, from a clean `master`, with cargo-release installed
(`cargo install cargo-release --version =1.1.7 --locked`):

```sh
cargo release patch --execute
```

Use `minor` or `major` in place of `patch` when the change calls for it. The command bumps
the version in `Cargo.toml`, commits, tags `v<version>` and pushes. The tag runs the release
workflow. When it finishes, check the formula landed and installs:

```sh
brew install siutsin/tap/9up
```

To change targets or installers, edit `dist-workspace.toml`, run `dist generate` and commit
the result. `dist` itself comes from its installer, at the version the config pins:
`curl --proto '=https' --tlsv1.2 -LsSf https://github.com/axodotdev/cargo-dist/releases/download/v0.33.1/cargo-dist-installer.sh | sh`.
The generated workflow carries hand adjustments (action pins, scoped permissions, zizmor
ignores), and the config's `allow-dirty` entry keeps `dist generate` from overwriting them
silently — reapply the adjustments to the regenerated file before committing.

## Credit

9up is inspired by [`gitup`](https://github.com/earwig/git-repo-updater) (git-repo-updater), the Python tool by
Ben Kurtovic.
