# Changelog

All notable changes to the Delonix Seiva VS Code extension are documented here.

## [0.1.0] - 2026-11-05

### Added

- `dorm-schema` language registration for `.schema` files, with `//` line
  comments and auto-closing pairs for `{ } ( ) [ ]` and `"`.
- TextMate grammar highlighting DORM schema keywords (`datasource`,
  `namespace`, `model`, ...), the `env(...)` and default functions, field and
  block attributes (`@...` / `@@...`), scalar types, strings, numbers,
  booleans, and comments.
- Snippets for DORM (`model`, `namespace`, `datasource`) and for Seiva Rust
  helpers (`seiva-facade`, `requires`, `seiva-controller`, `wire-schema`).
