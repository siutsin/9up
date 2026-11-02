# Releasing

Releases run from git tags: cargo-dist builds the binaries, creates the GitHub Release and
publishes a Homebrew formula to the `siutsin/homebrew-tap` tap.

One-time setup:

- Create the `siutsin/homebrew-tap` repository on GitHub.
- In this repo, create a `deployment` environment and add a `HOMEBREW_TAP_TOKEN` secret to it, holding a token with
  write access to the tap. The workflow's own token cannot write to another repository, so
  the formula publish step uses this one, and its job declares the environment to read it.

Cut a release from a clean `master`, with cargo-release installed
(`cargo install cargo-release --version =1.1.7 --locked`):

```sh
cargo release patch --execute
```

Use `minor` or `major` in place of `patch` when the change calls for it. The command bumps
the version in `Cargo.toml`, commits, tags `v<version>` and pushes. The tag runs the release
workflow. When it finishes, check the formula landed and installs:

```sh
brew install siutsin/tap/gau-up
```

To change targets or installers, edit `dist-workspace.toml`, run `dist generate` and commit
the result. A fresh generate overwrites the hand trims and the publish job's environment
line, so re-apply both before committing. `dist` itself comes from its installer, at the version the config pins:
`curl --proto '=https' --tlsv1.2 -LsSf https://github.com/axodotdev/cargo-dist/releases/download/v0.33.1/cargo-dist-installer.sh | sh`.
The workflow started as `dist generate` output and is now maintained by hand, trimmed to
what this repo needs; the config's `allow-dirty` entry records that it differs from the
generated file. One line in it is local: `environment: deployment` on the publish job, so
it can read the tap token from that environment. The workflow's zizmor findings are
accepted in the root `zizmor.yaml`, scoped to that file.
