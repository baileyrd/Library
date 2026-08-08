//! Kindle source.
//!
//! Amazon has no official API for listing owned Kindle books, and the
//! unofficial routes community tools use against `read.amazon.com` require
//! working around Amazon's TLS/JA3 anti-bot fingerprinting -- that's evasion
//! infrastructure, not a lightweight cookie paste, so it's out of scope here.
//! Instead this reads a CSV file the user prepares themselves, e.g. from
//! Amazon's official "Request My Data" export (Amazon Help -> Request Your
//! Information) or hand-typed from the "Manage Your Content and Devices"
//! page. Expected columns: `title,authors,isbn,formats`, where `authors` and
//! `formats` are `;`-separated. Only `title` is required.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::dedup::normalize_title;
use crate::model::{Book, Source as BookSource};

#[derive(Debug, Deserialize)]
struct Record {
    title: String,
    #[serde(default)]
    authors: String,
    #[serde(default)]
    isbn: String,
    #[serde(default)]
    formats: String,
}

pub struct Kindle {
    pub path: PathBuf,
}

impl super::Source for Kindle {
    fn name(&self) -> &'static str {
        "kindle"
    }

    fn fetch(&self) -> Result<Vec<Book>> {
        parse_csv_file(&self.path)
    }
}

fn parse_csv_file(path: &Path) -> Result<Vec<Book>> {
    let contents = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read Kindle CSV at {}", path.display()))?;
    parse_csv(&contents)
}

pub fn parse_csv(csv_contents: &str) -> Result<Vec<Book>> {
    let mut reader = csv::Reader::from_reader(csv_contents.as_bytes());
    let mut books = Vec::new();

    for result in reader.deserialize() {
        let record: Record = result.context("failed to parse Kindle CSV row")?;

        let authors: Vec<String> = record
            .authors
            .split(';')
            .map(|a| a.trim().to_string())
            .filter(|a| !a.is_empty())
            .collect();
        let formats: Vec<String> = record
            .formats
            .split(';')
            .map(|f| f.trim().to_lowercase())
            .filter(|f| !f.is_empty())
            .collect();
        let isbn = if record.isbn.trim().is_empty() {
            None
        } else {
            Some(record.isbn.trim().to_string())
        };

        // Keyed on ISBN when available, else the normalized title, so
        // re-importing the same CSV updates existing rows instead of
        // duplicating them (unlike the live sources, there's no stable
        // per-item id to key on here).
        let source_id = isbn.clone().unwrap_or_else(|| normalize_title(&record.title));

        books.push(Book {
            id: None,
            title: record.title,
            authors,
            isbn,
            source: BookSource::Kindle,
            source_id: Some(source_id),
            formats,
            acquired_at: None,
            raw_json: None,
        });
    }

    Ok(books)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rows_with_authors_and_formats() {
        let csv = "title,authors,isbn,formats\nProgramming Rust,Jim Blandy;Jason Orendorff,9781492052548,epub;mobi\n";
        let books = parse_csv(csv).unwrap();
        assert_eq!(books.len(), 1);
        assert_eq!(books[0].title, "Programming Rust");
        assert_eq!(books[0].authors, vec!["Jim Blandy".to_string(), "Jason Orendorff".to_string()]);
        assert_eq!(books[0].formats, vec!["epub".to_string(), "mobi".to_string()]);
        assert_eq!(books[0].isbn, Some("9781492052548".to_string()));
        assert_eq!(books[0].source_id, Some("9781492052548".to_string()));
    }

    #[test]
    fn falls_back_to_normalized_title_as_source_id_without_isbn() {
        let csv = "title,authors,isbn,formats\nRust in Action,,,\n";
        let books = parse_csv(csv).unwrap();
        assert_eq!(books.len(), 1);
        assert!(books[0].isbn.is_none());
        assert!(books[0].authors.is_empty());
        assert_eq!(books[0].source_id, Some(normalize_title("Rust in Action")));
    }

    #[test]
    fn missing_optional_columns_are_allowed() {
        let csv = "title\nMinimal Book\n";
        let books = parse_csv(csv).unwrap();
        assert_eq!(books.len(), 1);
        assert_eq!(books[0].title, "Minimal Book");
    }
}
