use std::path::Path;

use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension};

use crate::dedup::normalize_title;
use crate::model::{Book, Source};

pub struct Db {
    conn: Connection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpsertOutcome {
    Inserted(i64),
    Updated(i64),
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS books (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    title TEXT NOT NULL,
    normalized_title TEXT NOT NULL,
    authors TEXT,
    isbn TEXT,
    source TEXT NOT NULL,
    source_id TEXT,
    formats TEXT,
    acquired_at TEXT,
    raw_json TEXT,
    created_at TEXT NOT NULL,
    UNIQUE(source, source_id)
);
CREATE INDEX IF NOT EXISTS idx_books_normalized_title ON books(normalized_title);
CREATE INDEX IF NOT EXISTS idx_books_isbn ON books(isbn);
";

impl Db {
    pub fn open(path: &Path) -> Result<Db> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let conn = Connection::open(path)?;
        conn.execute_batch(SCHEMA)?;
        Ok(Db { conn })
    }

    #[cfg(test)]
    pub fn open_in_memory() -> Result<Db> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(SCHEMA)?;
        Ok(Db { conn })
    }

    pub fn upsert_book(&self, book: &Book) -> Result<UpsertOutcome> {
        let now = chrono::Utc::now().to_rfc3339();
        let normalized = normalize_title(&book.title);
        let authors = book.authors.join(", ");
        let formats = book.formats.join(",");
        let acquired_at = book.acquired_at.map(|d| d.to_string());
        let source = book.source.as_str();

        match &book.source_id {
            Some(source_id) => {
                let existing_id: Option<i64> = self
                    .conn
                    .query_row(
                        "SELECT id FROM books WHERE source = ?1 AND source_id = ?2",
                        params![source, source_id],
                        |row| row.get(0),
                    )
                    .optional()?;

                self.conn.execute(
                    "INSERT INTO books
                        (title, normalized_title, authors, isbn, source, source_id, formats, acquired_at, raw_json, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                     ON CONFLICT(source, source_id) DO UPDATE SET
                        title = excluded.title,
                        normalized_title = excluded.normalized_title,
                        authors = excluded.authors,
                        isbn = excluded.isbn,
                        formats = excluded.formats,
                        acquired_at = excluded.acquired_at,
                        raw_json = excluded.raw_json",
                    params![
                        book.title,
                        normalized,
                        authors,
                        book.isbn,
                        source,
                        source_id,
                        formats,
                        acquired_at,
                        book.raw_json,
                        now,
                    ],
                )?;

                let id: i64 = self.conn.query_row(
                    "SELECT id FROM books WHERE source = ?1 AND source_id = ?2",
                    params![source, source_id],
                    |row| row.get(0),
                )?;

                match existing_id {
                    Some(_) => Ok(UpsertOutcome::Updated(id)),
                    None => Ok(UpsertOutcome::Inserted(id)),
                }
            }
            // NULL source_id values never collide under SQLite's UNIQUE
            // semantics (each NULL is distinct), so manual entries always insert.
            None => {
                self.conn.execute(
                    "INSERT INTO books
                        (title, normalized_title, authors, isbn, source, source_id, formats, acquired_at, raw_json, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, NULL, ?6, ?7, ?8, ?9)",
                    params![
                        book.title,
                        normalized,
                        authors,
                        book.isbn,
                        source,
                        formats,
                        acquired_at,
                        book.raw_json,
                        now,
                    ],
                )?;
                Ok(UpsertOutcome::Inserted(self.conn.last_insert_rowid()))
            }
        }
    }

    pub fn list_books(&self, source_filter: Option<Source>) -> Result<Vec<Book>> {
        match source_filter {
            Some(source) => {
                let mut stmt = self.conn.prepare(
                    "SELECT id, title, authors, isbn, source, source_id, formats, acquired_at, raw_json
                     FROM books WHERE source = ?1 ORDER BY title",
                )?;
                let rows = stmt.query_map(params![source.as_str()], row_to_book)?;
                rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
            }
            None => self.all_books(),
        }
    }

    pub fn all_books(&self) -> Result<Vec<Book>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, title, authors, isbn, source, source_id, formats, acquired_at, raw_json
             FROM books ORDER BY title",
        )?;
        let rows = stmt.query_map([], row_to_book)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
    }

    pub fn get_book(&self, id: i64) -> Result<Option<Book>> {
        self.conn
            .query_row(
                "SELECT id, title, authors, isbn, source, source_id, formats, acquired_at, raw_json
                 FROM books WHERE id = ?1",
                params![id],
                row_to_book,
            )
            .optional()
            .map_err(Into::into)
    }

    /// Updates title/authors/isbn/formats in place, keeping the row's id,
    /// source, source_id, acquired_at and raw_json untouched -- used by the
    /// desktop app's edit form, where re-running the source/source_id-keyed
    /// `upsert_book` logic would be the wrong tool (edits aren't re-imports).
    pub fn update_book(
        &self,
        id: i64,
        title: &str,
        authors: &[String],
        isbn: Option<&str>,
        formats: &[String],
    ) -> Result<bool> {
        let normalized = normalize_title(title);
        let authors_joined = authors.join(", ");
        let formats_joined = formats.join(",");
        let affected = self.conn.execute(
            "UPDATE books SET title = ?1, normalized_title = ?2, authors = ?3, isbn = ?4, formats = ?5
             WHERE id = ?6",
            params![title, normalized, authors_joined, isbn, formats_joined, id],
        )?;
        Ok(affected > 0)
    }

    pub fn delete_book(&self, id: i64) -> Result<bool> {
        let affected = self.conn.execute("DELETE FROM books WHERE id = ?1", params![id])?;
        Ok(affected > 0)
    }

    pub fn stats(&self) -> Result<Vec<(Source, i64)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT source, COUNT(*) FROM books GROUP BY source ORDER BY source")?;
        let rows = stmt.query_map([], |row| {
            let source_str: String = row.get(0)?;
            let count: i64 = row.get(1)?;
            Ok((source_str, count))
        })?;

        let mut result = Vec::new();
        for row in rows {
            let (source_str, count) = row?;
            let source: Source = source_str.parse().map_err(|e: anyhow::Error| {
                rusqlite::Error::InvalidColumnType(
                    0,
                    e.to_string(),
                    rusqlite::types::Type::Text,
                )
            })?;
            result.push((source, count));
        }
        Ok(result)
    }
}

