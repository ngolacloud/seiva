<!-- Thanks for contributing! Keep PRs focused — one concern reviews faster. -->

## What and why

What does this change, and why? Link the issue it closes (`Closes #…`).

## How verified

How did you check it works? (a test, a reproducible run, a fitness lint)

## Checklist

- [ ] `cargo fmt --all --check` passes
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` passes
- [ ] `cargo test --workspace` passes
- [ ] `cargo deny check` passes (if dependencies changed)
- [ ] Docs/`CHANGELOG.md` updated for user-visible changes
- [ ] The DORM boundary is intact (core does not depend on DORM)
- [ ] `seiva lint` passes (if a module or layer changed)
- [ ] An ADR is added/updated (if a pattern, boundary or public contract changed)
