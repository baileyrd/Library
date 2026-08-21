//! Metadata enrichment for books whose source doesn't expose authors, an
//! ISBN, and/or usable cover art.
//!
//! None of the live sources reliably expose author names, and Humble
//! Bundle/Manning don't expose ISBNs at all (see the README's "Known
//! limitations" section and the source-level comments in `sources::humble`
//! and `sources::packt`) -- those fields come back empty/`None` straight out
//! of `sources::*::fetch`. Humble Bundle's `subproducts[].icon` field is
//! populated but useless as cover art (a 70x70 UI badge, not a book cover --
//! see `sources::humble`), so that source leaves `cover_url` empty too. This
//! module fills the gaps after the fact from Open Library, which needs no
//! API key or account -- appropriate for a local, single-user tool:
//! - Authors/ISBN come from the search API
//!   (<https://openlibrary.org/dev/docs/api/search>), matched by title.
//! - Cover art comes from the covers API
//!   (<https://openlibrary.org/dev/docs/api/covers>), looked up directly by
//!   ISBN when one is known (cheaper and more precise than a title match),
//!   falling back to the search result's own cover only when no ISBN is
//!   available at all.
//!
//! A title-search result is only accepted when its title is a close fuzzy
//! match to the one being searched for (the same threshold `dedup` uses for
//! duplicate detection), since the search endpoint returns its best-effort
//! ranked guess rather than an exact lookup, and a wrong match would
//! silently attach someone else's author/ISBN/cover to a book.

use std::thread;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::db::Db;
use crate::dedup::normalize_title;

const USER_AGENT: &str = "library-inventory/0.1 (personal ebook inventory tool)";
const MATCH_THRESHOLD: f64 = 0.90;
/// Open Library asks API consumers to keep request rates reasonable; this
/// mirrors the pacing already used against Packt's Cloudflare-fronted API
/// (see `sources::packt::PAGE_DELAY`).
const REQUEST_DELAY: Duration = Duration::from_millis(350);

#[derive(Debug, Deserialize, Default)]
struct SearchResponse {
    #[serde(default)]
    docs: Vec<SearchDoc>,
}

#[derive(Debug, Deserialize, Default)]
struct SearchDoc {
    #[serde(default)]
    title: String,
    #[serde(default)]
    author_name: Vec<String>,
    #[serde(default)]
    isbn: Vec<String>,
    /// Open Library's internal cover id for (one edition of) this work.
    /// `lookup_title`'s caller prefers looking a cover up directly by ISBN
    /// instead (see `cover_url_for_isbn`) when one is available -- more
    /// reliable than whichever edition search.json happened to return --
    /// and only falls back to this when no ISBN could be resolved at all.
    #[serde(default)]
    cover_i: Option<u64>,
}

/// What a lookup found for one book -- any field may be empty/`None` if the
/// matched Open Library record had no data for it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FoundMetadata {
    pub authors: Vec<String>,
    pub isbn: Option<String>,
    pub cover_url: Option<String>,
}

fn client() -> Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .user_agent(USER_AGENT)
        .build()
        .context("failed to build HTTP client")
}

/// Looks `title` up against Open Library's search API and returns the
/// best-matching result's authors/ISBN/cover, if any result was a close
/// enough title match. `Ok(None)` covers both "no results" and "results,
/// but none close enough to trust" -- both mean "nothing usable found",
/// not an error.
pub fn lookup_title(title: &str) -> Result<Option<FoundMetadata>> {
    let client = client()?;
    let response = client
        .get("https://openlibrary.org/search.json")
        .query(&[
            ("title", title),
            ("limit", "5"),
            ("fields", "title,author_name,isbn,cover_i"),
        ])
        .send()
        .context("failed to query Open Library")?;
    let status = response.status();
    let body = response
        .text()
        .context("failed to read Open Library response body")?;
    if !status.is_success() {
        bail!("Open Library returned HTTP {status}");
    }
    parse_search_response(&body, title).context("failed to parse Open Library response JSON")
}

/// Looks a cover up directly by ISBN against Open Library's covers API
/// (<https://openlibrary.org/dev/docs/api/covers>) -- no title match needed
/// when an ISBN is already known, so this is cheaper and carries no
/// match-confidence risk, unlike `lookup_title`. `default=false` makes the
/// endpoint 404 rather than serving Open Library's grey "no cover"
/// placeholder, so a real cover can be told apart from a missing one.
pub fn cover_url_for_isbn(isbn: &str) -> Result<Option<String>> {
    let client = client()?;
    let url = format!("https://covers.openlibrary.org/b/isbn/{isbn}-L.jpg");
    let status = client
        .get(&url)
        .query(&[("default", "false")])
        .send()
        .context("failed to query Open Library covers")?
        .status();
    Ok(status.is_success().then_some(url))
}

