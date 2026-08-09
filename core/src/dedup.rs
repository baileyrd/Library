use std::collections::HashMap;
use std::sync::LazyLock;

use regex::Regex;
use serde::Serialize;

use crate::model::{Book, Source};

const FUZZY_THRESHOLD: f64 = 0.90;

// Compiled once and reused -- `normalize_title` is called O(n) times per
// import and O(n^2) times by `cross_source_duplicates` (every book against
// every other), and `Regex::new` is not cheap (parses + builds an NFA per
// call). Recompiling five patterns per call turned a ~750-book library
// into a multi-minute hang instead of a sub-second comparison.
static LEADING_ARTICLE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(the|a|an)\s+").unwrap());
static EDITION_MARKER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"\b(\d+(st|nd|rd|th)\s+edition|\d+(st|nd|rd|th)\s+ed\.?|revised edition|updated edition|new edition)\b",
    )
    .unwrap()
});
static PARENTHETICAL_YEAR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\(\s*(19|20)\d{2}\s*\)").unwrap());
static PUBLISHER_SUFFIX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\s*[-:]\s*(o'reilly|packt|manning|apress|no starch press)\s*$").unwrap()
});
static PUNCTUATION: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[^\w\s]").unwrap());
static WHITESPACE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());

/// Normalization is a pragmatic sequence of string/regex passes rather than
/// a general NLP pipeline: lowercase, strip a leading article, strip common
/// edition/printing markers and parenthetical years, then collapse
/// punctuation and whitespace. Good enough to match "The Rust Programming
/// Language, 2nd Edition (2023)" against "Rust Programming Language".
pub fn normalize_title(title: &str) -> String {
    let lower = title.to_lowercase();
    let without_article = LEADING_ARTICLE.replace(&lower, "");
    let without_edition = EDITION_MARKER.replace_all(&without_article, "");
    let without_year = PARENTHETICAL_YEAR.replace_all(&without_edition, "");
    let without_publisher = PUBLISHER_SUFFIX.replace_all(&without_year, "");
    let without_punctuation = PUNCTUATION.replace_all(&without_publisher, " ");
    WHITESPACE
        .replace_all(without_punctuation.trim(), " ")
        .trim()
        .to_string()
}

fn normalize_isbn(isbn: &str) -> String {
    isbn.chars().filter(|c| c.is_ascii_digit()).collect()
}

#[derive(Debug, Clone, Serialize)]
pub struct DedupMatch {
    pub book: Book,
    pub confidence: f64,
    pub reason: String,
}

/// Scores `candidate` against every book in `existing`, in tiers: exact
/// ISBN match, exact normalized-title match, then Jaro-Winkler similarity
/// on normalized titles (kept only when >= `min_confidence`) -- but only
/// when the fuzzy pass isn't already overruled by ISBN evidence (see
/// below). Results are sorted by confidence descending.
pub fn find_duplicates_with_threshold(
    existing: &[Book],
    candidate: &Book,
    min_confidence: f64,
) -> Vec<DedupMatch> {
    let candidate_normalized_title = normalize_title(&candidate.title);
    let candidate_isbn = candidate.isbn.as_deref().map(normalize_isbn);

    let mut matches: Vec<DedupMatch> = Vec::new();

    for book in existing {
        let book_normalized_title = normalize_title(&book.title);
        let book_isbn = book.isbn.as_deref().map(normalize_isbn);

        let isbn_match = matches!(
            (&candidate_isbn, &book_isbn),
            (Some(a), Some(b)) if !a.is_empty() && a == b
        );
        if isbn_match {
            matches.push(DedupMatch {
                book: book.clone(),
                confidence: 1.0,
                reason: "ISBN match".to_string(),
            });
            continue;
        }

        if !candidate_normalized_title.is_empty()
            && candidate_normalized_title == book_normalized_title
        {
            // Still checked unconditionally, even when both sides carry a
            // real (and here, by construction, non-matching) ISBN:
            // different editions of the same title normalize to identical
            // strings by design (`normalize_title` strips edition/printing
            // markers), and that's a title match worth surfacing
            // regardless of each edition's own distinct ISBN.
            matches.push(DedupMatch {
                book: book.clone(),
                confidence: 0.95,
                reason: "exact title match".to_string(),
            });
            continue;
        }

        // Both sides carry a real ISBN and it's confirmed different --
        // authoritative evidence these are different books, which
        // outranks a merely-approximate title score. Gates only the fuzzy
        // pass below (the exact-title pass above already ran unaffected):
        // this is what stops "Mastering Palo Alto Networks" from fuzzy-
        // matching "Mastering Bootstrap 4" just because Jaro-Winkler
        // rewards their shared "Mastering" prefix, once both have their
        // own distinct, known ISBN.
        let isbns_confirmed_different = matches!(
            (&candidate_isbn, &book_isbn),
            (Some(a), Some(b)) if !a.is_empty() && !b.is_empty() && a != b
        );
        if isbns_confirmed_different {
            continue;
        }

        let score = strsim::jaro_winkler(&candidate_normalized_title, &book_normalized_title);
        if score >= min_confidence {
            matches.push(DedupMatch {
                book: book.clone(),
                confidence: score,
                reason: format!("similar title ({score:.2})"),
            });
        }
    }

    matches.sort_by(|a, b| b.confidence.partial_cmp(&a.confidence).unwrap());
    matches
}

