# Release Notes

<!--
Two variants, pick the one that fits this repo's actual unit of change:

1. No version tags yet (pre-1.0, nothing published) — track by PR instead, same way
   AISF does it: one entry per merged PR against main, reverse chronological, each
   linking to its PR and (where one exists) to the doc that covers the change in full
   detail. Use "## PR #N — <summary>" headers.

2. Actual version tags exist — use "## vX.Y.Z - YYYY-MM-DD" headers instead, each
   linking to the PRs it shipped and a compare link to the previous tag. Add an
   "### Upgrade notes" subsection under any entry with a breaking change.

Either way, keep the tone AISF's file uses: bolded category tags inline in the
bullet (**Added:** / **Changed:** / **Fixed:**), not separate subheaders per
category — and state known limitations or deliberate scope cuts plainly instead of
leaving them implied.
-->

Tracks notable changes to this repo, one entry per merged PR against `main`,
reverse chronological (no version tags yet — this is pre-1.0 and unpublished).

---

## PR #6 — Run cargo fmt --all to fix CI-blocking formatting drift
**2026-08-21** · [#6](https://github.com/baileyrd/Library/pull/6)

- **Fixed:** `main`'s `check` workflow had been failing on `cargo fmt --all --
  check` since commit 8a7ef99, blocking every PR's `check` job (including
  doc-only ones) from passing. Ran `cargo fmt --all`; touched
  `cli/src/main.rs`, `core/src/db.rs`, `core/src/enrich.rs`,
  `core/src/sources/humble.rs`, and `desktop/src-tauri/src/commands.rs` —
  all whitespace/line-wrapping, no logic changes. Closes #5.
- **Fixed:** with `Format` finally passing, `Clippy` ran against current
  `main` for the first time and surfaced two genuine, pre-existing lints in
  `desktop/src-tauri/src/commands.rs` that had never actually been checked
  before (`needless_borrows_for_generic_args` on
  `.initialization_script(&capture::injected_script(spec))`, and
  `question_mark` on an `if ... .is_none() { return None; }` block inside a
  closure already returning `Option<String>`). Fixed both with clippy's own
  suggested rewrites — no behavior change.

## PR #4 — Fix documentation drift found in a docs-loop audit pass
**2026-08-21** · [#4](https://github.com/baileyrd/Library/pull/4)

- **Fixed:** README.md's `check` weak-match description claimed matches down
  to 0.75 confidence show under "review manually", but `CHECK_WEAK_THRESHOLD`
  is 0.90 in both the CLI and desktop app (same as the strong cutoff), so
  that section is currently always empty in practice — the doc now says so
  and explains why (a lower floor produced only false positives, per this
  file's own PR #3-adjacent history in CHANGELOG.md).
- **Fixed:** ARCHITECTURE.md referenced the desktop crate's paths
  (`src-tauri/src/commands.rs`, `frontend/`) without the `desktop/` prefix,
  so they didn't resolve from the repo root.
- **Fixed:** `ci-rust.yml`'s Clippy step comment cited "desktop/src-tauri's
  mutex-lock unwraps" as an unwrap() example; that crate uses
  `parking_lot::Mutex` specifically to avoid needing `lock().unwrap()`, so
  no such calls exist there anymore. The still-true `core/src/dedup.rs`
  example is kept.
- **Added:** `docs-audit.md` — the full whole-repo documentation audit this
  PR is based on, including 9 confirmed-accurate claims and 1 unverifiable
  claim logged so a future audit doesn't re-litigate them.

## PR #3 — Apply standard repo governance files and fix CI-blocking issues they surfaced
**2026-08-08** · [#3](https://github.com/baileyrd/Library/pull/3)

- **Added:** PR/issue templates, CONTRIBUTING, CODE_OF_CONDUCT, SECURITY,
  CHANGELOG, this file, ARCHITECTURE (with a real boundary table, not the
  scaffold), an ADR seed, and a Rust CI workflow (fmt + clippy + test).
- **Fixed:** the existing code wasn't rustfmt-clean and had one clippy lint
  (`print_literal` in the CLI's `list` table header) — both would have made
  the new CI red from the first push, so both are fixed as part of adding it.
- **Fixed:** the CI workflow as generated didn't account for this repo having
  a Tauri desktop crate — GitHub's `ubuntu-latest` runners don't ship
  `libwebkit2gtk` and friends, so `cargo build`/`clippy`/`test` would fail to
  even compile `desktop/` without an explicit apt-get install step. Added one,
  and switched `clippy`/`test` to `--workspace` so all three crates are
  actually covered, not just whichever one Cargo defaults to.
- **Known limitation:** the CI required-status-check step is a manual
  follow-up — under this repo's Settings -> Branches, add a branch protection
  rule for `main` requiring the `check` job to pass before merging. Nothing
  in this PR sets that automatically.
- **Fixed:** a documentation accuracy pass over every doc in the repo caught
  and fixed several drift issues: two stale `src/...` paths left over from
  the pre-workspace layout (README's Manning troubleshooting note,
  desktop/README's `core/` crate reference), an incorrect claim in
  desktop/README that `cargo tauri build` outputs to
  `desktop/src-tauri/target/` (this is a Cargo workspace — `target/` is
  shared at the repo root), an inaccurate "no unwrap/expect outside tests"
  claim in the CI workflow's own comment (untrue — see e.g.
  `core/src/dedup.rs`'s `Regex::new(...).unwrap()` calls and
  `desktop/src-tauri`'s mutex-lock unwraps; the comment now says so plainly
  instead of asserting a standard the code doesn't meet), two comments
  pointing readers at a `references/ci-and-branch-protection.md` path that
  only exists inside the repo-config skill's own directory, not this repo
  (replaced with the actual steps inline), and an empty CHANGELOG.md
  `[Unreleased]` section despite the CLI and desktop app already existing on
  `main` (backfilled with what's actually shipped).