/// Pure parsing/matching, split out from `lookup_title` for testing without
/// a live network call -- same pattern as `humble::parse_order_response`
/// and `packt::parse_products_page`.
fn parse_search_response(json: &str, query_title: &str) -> Result<Option<FoundMetadata>> {
    let response: SearchResponse = serde_json::from_str(json)?;
    let query_normalized = normalize_title(query_title);

    let mut best_score = 0.0;
    let mut best_doc: Option<&SearchDoc> = None;
    for doc in &response.docs {
        let score = strsim::jaro_winkler(&query_normalized, &normalize_title(&doc.title));
        if score >= MATCH_THRESHOLD && score > best_score {
            best_score = score;
            best_doc = Some(doc);
        }
    }

    let Some(doc) = best_doc else {
        return Ok(None);
    };
    let isbn = best_isbn(&doc.isbn);
    let cover_url = doc
        .cover_i
        .map(|id| format!("https://covers.openlibrary.org/b/id/{id}-L.jpg"));
    if doc.author_name.is_empty() && isbn.is_none() && cover_url.is_none() {
        return Ok(None);
    }
    Ok(Some(FoundMetadata {
        authors: doc.author_name.clone(),
        isbn,
        cover_url,
    }))
}

/// Open Library lists every edition's ISBN under one work; prefer ISBN-13
/// (what the rest of this app otherwise stores -- see Packt's `isbn13`
/// field) over ISBN-10, and the first candidate within a length as a
/// tie-break.
fn best_isbn(candidates: &[String]) -> Option<String> {
    candidates
        .iter()
        .find(|s| is_isbn13(s))
        .or_else(|| candidates.iter().find(|s| is_isbn10(s)))
        .cloned()
}

fn is_isbn13(s: &str) -> bool {
    s.len() == 13 && s.chars().all(|c| c.is_ascii_digit())
}

/// ISBN-10's check digit may be `X` (representing the value 10), so unlike
/// ISBN-13 the last character isn't necessarily a digit.
fn is_isbn10(s: &str) -> bool {
    s.len() == 10
        && s[..9].chars().all(|c| c.is_ascii_digit())
        && matches!(s.as_bytes()[9], b'0'..=b'9' | b'X' | b'x')
}

/// Summary of an `enrich_missing` run.
#[derive(Debug, Clone, Default, Serialize)]
pub struct EnrichSummary {
    /// Books that had a missing author list, ISBN, and/or cover, and so
    /// were looked up.
    pub checked: usize,
    /// Of those, how many got at least one field filled in.
    pub updated: usize,
    /// Of those, how many had no usable Open Library match.
    pub not_found: usize,
    /// Of those, how many the lookup itself failed for (network/parse
    /// error) -- counted separately from `not_found` so a network outage
    /// shows up as errors, not as "this book doesn't exist".
    pub errors: usize,
}