pub fn find_duplicates(existing: &[Book], candidate: &Book) -> Vec<DedupMatch> {
    find_duplicates_with_threshold(existing, candidate, FUZZY_THRESHOLD)
}

/// For every book with a persisted id, finds which *other* sources also
/// appear to own the same title -- the same ISBN/exact-title/fuzzy-title
/// signal `find_duplicates` uses at import time, computed once across the
/// whole library instead of transiently per import. Import only surfaces a
/// cross-source duplicate once, in a console line or toast that's gone the
/// moment it scrolls off; this lets `list` show the same signal
/// persistently instead of a duplicate purchase silently vanishing from
/// view the moment the import finishes.
pub fn cross_source_duplicates(books: &[Book]) -> HashMap<i64, Vec<Source>> {
    // Precompute each book's normalized title/ISBN exactly once -- calling
    // `find_duplicates` per book (as an earlier version did) re-normalizes
    // every *other* book's title from scratch on every outer iteration,
    // which is O(n^2) normalization work (regex passes + string
    // allocation) on top of an already O(n^2) comparison count. Fine for
    // one candidate against a library at import time; not for every book
    // against every book on every `list` -- that's what turned a ~750-book
    // library into a multi-minute hang.
    struct Entry {
        id: i64,
        source: Source,
        normalized_title: String,
        normalized_isbn: Option<String>,
    }
    let entries: Vec<Entry> = books
        .iter()
        .filter_map(|b| {
            Some(Entry {
                id: b.id?,
                source: b.source,
                normalized_title: normalize_title(&b.title),
                normalized_isbn: b.isbn.as_deref().map(normalize_isbn),
            })
        })
        .collect();

    let mut result: HashMap<i64, Vec<Source>> = HashMap::new();
    for (i, a) in entries.iter().enumerate() {
        for b in &entries[i + 1..] {
            if a.source == b.source {
                continue;
            }
            let isbn_match = matches!(
                (&a.normalized_isbn, &b.normalized_isbn),
                (Some(x), Some(y)) if !x.is_empty() && x == y
            );
            let exact_title_match =
                !a.normalized_title.is_empty() && a.normalized_title == b.normalized_title;
            // Same tiering as `find_duplicates_with_threshold`: a
            // confirmed ISBN mismatch on both sides overrules an
            // approximate title score, but not an exact-normalized-title
            // match (different editions of the same title, which share a
            // normalized form by design regardless of each edition's own
            // ISBN).
            let isbns_confirmed_different = matches!(
                (&a.normalized_isbn, &b.normalized_isbn),
                (Some(x), Some(y)) if !x.is_empty() && !y.is_empty() && x != y
            );
            let is_match = isbn_match
                || exact_title_match
                || (!isbns_confirmed_different
                    && strsim::jaro_winkler(&a.normalized_title, &b.normalized_title)
                        >= FUZZY_THRESHOLD);
            if is_match {
                result.entry(a.id).or_default().push(b.source);
                result.entry(b.id).or_default().push(a.source);
            }
        }
    }

    for sources in result.values_mut() {
        sources.sort_by_key(Source::as_str);
        sources.dedup_by_key(|s| s.as_str());
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn book(title: &str, isbn: Option<&str>, source: Source) -> Book {
        Book {
            id: Some(1),
            title: title.to_string(),
            authors: vec![],
            isbn: isbn.map(|s| s.to_string()),
            source,
            source_id: None,
            formats: vec![],
            acquired_at: None,
            raw_json: None,
            cover_url: None,
        }
    }

    #[test]
    fn normalize_strips_article_edition_and_year() {
        assert_eq!(
            normalize_title("The Rust Programming Language, 2nd Edition (2023)"),
            normalize_title("Rust Programming Language")
        );
    }

    #[test]
    fn normalize_strips_ordinal_edition_variants() {
        assert_eq!(
            normalize_title("Programming Rust, 3rd ed."),
            normalize_title("Programming Rust")
        );
    }

    #[test]
    fn normalize_lowercases_and_collapses_punctuation() {
        assert_eq!(normalize_title("Rust: In Action!!"), "rust in action");
    }

    #[test]
    fn normalize_strips_publisher_suffix() {
        assert_eq!(
            normalize_title("Zero To Production - Manning"),
            normalize_title("Zero To Production")
        );
    }

    #[test]
    fn find_duplicates_isbn_exact_match() {
        let existing = vec![book(
            "Rust in Action",
            Some("978-1-6172-9455-4"),
            Source::HumbleBundle,
        )];
        let candidate = book(
            "A Completely Different Title",
            Some("9781617294554"),
            Source::Packt,
        );
        let matches = find_duplicates(&existing, &candidate);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].confidence, 1.0);
        assert_eq!(matches[0].reason, "ISBN match");
    }

    #[test]
    fn find_duplicates_exact_title_match() {
        let existing = vec![book(
            "The Rust Programming Language",
            None,
            Source::HumbleBundle,
        )];
        let candidate = book("Rust Programming Language", None, Source::Packt);
        let matches = find_duplicates(&existing, &candidate);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].confidence, 0.95);
        assert_eq!(matches[0].reason, "exact title match");
    }

    #[test]
    fn find_duplicates_fuzzy_match_above_threshold() {
        let existing = vec![book("Programming Rust", None, Source::HumbleBundle)];
        let candidate = book("Programming Rus", None, Source::Packt);
        let matches = find_duplicates(&existing, &candidate);
        assert_eq!(matches.len(), 1);
        assert!(matches[0].confidence >= 0.90);
        assert!(matches[0].reason.starts_with("similar title"));
    }

    #[test]
    fn find_duplicates_below_threshold_excluded() {
        let existing = vec![book("Learning Python", None, Source::HumbleBundle)];
        let candidate = book("Rust in Action", None, Source::Packt);
        let matches = find_duplicates(&existing, &candidate);
        assert!(matches.is_empty());
    }

    #[test]
    fn find_duplicates_no_match() {
        let existing: Vec<Book> = vec![];
        let candidate = book("Anything", None, Source::Manual);
        assert!(find_duplicates(&existing, &candidate).is_empty());
    }

    #[test]
    fn find_duplicates_sorted_descending() {
        let existing = vec![
            book("Rust in Action", None, Source::HumbleBundle),
            book("Rust in Action", Some("9781617294554"), Source::Packt),
        ];
        let candidate = book("Rust in Action", Some("9781617294554"), Source::Manual);
        let matches = find_duplicates(&existing, &candidate);
        assert_eq!(matches.len(), 2);
        assert!(matches[0].confidence >= matches[1].confidence);
        assert_eq!(matches[0].reason, "ISBN match");
    }

    #[test]
    fn find_duplicates_confirmed_different_isbn_skips_fuzzy_pass() {
        // Both sides carry a real, different ISBN -- authoritative proof
        // these are different books, even though the titles are similar
        // enough (shared "Mastering..." prefix) that Jaro-Winkler alone
        // would clear a generous threshold.
        let existing = vec![book(
            "Mastering Palo Alto Networks - Third Edition",
            Some("9781800000001"),
            Source::Packt,
        )];
        let candidate = book(
            "Mastering Bootstrap 4 - Second Edition",
            Some("9781800000002"),
            Source::Packt,
        );
        // A low threshold that would otherwise let the fuzzy pass through.
        let matches = find_duplicates_with_threshold(&existing, &candidate, 0.5);
        assert!(matches.is_empty());
    }

    #[test]
    fn find_duplicates_exact_title_still_matches_despite_different_isbn() {
        // Different editions of the same title carry their own distinct
        // ISBNs by design -- the exact-normalized-title pass must still
        // catch this even though the ISBN-mismatch gate below it exists.
        let existing = vec![book(
            "Mastering Python Networking, 3rd Edition",
            Some("9781800000001"),
            Source::Packt,
        )];
        let candidate = book(
            "Mastering Python Networking, 4th Edition",
            Some("9781800000002"),
            Source::HumbleBundle,
        );
        let matches = find_duplicates(&existing, &candidate);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].confidence, 0.95);
        assert_eq!(matches[0].reason, "exact title match");
    }

    #[test]
    fn cross_source_duplicates_confirmed_different_isbn_skips_fuzzy_pass() {
        let books = vec![
            Book {
                id: Some(1),
                ..book(
                    "Mastering Palo Alto Networks - Third Edition",
                    Some("9781800000001"),
                    Source::Packt,
                )
            },
            Book {
                id: Some(2),
                ..book(
                    "Mastering Bootstrap 4 - Second Edition",
                    Some("9781800000002"),
                    Source::HumbleBundle,
                )
            },
        ];
        assert!(cross_source_duplicates(&books).is_empty());
    }

    #[test]
    fn cross_source_duplicates_flags_matches_across_sources_only() {
        let books = vec![
            Book {
                id: Some(1),
                source: Source::Packt,
                ..book("Rust Web Programming", None, Source::Packt)
            },
            Book {
                id: Some(2),
                source: Source::HumbleBundle,
                ..book("Rust Web Programming", None, Source::HumbleBundle)
            },
            Book {
                id: Some(3),
                ..book("Completely Unrelated Title", None, Source::Manning)
            },
        ];
        let dupes = cross_source_duplicates(&books);
        assert_eq!(dupes.get(&1), Some(&vec![Source::HumbleBundle]));
        assert_eq!(dupes.get(&2), Some(&vec![Source::Packt]));
        assert_eq!(dupes.get(&3), None);
    }

    #[test]
    fn cross_source_duplicates_ignores_same_source_matches() {
        let books = vec![
            Book {
                id: Some(1),
                ..book("Programming Rust", None, Source::Manual)
            },
            Book {
                id: Some(2),
                ..book("Programming Rust", None, Source::Manual)
            },
        ];
        assert!(cross_source_duplicates(&books).is_empty());
    }

    #[test]
    fn cross_source_duplicates_ignores_books_without_an_id() {
        let books = vec![
            Book {
                id: None,
                ..book("Programming Rust", None, Source::Packt)
            },
            Book {
                id: Some(2),
                ..book("Programming Rust", None, Source::HumbleBundle)
            },
        ];
        // A book with no persisted id can't be keyed in the result map, so
        // it's excluded from comparisons entirely rather than only as a
        // key -- it can't be used as evidence to flag the other side
        // either. Doesn't occur with real `all_books()` results (every
        // persisted row has an id); this only guards the function against
        // a `None` id, e.g. an in-memory candidate, sneaking into the
        // input.
        assert!(cross_source_duplicates(&books).is_empty());
    }
}
