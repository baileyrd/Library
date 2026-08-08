//! Packt source.
//!
//! Based on historical-but-corroborated third-party research, not official
//! Packt documentation — the exact endpoint/response shape may have drifted
//! since. Auth is a JWT the user pastes into config themselves (obtained
//! from their own browser session's devtools); this tool does not perform
//! Packt's username/password login flow, since that flow is unverified and
//! out of scope.

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::model::{Book, Source as BookSource};

const PAGE_LIMIT: u32 = 100;

#[derive(Debug, Deserialize)]
struct ProductsResponse {
    count: i64,
    data: Vec<ProductEntry>,
}

#[derive(Debug, Deserialize)]
struct ProductEntry {
    #[serde(rename = "productId")]
    product_id: String,
    #[serde(rename = "productName")]
    product_name: String,
}

pub struct Packt {
    pub token: String,
}

impl super::Source for Packt {
    fn name(&self) -> &'static str {
        "packt"
    }

    fn fetch(&self) -> Result<Vec<Book>> {
        fetch_products(&self.token)
    }
}

pub fn fetch_products(token: &str) -> Result<Vec<Book>> {
    let client = reqwest::blocking::Client::new();
    let mut offset: u32 = 0;
    let mut books = Vec::new();

    loop {
        let url = format!(
            "https://services.packtpub.com/entitlements-v1/users/me/products?sort=createdAt:DESC&offset={offset}&limit={PAGE_LIMIT}"
        );
        let response_body = client
            .get(&url)
            .header("Authorization", format!("Bearer {token}"))
            .header("Accept", "application/json")
            .send()
            .context("failed to fetch Packt products page")?
            .text()
            .context("failed to read Packt products page response body")?;

        let (count, page_books) = parse_products_page(&response_body)?;
        let page_len = page_books.len() as u32;
        books.extend(page_books);

        offset += PAGE_LIMIT;
        if offset as i64 >= count || page_len == 0 {
            break;
        }
    }

    Ok(books)
}

pub fn parse_products_page(json: &str) -> Result<(i64, Vec<Book>)> {
    let response: ProductsResponse =
        serde_json::from_str(json).context("failed to parse Packt products page JSON")?;

    let books = response
        .data
        .into_iter()
        .map(|entry| Book {
            id: None,
            title: entry.product_name,
            // Not available from this endpoint.
            authors: Vec::new(),
            // Not confirmed available on this endpoint; leave unset.
            isbn: None,
            source: BookSource::Packt,
            source_id: Some(entry.product_id),
            // TODO: could be fetched per-book via
            // /products-v1/products/{id}/types, but that's an extra
            // round-trip per book — skipped for now to keep imports fast.
            formats: Vec::new(),
            acquired_at: None,
            raw_json: None,
        })
        .collect();

    Ok((response.count, books))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE_FIXTURE: &str = r#"
    {
        "count": 2,
        "data": [
            { "productId": "abc-123", "productName": "Rust Web Programming" },
            { "productId": "def-456", "productName": "Hands-On Concurrency with Rust" }
        ]
    }
    "#;

    #[test]
    fn parses_products_page() {
        let (count, books) = parse_products_page(PAGE_FIXTURE).unwrap();
        assert_eq!(count, 2);
        assert_eq!(books.len(), 2);
        assert_eq!(books[0].title, "Rust Web Programming");
        assert_eq!(books[0].source_id, Some("abc-123".to_string()));
        assert!(matches!(books[0].source, BookSource::Packt));
        assert!(books[0].authors.is_empty());
        assert!(books[0].isbn.is_none());
        assert!(books[0].formats.is_empty());
    }

    #[test]
    fn empty_page_parses_cleanly() {
        let json = r#"{"count": 0, "data": []}"#;
        let (count, books) = parse_products_page(json).unwrap();
        assert_eq!(count, 0);
        assert!(books.is_empty());
    }
}
