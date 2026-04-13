# Agent Rules

## General

- This is a Rust project (`grip`). Always run `cargo check` before considering a task complete.
- Do not add dead code, unused imports, or speculative abstractions.
- Prefer editing existing files over creating new ones.
- Do not add comments unless the logic is genuinely non-obvious.

## Tests

- Always update any failing tests caused by your changes.
- If you add new behavior, add a corresponding test.
- Run `cargo test` after every non-trivial change and fix any failures before finishing.

## Code Style

- Follow existing naming and formatting conventions in the file you are editing.
- Run `cargo fmt` before finalizing changes.
- Do not suppress clippy warnings with `#[allow(...)]` unless there is a clear, documented reason.

## Documentation

- After every change, check whether any of the doc files (`README.md`, `COMMANDS.md`, `OVERVIEW.md`, `CONTRIBUTING.md`) need updating to reflect the change.
- If a command, flag, or behavior is added, removed, or renamed, update the relevant doc files before finishing.

## Commits

- Do not commit unless explicitly asked.
- Keep commit messages short and focused on *why*, not *what*.
