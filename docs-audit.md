# docs-audit.md

Docs-loop audit of `baileyrd/Library`, run 2026-08-21. Whole-repo scope (every
tracked `*.md`), full current state (not diff-scoped). No doc-comment (`///`)
audit performed — not requested.

Ground truth built from: `Cargo.toml`/crate manifests, `cli/src/cli.rs` +
`main.rs`, `core/src/{dedup,config,model,db,enrich}.rs`,
`core/src/sources/{humble,packt,manning,kindle,capture}.rs`,
`desktop/src-tauri/src/{commands,state,lib}.rs`, `.github/workflows/ci-rust.yml`,
`.github/ISSUE_TEMPLATE/`, `.github/PULL_REQUEST_TEMPLATE/`, `docs/adr/`, and
`git log`/`CHANGELOG.md`/`RELEASE_NOTES.md` as the record of prior intentional
changes (used to distinguish "code changed, doc didn't" from "doc was always
wrong").

## Findings

| Doc | Where | Claim | Classification | Ground truth | Fix | Size |
| --- | --- | --- | --- | --- | --- | --- |
| README.md | `Usage`, L164-165 | "`check` additionally shows weaker matches down to 0.75 confidence under a separate 'review manually' section" | stale | `cli/src/main.rs` `CHECK_WEAK_THRESHOLD = 0.90` (identical constant in `desktop/src-tauri/src/commands.rs`, explicit comment: "Set equal to the 0.90 strong-match cutoff... This makes the 'weak' bucket structurally always empty for now"). `CHANGELOG.md` `[Unreleased]/Fixed` confirms this was deliberately raised from 0.75 → 0.90, not an accident. | Update the sentence to state the weak-match cutoff is 0.90 (same as the strong cutoff), so the weak bucket is currently always empty — don't just swap the number, since "down to 0.75" is no longer a true description of what the code does at any confidence level | S |
| ARCHITECTURE.md | `Structure`, L35-36 | "`src-tauri/src/commands.rs` is a thin `#[tauri::command]` wrapper layer, `frontend/` is static HTML/CSS/JS" | stale (precision) | Real paths from repo root are `desktop/src-tauri/src/commands.rs` and `desktop/frontend/`; both files/dirs exist, just not at the written path relative to repo root (`check_references.py` flags both `unresolved`) | Prefix both with `desktop/` for a repo-root-relative reading; low-severity since the surrounding sentence already establishes the `desktop/` context, but cheap to fix and removes the only two unresolved paths in the repo's docs that aren't a build artifact, a URL fragment, or historical | S |

## Accurate (checked, confirmed — logged so a re-run doesn't re-litigate)

| Doc | Where | Claim | Ground truth |
| --- | --- | --- | --- |
| README.md | `Usage`, L160-163 | Import/add duplicate warnings fire at "confidence >= 0.90 fuzzy title match, or ISBN match, or exact-title match" | `core/src/dedup.rs`: `FUZZY_THRESHOLD = 0.90`, ISBN match confidence `1.0`, exact-title match confidence `0.95` |
| README.md | `Kindle`, L119-120 | "Re-importing the same CSV updates existing rows (keyed on ISBN, or on the normalized title when no ISBN is given)" | `core/src/sources/kindle.rs`: `source_id = isbn.unwrap_or_else(\|\| normalize_title(&record.title))`; `core/src/db.rs` upserts on `UNIQUE(source, source_id)` |
| README.md | `Getting credentials` intro | Desktop app's automatic capture opens "a real embedded browser window on that site's own login page" for Humble/Packt/Manning, with Humble/Packt/Manning showing an extra "I'm logged in" button | `core/src/sources/capture.rs`: `HUMBLE_CAPTURE`/`PACKT_CAPTURE`/`MANNING_CAPTURE` specs, `Completion::ManualCookie`/`ManualCookieJar`, injected banner JS |
| README.md | `Packt` | "Packt migrated its owned-books API from `services.packtpub.com` (bearer-token auth) to `subscription.packtpub.com/api/entitlements` (cookie + CSRF-token auth)" | `core/src/sources/packt.rs` module doc + `XSRF_COOKIE_NAME`/`packt_session` handling matches exactly |
| README.md | `Humble Bundle` limitations | Uses `/api/v1/user/order` + `/api/v1/order/{gamekey}`; no real cover art from the API; bundle-exclude via `sources::humble::matches_excluded_bundle` | `core/src/sources/humble.rs` L93, L107, L229 (`is_fiction_or_comic`), L255 (`matches_excluded_bundle`) |
| ARCHITECTURE.md | `Data flow` | Upsert "keyed on `(source, source_id)` where the source has a stable id, or always inserts for sources without one" | `core/src/db.rs`: `UNIQUE(source, source_id)`, manual entries always pass `source_id: None` → always-insert branch |
| CONTRIBUTING.md | `Workflow` step 5 | "pick the template that matches (feature / bug fix / docs / chore)" | `.github/PULL_REQUEST_TEMPLATE/{feature,bug_fix,docs,chore}.md` all present |
| desktop/README.md | `Running it` | "no Node.js/npm involved" — frontend has no build step | `desktop/frontend/` contains only `app.js`, `index.html`, `styles.css` — no `package.json` |
| desktop/README.md | `Running it` | `cargo tauri build` "produces an installer/bundle under `target/release/bundle/` (repo root — this is a Cargo workspace, so `target/` is shared, not per-crate)" | Root `Cargo.toml` workspace with 3 members; matches `RELEASE_NOTES.md` PR #3's explicit fix of this exact claim |
| desktop/frontend/styles.css | (no doc claim to check — cross-referenced from CHANGELOG) | CHANGELOG's "Now `object-fit: contain`" claim about the covers grid | `desktop/frontend/styles.css` L173 |

## Historical (CHANGELOG.md / RELEASE_NOTES.md — not drift, not touched)

`check_references.py` flagged 4 paths in these two files as unresolved
(`subscription.packtpub.com/api/entitlements` as a historical URL fragment in
CHANGELOG.md, and `desktop/src-tauri/target/`, `target/`,
`references/ci-and-branch-protection.md` in RELEASE_NOTES.md's PR #3 entry).
All four are the log correctly recording something that used to be true (an
old API host, a wrong path that was fixed, a reference that was removed) — per
the skill's Rules, historical entries are never "fixed" to match the present.

## Unverifiable

| Doc | Where | Claim | Why unverifiable |
| --- | --- | --- | --- |
| SECURITY.md | `Reporting a vulnerability` | GitHub Security Advisories are (or aren't yet) set up on this repo | Nothing in the tree can confirm whether Advisories is enabled in the repo's GitHub settings; the doc already hedges this itself ("if advisories aren't set up on this repo yet") |

## Called out separately: code, not docs, and outside default scope

**`.github/workflows/ci-rust.yml`'s own inline comment** (not a tracked
`*.md`/`*.mdx` file, so outside this run's default doc surface per SKILL.md
step 0 — flagged only because `RELEASE_NOTES.md` PR #3 previously treated this
exact comment as an in-scope "documentation accuracy" fix, so the precedent is
already set in this repo):

> Warnings fail the build. Note: this repo does not currently forbid
> unwrap()/expect() in production code (see e.g. core/src/dedup.rs's
> Regex::new(...).unwrap() calls and desktop/src-tauri's mutex-lock
> unwraps) — "clean" here means no clippy lint warnings, not adherence to a
> stricter unwrap-free standard.

The `core/src/dedup.rs` half is still true (confirmed:
`Regex::new(...).unwrap()` x6). The `desktop/src-tauri's mutex-lock unwraps`
half is now **stale**: `desktop/src-tauri/src/state.rs` uses
`parking_lot::Mutex` specifically *because* "no poisoning to unwrap at every
call site" (its own doc comment says so), and a full grep of
`desktop/src-tauri/src/` finds zero `.lock().unwrap()` calls — the only
`.unwrap()`-family call left in that crate is an unrelated `.expect(...)` in
`lib.rs` running the Tauri app, plus one code *comment* referencing a
different crate's (`tauri-runtime-wry`) internal panic behavior. The code
changed (a deliberate parking_lot migration) after this comment was written;
the comment's supporting example didn't follow. Not touched in this run since
it's outside the requested scope — flagged for a decision on whether to
include it.

## Counts

| Classification | Count |
| --- | --- |
| stale | 2 (+1 out-of-scope, reported above) |
| missing | 0 |
| orphaned | 0 |
| aspirational | 0 |
| unverifiable | 1 |
| accurate | 9 |

## Harness mode

`LOOP_HARNESS_MODE` is unset → **interactive**. Nothing proceeds until rows
are picked, regardless of classification. (For reference: both `stale` rows
above are mechanically verifiable against a manifest/constant/path, so under
`auto` mode they'd be eligible to proceed unattended; the CI-comment note
would still need an explicit decision either way, since it's outside default
scope.)
