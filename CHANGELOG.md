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
- Desktop app: "Sign in automatically" per source in Settings — opens an
  embedded login window on the source's own site and captures the resulting
  session/token straight into config once you log in, instead of requiring
  devtools cookie-hunting (`core/src/sources/capture.rs`,
  `desktop/src-tauri/src/commands.rs::capture_credential`). Manual paste
  remains available as a fallback.
- `dedup::cross_source_duplicates`: cross-source duplicates (e.g. a title
  owned via both Humble Bundle and Packt) are now flagged persistently in
  `library list` (CLI, both table and `--json` output) and the desktop
  Books tab, instead of only appearing once as a transient warning during
  `import`/`add` that's gone the moment it scrolls off.
- Book cover thumbnails: `Book.cover_url`, populated from Packt's
  `simplifiedProduct.smallImage`/`coverImage` (Humble Bundle, Manning, and
  Kindle have no confirmed image source, so stay unset), plus an optional
  `--cover-url`/"Cover URL" field for manual entries. Rendered as a small
  thumbnail column in the desktop Books tab.
- Desktop app: "Check a Humble Bundle" — paste a bundle page URL and every
  ebook in it is checked against the library at once (`library check-bundle
  <url>` on the CLI), using the same dedup logic as a single title/ISBN
  check. No login needed; bundle contents are a public page, scraped via
  the embedded `webpack-bundle-page-data` JSON
  (`core/src/sources/humble.rs::fetch_bundle_contents`).
- "Check against current bundles" — discovers every bundle currently listed
  on humblebundle.com/books (scraped the same way, via that page's own
  `landingPage-json-data` JSON) and checks all of them in one click
  (`library check-bundles` on the CLI), instead of finding and pasting each
  bundle's URL by hand. One bundle's fetch/parse failure is reported
  inline rather than failing the whole batch
  (`core/src/sources/humble.rs::fetch_all_active_bundles`).
- Desktop app: bundle results now link back to the source -- each
  bundle's own heading opens its humblebundle.com page in the system
  default browser (`open_url` command, backed by the `open` crate; a
  plain `<a href>` would have navigated the app's own window away instead
  of launching a browser), and every matched candidate links to that
  book's detail page.
- Desktop app: a per-book detail page (`get_book_detail` command), reached
  by clicking a title/cover instead of going straight to the edit form --
  large cover, source/ISBN/formats/acquired date, resolved cross-source
  duplicates (same signal as the list view's compact badge, with full
  match detail), and a collapsed raw-source-data panel. Edit/Remove stay
  one click away instead of being folded into it.
- Bundle-name exclude terms (`Config.bundle_exclude_terms`): `check-bundles`
  (CLI) and "Check against current bundles" (desktop) now skip whole
  bundles whose name contains a user-added term (case-insensitive
  substring match, `sources::humble::matches_excluded_bundle`), instead of
  always screening every bundle Humble currently lists under Books --
  handy for recurring non-book bundles that show up there too. Manage the
  list via `library config bundle-exclude-add/-remove/-list` on the CLI, or
  the new "Bundle exclude terms" box in the desktop Settings tab
  (`add_bundle_exclude_term`/`remove_bundle_exclude_term` commands). A
  single explicitly-pasted `check-bundle <url>` is unaffected -- the filter
  only applies to the bulk discovery command.
### Changed
- Packt import now targets `subscription.packtpub.com/api/entitlements`
  with `packt_session`/`XSRF-TOKEN` cookie auth, replacing the old
  `services.packtpub.com` bearer-token scheme that Packt has since retired
  (was returning unconditional HTTP 403/402, not a rate limit). Desktop
  "Sign in automatically" for Packt now captures the cookie jar via the
  same manual-confirmation flow Manning uses, instead of sniffing an
  `Authorization: Bearer` header that Packt's frontend no longer sends.
  `packt_token` config/CLI (`--packt-token`) is renamed to `packt_cookies`
  (`--packt-cookies`).
### Fixed
- Packt fetch errors now include a snippet of the response body instead of
  discarding it, so a non-2xx response is diagnosable instead of just
  guessing "rate limited or invalid token".
- Packt import now populates `isbn` from `simplifiedProduct.isbn13` when
  present and non-empty; was previously always left unset pending live
  confirmation the field existed.
- `dedup::cross_source_duplicates` (and `normalize_title` generally) no
  longer recompile its regexes and re-normalize every other book's title
  on every comparison -- was O(n^2) in normalization work on top of an
  already O(n^2) comparison count, which hung `library list`/the desktop
  Books tab indefinitely on a real ~750-book library instead of the
  sub-second response it should be.
- Desktop Books grid: covers no longer render with `object-fit: cover`,
  which cropped the left/right edges off any cover whose aspect ratio
  wasn't already ~2:3 -- chopped text off titles for Humble Bundle's
  70x70 square subproduct icon in particular. Now `object-fit: contain`,
  and a cover that's natively too small to fill a grid cell without
  visible upscale blur falls back to the readable text-only card instead
  of stretching it.
- Humble Bundle import no longer uses `subproducts[].icon` as
  `cover_url` -- it's a 70x70 UI badge, not book cover art, and looked
  blurry/cropped at any real thumbnail size with no larger image
  available from this API. `enrich::enrich_missing` now also resolves
  covers (by ISBN against Open Library's covers API, or the search
  result's own cover as a fallback) for any book missing one, not just
  authors/ISBN, so Humble Bundle books get real cover art there instead.
- Duplicate-check quality: `CHECK_WEAK_THRESHOLD` (the "possible match,
  review manually" cutoff used by `check`/`check-bundle`/`check-bundles`)
  raised from 0.75 to 0.90. At 0.75, Jaro-Winkler's shared-prefix bonus
  turned generic tech-book title words into mass false positives -- e.g.
  "Mastering Palo Alto Networks" flagged 44 unrelated "Mastering ..."
  Packt titles as possible duplicates, "Learn Ansible" flagged 17
  unrelated "Learning ..." titles. Verified against the real library: zero
  genuine duplicates scored between 0.75 and 0.90 anywhere sampled; every
  real duplicate already scored >= 0.90. `dedup::find_duplicates_with_threshold`
  and `cross_source_duplicates` also gained a tiering rule: when both
  sides carry a real, differing ISBN, that's treated as authoritative
  proof they're different books and skips the fuzzy-title pass entirely
  (but not the exact-normalized-title pass, so same-title different-
  edition matches -- which share a normalized form by design -- still
  match at 0.95).
- Desktop app: "possible match"/"already in your library" entries in
  Check-before-buying and the Humble Bundle checkers now show the matched
  book's own title (and link to its detail page), not just its source and
  confidence -- e.g. "Mastering Bootstrap 4 (Packt, 83% match)" instead of
  an unreadable "Packt, 83% match" repeated for every candidate.
- Desktop app: toggling "Exclude fiction/comics" on the Humble Bundle
  checkers now just re-filters the already-fetched results instead of
  re-running the whole check -- `check_bundle_url`/`check_active_bundles`
  used to take `exclude_fiction` and apply it before scoring, so changing
  the checkbox after a check meant refetching every bundle page (and, for
  "Check against current bundles", every bundle on the site) over the
  network again just to change what's displayed. Each item's
  `is_fiction_or_comic` heuristic is now computed once and shipped with
  the (unfiltered) result, and the frontend caches the last raw response
  to re-filter locally on checkbox change.
### Security

<!-- ## [0.1.0] - YYYY-MM-DD
### Added
- Initial release -->
