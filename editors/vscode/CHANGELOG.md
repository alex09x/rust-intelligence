# Changelog

## 0.1.1

- An older ridx than the latest release (one installed long ago in `~/.local/bin`, say) is offered an update; the
  `rustIntelligence.server.checkForUpdates` setting turns the check off.
- Declares that it does not run in Restricted Mode: indexing builds the workspace, which runs its build scripts.

## 0.1.0

- Runs `ridx lsp` for Rust files; finds `ridx` or installs the latest release after asking, checked against its SHA-256.
- Warns when the rust-analyzer extension is enabled too.
