# library

A personal command-line tool that inventories ebooks you own across Humble
Bundle, Packt, Manning, Kindle (via CSV import), and manually-entered
sources, storing everything in a local SQLite database — so you can check
for duplicates before buying a book you already have.

## Building

```
cargo build --release
```

The binary is written to `target/release/library`.

## Where things live

- Config (cookies/tokens): `$XDG_CONFIG_HOME/library-inventory/config.toml`
  (or the OS equivalent, e.g. `~/.config/library-inventory/config.toml` on
  Linux, `~/Library/Application Support/library-inventory/config.toml` on
  macOS). This file is written with `0600` permissions since it holds session
  cookies/API tokens.
- Database: `$XDG_DATA_HOME/library-inventory/library.db` by default, or the
  path set in config, or a `--db <path>` override on any command.

Both live outside this repository, so there's nothing sensitive to
`.gitignore` inside the project itself — only `/target` is ignored.

## Getting credentials

None of these sites' credentials are things this tool can obtain for you via
username/password login (this is intentionally out of scope). Instead you
copy a cookie or token out of your own already-logged-in browser session.

### Humble Bundle

1. Log into humblebundle.com in your browser.
2. Open devtools -> Application (Chrome) or Storage (Firefox) -> Cookies ->
   `https://www.humblebundle.com`.
3. Find the cookie named `_simpleauth_sess` and copy its value.
4. `library config set --humble-cookie '<value>'`

### Packt

1. Log into subscription.packtpub.com / packtpub.com in your browser.
2. Open devtools -> Network tab, then trigger any request to
   `services.packtpub.com` (e.g. browse your library).
3. Find the `Authorization: Bearer <token>` request header and copy the
   token (the part after `Bearer `).
4. `library config set --packt-token '<value>'`

Note: Packt's API shape here was reverse-engineered from third-party
research, not official documentation, and may need adjustment if Packt
changes their API.

### Manning

1. Log into manning.com in your browser.
2. Open devtools -> Application/Storage -> Cookies, and look at both
   `https://www.manning.com` and `https://login.manning.com`.
3. Copy every `name=value` cookie pair for both domains and join them with
   `; ` into one string (this is exactly what a browser would send in a
   `Cookie` header).
4. `library config set --manning-cookies '<value>'`

Manning has no public API for this, so the tool scrapes the dashboard HTML
page. The selectors/attribute names used were reverse-engineered from a
third-party tool, not official docs, and may need adjustment if Manning
changes their markup.

### Kindle

Amazon has no official API for listing owned Kindle books, and the
unofficial routes require working around Amazon's anti-bot TLS
fingerprinting to even make requests — that's evasion infrastructure, not a
lightweight cookie paste, so this tool doesn't do live Kindle fetching.
Instead, import a CSV file you prepare yourself:

1. Get your book list — either request it via Amazon's official "Request
   Your Information" data export (Amazon Help -> Request Your Information ->
   choose a digital content/orders category), or just note down titles from
   the "Manage Your Content and Devices" page (read.amazon.com/kindle-library).
2. Build a CSV with columns `title,authors,isbn,formats` — only `title` is
   required; `authors` and `formats` are `;`-separated (e.g.
   `Jim Blandy;Jason Orendorff`). See example below.
3. `library import kindle --file mybooks.csv`

```csv
title,authors,isbn,formats
Programming Rust,Jim Blandy;Jason Orendorff,9781492052548,epub;mobi
```

Re-importing the same CSV updates existing rows (keyed on ISBN, or on the
normalized title when no ISBN is given) instead of creating duplicates.

## Usage

```
# Import from a storefront (reads credentials from config)
library import humble
library import packt
library import manning
library import kindle --file mybooks.csv

# Add a book you own but that isn't from one of the above sources
library add --title "Programming Rust" --author "Jim Blandy" --author "Jason Orendorff" \
  --isbn 9781492052548 --format epub --format pdf

# List everything, or filter by source
library list
library list --source humble_bundle
library list --json

# Check before buying
library check "Programming Rust"
library check 9781492052548

# See counts per source
library stats

# Remove an entry
library remove 42
```

Every `import` and `add` prints duplicate warnings (confidence >= 0.90 fuzzy
title match, or ISBN match, or exact-title match) against books already in
your library from a *different* source — these are warnings only, nothing is
ever auto-skipped, since you may legitimately want the same book from two
storefronts (e.g. different formats). `check` additionally shows weaker
matches down to 0.75 confidence under a separate "review manually" section.

Global options: `--db <path>` overrides the database location for a single
invocation; `-v` / `--verbose` prints extra diagnostics to stderr.

## Known limitations / caveats

- **Humble Bundle**: uses the documented (if unofficial and possibly
  deprecated) `/api/v1/user/order` + `/api/v1/order/{gamekey}` endpoints.
  TODO: if Humble Bundle drops this API, fall back to scraping the embedded
  JSON on the `/home/library` HTML page instead.
- **Packt**: entitlements API shape is based on historical-but-corroborated
  third-party research; the exact response shape may have drifted since.
  Formats are not fetched per-book (would require an extra API round-trip
  per book) — left empty for now.
- **Manning**: no public API exists, so this scrapes dashboard HTML with
  selectors reverse-engineered from a third-party tool. Adjust the
  selectors/attributes in `src/sources/manning.rs` if Manning's markup
  changes.
- **Kindle**: no live fetch (see above) — CSV import only, with data quality
  depending entirely on what you put in the file.
- None of the live sources expose authors reliably; Humble Bundle and
  Manning don't expose ISBNs either. These fields are simply left empty/None
  where the source doesn't provide them.