fn row_to_book(row: &rusqlite::Row) -> rusqlite::Result<Book> {
    let id: i64 = row.get(0)?;
    let title: String = row.get(1)?;
    let authors_raw: Option<String> = row.get(2)?;
    let isbn: Option<String> = row.get(3)?;
    let source_raw: String = row.get(4)?;
    let source_id: Option<String> = row.get(5)?;
    let formats_raw: Option<String> = row.get(6)?;
    let acquired_at_raw: Option<String> = row.get(7)?;
    let raw_json: Option<String> = row.get(8)?;

    let authors = authors_raw
        .filter(|s| !s.is_empty())
        .map(|s| s.split(", ").map(|p| p.to_string()).collect())
        .unwrap_or_default();
    let formats = formats_raw
        .filter(|s| !s.is_empty())
        .map(|s| s.split(',').map(|p| p.to_string()).collect())
        .unwrap_or_default();
    let source: Source = source_raw.parse().map_err(|e: anyhow::Error| {
        rusqlite::Error::InvalidColumnType(4, e.to_string(), rusqlite::types::Type::Text)
    })?;
    let acquired_at = acquired_at_raw.and_then(|s| chrono::NaiveDate::parse_from_str(&s, "%Y-%m-%d").ok());

    Ok(Book {
        id: Some(id),
        title,
        authors,
        isbn,
        source,
        source_id,
        formats,
        acquired_at,
        raw_json,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Source;

    fn sample_book(title: &str, source: Source, source_id: Option<&str>) -> Book {
        Book {
            id: None,
            title: title.to_string(),
            authors: vec!["Jane Doe".to_string()],
            isbn: Some("9781617294554".to_string()),
            source,
            source_id: source_id.map(|s| s.to_string()),
            formats: vec!["epub".to_string(), "pdf".to_string()],
            acquired_at: None,
            raw_json: None,
        }
    }

    #[test]
    fn open_creates_parent_dirs_and_schema() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("library.db");
        let db = Db::open(&path).unwrap();
        assert!(path.exists());
        assert!(db.all_books().unwrap().is_empty());
    }

    #[test]
    fn upsert_inserts_then_updates_on_same_source_id() {
        let db = Db::open_in_memory().unwrap();
        let book = sample_book("Rust in Action", Source::HumbleBundle, Some("rust-in-action"));

        let outcome = db.upsert_book(&book).unwrap();
        assert!(matches!(outcome, UpsertOutcome::Inserted(_)));

        let mut updated = book.clone();
        updated.title = "Rust in Action (Updated)".to_string();
        let outcome2 = db.upsert_book(&updated).unwrap();
        assert!(matches!(outcome2, UpsertOutcome::Updated(_)));

        let all = db.all_books().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].title, "Rust in Action (Updated)");
    }

    #[test]
    fn upsert_with_no_source_id_always_inserts() {
        let db = Db::open_in_memory().unwrap();
        let book = sample_book("Manual Entry", Source::Manual, None);
        db.upsert_book(&book).unwrap();
        db.upsert_book(&book).unwrap();
        let all = db.all_books().unwrap();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn list_books_filters_by_source() {
        let db = Db::open_in_memory().unwrap();
        db.upsert_book(&sample_book("Book A", Source::HumbleBundle, Some("a")))
            .unwrap();
        db.upsert_book(&sample_book("Book B", Source::Packt, Some("b")))
            .unwrap();

        let humble_only = db.list_books(Some(Source::HumbleBundle)).unwrap();
        assert_eq!(humble_only.len(), 1);
        assert_eq!(humble_only[0].title, "Book A");

        let all = db.list_books(None).unwrap();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn delete_book_removes_row() {
        let db = Db::open_in_memory().unwrap();
        db.upsert_book(&sample_book("Book A", Source::HumbleBundle, Some("a")))
            .unwrap();
        let id = db.all_books().unwrap()[0].id.unwrap();
        assert!(db.delete_book(id).unwrap());
        assert!(!db.delete_book(id).unwrap());
        assert!(db.all_books().unwrap().is_empty());
    }

    #[test]
    fn stats_counts_per_source() {
        let db = Db::open_in_memory().unwrap();
        db.upsert_book(&sample_book("Book A", Source::HumbleBundle, Some("a")))
            .unwrap();
        db.upsert_book(&sample_book("Book B", Source::HumbleBundle, Some("b")))
            .unwrap();
        db.upsert_book(&sample_book("Book C", Source::Packt, Some("c")))
            .unwrap();

        let stats = db.stats().unwrap();
        assert_eq!(stats.len(), 2);
        let humble_count = stats.iter().find(|(s, _)| *s == Source::HumbleBundle).unwrap().1;
        assert_eq!(humble_count, 2);
    }

    #[test]
    fn get_book_returns_none_for_missing_id() {
        let db = Db::open_in_memory().unwrap();
        assert!(db.get_book(999).unwrap().is_none());
    }

    #[test]
    fn update_book_changes_fields_and_keeps_source_id() {
        let db = Db::open_in_memory().unwrap();
        db.upsert_book(&sample_book("Book A", Source::HumbleBundle, Some("a")))
            .unwrap();
        let id = db.all_books().unwrap()[0].id.unwrap();

        let changed = db
            .update_book(
                id,
                "Book A (Revised)",
                &["New Author".to_string()],
                Some("9780000000000"),
                &["mobi".to_string()],
            )
            .unwrap();
        assert!(changed);

        let updated = db.get_book(id).unwrap().unwrap();
        assert_eq!(updated.title, "Book A (Revised)");
        assert_eq!(updated.authors, vec!["New Author".to_string()]);
        assert_eq!(updated.isbn, Some("9780000000000".to_string()));
        assert_eq!(updated.formats, vec!["mobi".to_string()]);
        assert_eq!(updated.source, Source::HumbleBundle);
        assert_eq!(updated.source_id, Some("a".to_string()));
    }

    #[test]
    fn update_book_returns_false_for_missing_id() {
        let db = Db::open_in_memory().unwrap();
        assert!(!db.update_book(999, "X", &[], None, &[]).unwrap());
    }

    #[test]
    fn authors_and_formats_round_trip() {
        let db = Db::open_in_memory().unwrap();
        db.upsert_book(&sample_book("Book A", Source::HumbleBundle, Some("a")))
            .unwrap();
        let all = db.all_books().unwrap();
        assert_eq!(all[0].authors, vec!["Jane Doe".to_string()]);
        assert_eq!(all[0].formats, vec!["epub".to_string(), "pdf".to_string()]);
    }
}
