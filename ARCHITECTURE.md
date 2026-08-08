# Architecture

## Overview

`library` inventories ebooks you own across storefronts (Humble Bundle, Packt,
Manning, Kindle) plus manual entries, in a local SQLite database, so you can check
before buying a book you already have. It's a personal tool, not a hosted service:
everything runs and stores data locally, on the machine it's run on.

Not goals: multi-user accounts, a hosted/synced backend, or scraping any site in a
way that requires working around anti-bot defenses (see Kindle's design note below).

## Boundaries

Domain logic (parsing, dedup matching, storage) lives entirely in `library-core` and
is free of any UI concerns. The CLI and the desktop app are both thin shells around
it — neither has its own copy of the matching or storage logic.

| Port | Adapter(s) | Notes |
| ---- | ---------- | ----- |
| `sources::Source` (fetch a source's owned books) | `humble::Humble`, `packt::Packt`, `manning::Manning`, `kindle::Kindle` | Live HTTP fetch (Humble/Packt/Manning, cookie or token supplied by the user) vs. local CSV import (Kindle — see below). All return the same `Vec<Book>`. |
| `Db` (storage) | `rusqlite`-backed SQLite, single file | No trait abstraction here — SQLite is the only storage backend and isn't expected to change. |
| Frontend | `cli` (clap), `desktop` (Tauri + vanilla JS) | Both call directly into `library-core`'s public API (`Db`, `dedup`, `sources`, `config`) — no shared "app service" layer between them, since neither has enough independent logic to justify one yet. |

## Structure

Modular monolith, ports-and-adapters for the parts that actually vary (sources).
Three crates in one Cargo workspace:

- `core/` (`library-core`) — `db`, `dedup`, `config`, `model`, `sources`. All
  business logic; unit-tested independently of any frontend.
- `cli/` (`library`) — clap-based CLI, no logic of its own beyond argument parsing
  and print formatting.
- `desktop/` (`library-desktop`) — Tauri app; `src-tauri/src/commands.rs` is a thin
  `#[tauri::command]` wrapper layer, `frontend/` is static HTML/CSS/JS with no build
  step (uses Tauri's `withGlobalTauri` bridge instead of an npm-installed API
  client, to keep the desktop app buildable with `cargo` alone).

Kindle is intentionally the odd one out: Amazon has no official API, and the
unofficial routes require working around active TLS/anti-bot fingerprinting, which
is evasion tooling rather than a lightweight cookie paste — so `Kindle` is a CSV
importer, not a live fetcher, unlike the other three sources.

## Data flow

`import <source>` (CLI) or the Import tab (desktop) → `Source::fetch()` returns
`Vec<Book>` → each book is checked against every existing book via
`dedup::find_duplicates` (exact ISBN, exact normalized title, then Jaro-Winkler
fuzzy title match) → non-blocking warnings are surfaced for anything at or above the
confidence threshold from a *different* source → `Db::upsert_book` inserts or
updates, keyed on `(source, source_id)` where the source has a stable id, or always
inserts for sources without one (manual entries).

## Key decisions

See [docs/adr/](./docs/adr/) for the record of individual decisions and their
tradeoffs.

## Non-goals

- **Live Kindle scraping** — out of scope; see the Kindle note under Structure.
- **O'Reilly** — their platform is subscription access, not per-book ownership, so
  there's nothing meaningful to dedupe against.
- **Multi-user / hosted deployment** — this is a local, single-user tool by design.
