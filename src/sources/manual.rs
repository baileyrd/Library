use crate::model::{Book, Source};

pub fn build_manual_book(
    title: String,
    authors: Vec<String>,
    isbn: Option<String>,
    formats: Vec<String>,
) -> Book {
    Book {
        id: None,
        title,
        authors,
        isbn,
        source: Source::Manual,
        source_id: None,
        formats,
        acquired_at: None,
        raw_json: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_manual_entry_with_no_source_id() {
        let book = build_manual_book(
            "My Book".to_string(),
            vec!["Author One".to_string()],
            Some("1234567890".to_string()),
            vec!["pdf".to_string()],
        );
        assert_eq!(book.title, "My Book");
        assert!(matches!(book.source, Source::Manual));
        assert!(book.source_id.is_none());
    }
}
