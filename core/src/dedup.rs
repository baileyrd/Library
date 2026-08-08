use regex::Regex;
use serde::Serialize;

use crate::model::Book;

const FUZZY_THRESHOLD: f64 = 0.90;

/// Normalization is a pragmatic sequence of string/regex passes rather than
/// a general NLP pipeline: lowercase, strip a leading article, strip common
/// edition/printing markers and parenthetical years, then collapse
/// punctuation and whitespace. Good enough to match "The Rust Programming
/// Language, 2nd Edition (2023)" against "Rust Programming Language".
pub fn normalize_title(title: &str) -> String {
    let lower = title.to_lowercase();

    let leading_article = Regex::new(r"^(the|a|an)\s+").unwrap();
    let without_article = leading_article.replace(&lower, "");

    let edition_marker = Regex::new(
        r"\b(\d+(st|nd|rd|th)\s+edition|\d+(st|nd|rd|th)\s+ed\.?|revised edition|updated edition|new edition)\b",
    )
    .unwrap();
    let without_edition = edition_marker.replace_all(&without_article, "");

    let parenthetical_year = Regex::new(r"\(\s*(19|20)\d{2}\s*\)").unwrap();
    let without_year = parenthetical_year.replace_all(&without_edition, "");

    let publisher_suffix =
        Regex::new(r"\s*[-:]\s*(o'reilly|packt|manning|apress|no starch press)\s*$").unwrap();
    let without_publisher = publisher_suffix.replace_all(&without_year, "");

    let punctuation = Regex::new(r"[^\w\s]").unwrap();
    let without_punctuation = punctuation.replace_all(&without_publisher, " ");

    let whitespace = Regex::new(r"\s+").unwrap();
    whitespace
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

/// Scores `candidate` against every book in `existing`, in three passes:
/// exact ISBN match, exact normalized-title match, then Jaro-Winkler
/// similarity on normalized titles (kept only when >= `min_confidence`).
/// Results are sorted by confidence descending.
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

        if let (Some(a), Some(b)) = (&candidate_isbn, &book_isbn) {
            if !a.is_empty() && a == b {
                matches.push(DedupMatch {
                    book: book.clone(),
                    confidence: 1.0,
                    reason: "ISBN match".to_string(),
                });
                continue;
            }
        }

        if !candidate_normalized_title.is_empty()
            && candidate_normalized_title == book_normalized_title
        {
            matches.push(DedupMatch {
                book: book.clone(),
                confidence: 0.95,
                reason: "exact title match".to_string(),
            });
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Source;

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
}
