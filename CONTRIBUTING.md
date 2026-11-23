# Contributing

Tonalli is free software (GPL-3.0-or-later). Fixes and improvements are welcome through pull requests.

- Keep changes small and focused; one topic per pull request.
- Before you open one, run `cargo fmt --all`, `cargo clippy --workspace -- -D warnings` and
  `cargo test --workspace`.
- Do not commit private data: local paths, names, emails, keys or tokens. `ci/privacy.py` checks for
  this; see `ci/test_privacy.py` for the rules.
- Keep third-party licences exact. New dependencies must be recorded in `LICENSES.md` with their
  version and licence, and must be compatible with GPL-3.0-or-later.
- Notes stay encrypted, the server serves installed packs only, and bad packs return errors instead of
  panics. See `AGENTS.md` for the full rules.

By contributing you agree your work is released under GPL-3.0-or-later.

## Releases and updates from a fork

Updates are signed. A fork publishes its own with `scripts/publish-update.ps1 -Repo you/your-fork
-AssetsDir <dir> -KeyPath <your key>`; users must enter your fork's address and your public key in
Settings, so a fork can never push updates to users of this project. Never commit a private key.
