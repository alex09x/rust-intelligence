# Contributing to Rust Intelligence (`ridx`)

Thank you for contributing to Rust Intelligence (`ridx`). To maintain consistent code quality, performance, and international developer accessibility, please adhere to the following project standards.

---

## 1. Language Policy: Strictly 100% English

- **All source code, comments, docstrings, and test fixtures** must be written in English.
- **Git commit messages, PR descriptions, and issue discussions** must be written strictly in English.
- **No Cyrillic or non-English natural language text** in committed files or commit history. (Non-ASCII Unicode characters used as test cases for UTF-8/UTF-16 encoding and surrogate pair handling, such as Greek letters or emojis like `🦀`, are permitted within specific Unicode test fixtures).

---

## 2. Compact Source Files Policy

- **Target size**: 100–300 lines per file (the sweet spot).
- **Upper limit**: 400 lines. When a file exceeds 400 lines, decompose it into cohesive submodules or domain-specific files.
- **Single responsibility**: Each file should represent one primary type (struct/trait/enum) and its immediate methods, or a cohesive handler/endpoint group, plus private helpers.

---

## 3. Testing and Verification

- **Deterministic tests**: Tests must be fully reproducible, zero-flake, and deterministic (e.g., fixed RNG seeds in property tests).
- **Zero Daemon Overhead**: Tests must run without requiring a background daemon or persistent external state.
- **Pre-commit Gate**: Always run formatting, lints, and affected test suites before committing changes.