/// Finds every book missing authors, an ISBN, and/or a cover, looks each
/// one up against Open Library, and writes back whatever fields were both
/// missing and found -- existing data is never overwritten. Best-effort:
/// one book's failed/empty lookup doesn't stop the rest.
pub fn enrich_missing(db: &Db) -> Result<EnrichSummary> {
    let books = db.all_books()?;
    let mut summary = EnrichSummary::default();
    let mut requests_made = 0u32;
    let pace = |requests_made: &mut u32| {
        if *requests_made > 0 {
            thread::sleep(REQUEST_DELAY);
        }
        *requests_made += 1;
    };

    for book in books {
        let needs_authors = book.authors.is_empty();
        let needs_isbn = book.isbn.is_none();
        let needs_cover = book.cover_url.is_none();
        if !needs_authors && !needs_isbn && !needs_cover {
            continue;
        }
        let Some(id) = book.id else { continue };
        summary.checked += 1;

        // Authors/ISBN already complete, only the cover is missing: an
        // ISBN is already in hand, so skip the fuzzy title search
        // entirely and go straight to the cheaper, precise ISBN-keyed
        // covers lookup.
        if !needs_authors && !needs_isbn && needs_cover {
            let isbn = book.isbn.as_deref().expect("needs_isbn is false");
            pace(&mut requests_made);
            match cover_url_for_isbn(isbn) {
                Ok(Some(cover_url)) => {
                    db.update_metadata(id, None, None, Some(&cover_url))?;
                    summary.updated += 1;
                }
                Ok(None) => summary.not_found += 1,
                Err(_) => summary.errors += 1,
            }
            continue;
        }

        pace(&mut requests_made);
        let found = match lookup_title(&book.title) {
            Ok(found) => found,
            Err(_) => {
                summary.errors += 1;
                continue;
            }
        };

        let Some(found) = found else {
            summary.not_found += 1;
            continue;
        };

        let authors =
            (needs_authors && !found.authors.is_empty()).then_some(found.authors.as_slice());
        let isbn = if needs_isbn {
            found.isbn.as_deref()
        } else {
            None
        };

        // Prefer a cover looked up directly by ISBN -- more reliable than
        // whichever edition search.json's `cover_i` happened to surface --
        // over the title search's own cover, using an ISBN either already
        // on the book or just resolved above. Only fall back to the
        // title-search cover when no ISBN is available at all.
        let cover_url = if !needs_cover {
            None
        } else if let Some(isbn) = book.isbn.as_deref().or(isbn) {
            pace(&mut requests_made);
            match cover_url_for_isbn(isbn) {
                Ok(by_isbn) => by_isbn.or_else(|| found.cover_url.clone()),
                Err(_) => found.cover_url.clone(),
            }
        } else {
            found.cover_url.clone()
        };

        if authors.is_some() || isbn.is_some() || cover_url.is_some() {
            db.update_metadata(id, authors, isbn, cover_url.as_deref())?;
            summary.updated += 1;
        } else {
            summary.not_found += 1;
        }
    }

    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Book, Source};

    fn book_missing(title: &str) -> Book {
        Book {
            id: None,
            title: title.to_string(),
            authors: Vec::new(),
            isbn: None,
            source: Source::HumbleBundle,
            source_id: Some(title.to_string()),
            formats: Vec::new(),
            acquired_at: None,
            raw_json: None,
            cover_url: None,
        }
    }

    #[test]
    fn parses_matching_result() {
        let json = r#"{"docs":[{"title":"Programming Rust","author_name":["Jim Blandy","Jason Orendorff"],"isbn":["9781492052548","149205254X"]}]}"#;
        let found = parse_search_response(json, "Programming Rust")
            .unwrap()
            .unwrap();
        assert_eq!(found.authors, vec!["Jim Blandy", "Jason Orendorff"]);
        assert_eq!(found.isbn, Some("9781492052548".to_string()));
    }

    #[test]
    fn parses_cover_from_search_result() {
        let json = r#"{"docs":[{"title":"Programming Rust","author_name":["Jim Blandy"],"isbn":["9781492052548"],"cover_i":12345}]}"#;
        let found = parse_search_response(json, "Programming Rust")
            .unwrap()
            .unwrap();
        assert_eq!(
            found.cover_url,
            Some("https://covers.openlibrary.org/b/id/12345-L.jpg".to_string())
        );
    }

    #[test]
    fn missing_cover_i_yields_no_cover_url() {
        let json = r#"{"docs":[{"title":"Programming Rust","author_name":["Jim Blandy"],"isbn":["9781492052548"]}]}"#;
        let found = parse_search_response(json, "Programming Rust")
            .unwrap()
            .unwrap();
        assert_eq!(found.cover_url, None);
    }

    #[test]
    fn rejects_weak_title_match() {
        let json = r#"{"docs":[{"title":"A Completely Different Book","author_name":["Someone"],"isbn":["1234567890123"]}]}"#;
        let found = parse_search_response(json, "Programming Rust").unwrap();
        assert!(found.is_none());
    }

    #[test]
    fn picks_highest_scoring_doc() {
        let json = r#"{"docs":[
            {"title":"Rust Programming","author_name":["Wrong Author"],"isbn":[]},
            {"title":"The Rust Programming Language","author_name":["Steve Klabnik","Carol Nichols"],"isbn":["9781718503106"]}
        ]}"#;
        let found = parse_search_response(json, "The Rust Programming Language")
            .unwrap()
            .unwrap();
        assert_eq!(found.authors, vec!["Steve Klabnik", "Carol Nichols"]);
    }

    #[test]
    fn empty_docs_returns_none() {
        let json = r#"{"docs":[]}"#;
        assert!(parse_search_response(json, "Anything").unwrap().is_none());
    }

    #[test]
    fn prefers_isbn13_over_isbn10() {
        assert_eq!(
            best_isbn(&["149205254X".to_string(), "9781492052548".to_string()]),
            Some("9781492052548".to_string())
        );
    }

    #[test]
    fn falls_back_to_isbn10_when_no_isbn13_present() {
        assert_eq!(
            best_isbn(&["149205254X".to_string()]),
            Some("149205254X".to_string())
        );
    }

    #[test]
    fn enrich_missing_skips_books_with_no_gaps() {
        let db = Db::open_in_memory().unwrap();
        let mut complete = book_missing("Complete Book");
        complete.authors = vec!["Someone".to_string()];
        complete.isbn = Some("9780000000000".to_string());
        complete.cover_url = Some("https://example.com/cover.jpg".to_string());
        db.upsert_book(&complete).unwrap();

        // No network access in tests -- with nothing missing, `enrich_missing`
        // must never attempt a lookup, so this must succeed offline.
        let summary = enrich_missing(&db).unwrap();
        assert_eq!(summary.checked, 0);
        assert_eq!(summary.updated, 0);
    }
}
