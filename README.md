# 9up

Update many git repositories at once.

## Install

Not yet. `Cargo.toml` arrives in Step 1.

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
