# 9up

Read the plan in `.agents/planning.md` before you change code.

Setup is in `README.md`. Verify every change with `cargo fmt --check`,
`cargo clippy --locked --all-targets -- -D warnings`, `cargo test --locked` and `markdownlint-cli2 '**/*.md'`.
Coverage is a gate: `mise run coverage` fails under 100 percent regions or functions
in production code. Unit tests live in `src/<module>/tests.rs`.

## Rules

1. The Feature Spec in `.agents/planning.md` is the reference for what 9up does. `gitup` is the inspiration,
   not the spec. The CLI details and the output wording are 9up's own. A difference from `gitup` that is not
   in the plan's Differences table needs a row there first.
2. `update_repo` and `exec_repo` return data. Only the front end (`main`, `commands`, `render`) writes output.
3. Start every git child process through the `Sys` seam (`RealSys::git` in `sys.rs`). The user's command
   in `exec_repo` goes through `Sys::exec`: the same discipline (no shell, null stdin, explicit directory).
4. Everything is built in `main` and passed down in the `App` context: no environment reads, `dirs` calls
   or service construction below `main`. The `Sys` trait is the standing exception to the no-traits rule,
   for the owner's composition-root architecture (plan, Module Breakdown). No other traits, error enums or
   extra modules without a reason in the plan.
5. Write idiomatic Rust: the current edition and syntax, the Rust API Guidelines, std first.
6. Use the GitHub pull request workflow. Work on a branch, never on `master`. Run the local checks before
   every push. Merge only when CI is green. Ask before pushing.
