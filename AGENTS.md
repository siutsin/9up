# 9up

Read the plan in `.agents/planning.md` before you change code.
Do not implement before this plan is merged into `master`.

Setup is in `README.md`. Verify every change with `cargo fmt --check`,
`cargo clippy --locked --all-targets -- -D warnings`, `cargo test --locked` and `markdownlint-cli2 '**/*.md'`.

## Rules

1. The Feature Spec in `.agents/planning.md` is the reference for what 9up does. `gitup` is the inspiration, not the spec. The CLI details and the output wording are 9up's own. A difference from `gitup` that is not in the plan's Differences table needs a row there first.
2. `update_repo` and `exec_repo` return data. Only `main.rs` writes output.
3. Start every child process through `run(dir, argv)`.
4. No traits, error enums or extra modules without a reason in the plan.
5. Write idiomatic Rust: the current edition and syntax, the Rust API Guidelines, std first.
6. Use the GitHub pull request workflow. Work on a branch, never on `master`. Merge only when CI is green. Ask before pushing.

