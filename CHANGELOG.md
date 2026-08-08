# Changelog

All notable changes to this repo are documented here.
Format: Added / Changed / Deprecated / Removed / Fixed / Security, newest first.

## [Unreleased]
### Added
- CLI (`library`, in `cli/`) to inventory ebooks across Humble Bundle, Packt,
  and Manning (live fetch via a pasted session cookie/token), Kindle (CSV
  import), and manual entries, stored in a local SQLite database.
- Duplicate detection (`library check`, and warnings on `import`/`add`) via
  exact ISBN match, exact normalized-title match, and Jaro-Winkler fuzzy
  title similarity.
- Native desktop app (`library-desktop`, in `desktop/`, Tauri) covering the
  same functionality — browse/search, add/edit/remove, run imports, manage
  credentials — from a GUI backed by the same database and config file.
- Shared logic extracted into `library-core` (`core/`), used by both
  frontends.
- CI workflow (fmt + clippy + test, gated on the whole workspace) and
  standard repo governance docs (this file, RELEASE_NOTES, ARCHITECTURE,
  CONTRIBUTING, CODE_OF_CONDUCT, SECURITY, PR/issue templates, ADR log).

### Changed
### Fixed
### Security

<!-- ## [0.1.0] - YYYY-MM-DD
### Added
- Initial release -->
