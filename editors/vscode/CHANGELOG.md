# Changelog

## 0.1.2

- A missing ridx or an available update also shows in the status bar and installs on click: VS Code hides the
  notification after a while, and a missed one left Rust files without a server and without a sign of why.
- Commands no longer wait for the install notification to be answered: activation returns at once.
- "Show rust-analyzer" opens the rust-analyzer extension's page, where it can be disabled; it did nothing before.

## 0.1.1

- An older ridx than the latest release (one installed long ago in `~/.local/bin`, say) is offered an update; the
  `rustIntelligence.server.checkForUpdates` setting turns the check off.
- Declares that it does not run in Restricted Mode: indexing builds the workspace, which runs its build scripts.

## 0.1.0

- Runs `ridx lsp` for Rust files; finds `ridx` or installs the latest release after asking, checked against its SHA-256.
- Warns when the rust-analyzer extension is enabled too.
