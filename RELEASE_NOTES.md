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
- **Known limitation:** the CI required-status-check step (branch protection)
  is a manual follow-up in the GitHub UI, not something this PR sets — see the
  repo-config skill's `ci-and-branch-protection.md` for the exact steps.
