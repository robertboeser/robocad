# Changelog

## Unreleased

### M0 — Project skeleton & CI
- Cargo workspace with crates `rc-mesh`, `rc-kernel`, `rc-doc`, `rc-render`, `rc-app` (`robocad`), `rc-cli` (`robocad-cli`).
- Shared lints (clippy pedantic with selected allows, `unsafe_code = "deny"`), tuned dev/release profiles.
- GitHub Actions CI: fmt, clippy, tests on macOS, Linux and Windows.
