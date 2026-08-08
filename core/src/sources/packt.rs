//! Packt source.
//!
//! Based on historical-but-corroborated third-party research, not official
//! Packt documentation — the exact endpoint/response shape may have drifted
//! since. Auth is a JWT the user pastes into config themselves (obtained
//! from their own browser session's devtools); this tool does not perform
//! Packt's username/password login flow, since that flow is unverified and
//! out of scope.

use std::thread;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use serde::Deserialize;

use crate::model::{Book, Source as BookSource};

const PAGE_LIMIT: u32 = 100;
const USER_AGENT: &str = "library-inventory/0.1";
/// Packt's API sits behind Cloudflare, which can bot-challenge back-to-back
/// requests with no pacing between them -- returning an HTML "Just a
/// moment..." challenge page instead of JSON on the second/third page of a
/// large library. This is well short of anything resembling evasion (see
/// the Kindle source and ARCHITECTURE.md's non-goals), just not hammering
/// the endpoint.
const PAGE_DELAY: Duration = Duration::from_millis(400);

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

fn client() -> Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .user_agent(USER_AGENT)
        .build()
        .context("failed to build HTTP client")
}

pub fn fetch_products(token: &str) -> Result<Vec<Book>> {
    let client = client()?;
    let mut offset: u32 = 0;
    let mut books = Vec::new();

    loop {
        if offset > 0 {
            thread::sleep(PAGE_DELAY);
        }

        let url = format!(
            "https://services.packtpub.com/entitlements-v1/users/me/products?sort=createdAt:DESC&offset={offset}&limit={PAGE_LIMIT}"
        );
        let response = client
            .get(&url)
            .header("Authorization", format!("Bearer {token}"))
            .header("Accept", "application/json")
            .send()
            .context("failed to fetch Packt products page")?;
        let status = response.status();
        let response_body = response
            .text()
            .context("failed to read Packt products page response body")?;
        if !status.is_success() {
            bail!(
                "Packt API returned HTTP {status} for offset {offset} instead of a products page \
                 (token may be invalid/expired, or the request was rate-limited/blocked -- try again \
                 in a moment)"
            );
        }

        let (count, page_books) = parse_products_page(&response_body)?;
        let page_len = page_books.len() as u32;
        books.extend(page_books);

        // Advance by however many rows this page actually returned rather
        // than assuming it honored `limit` -- the API has been observed to
        // ignore pagination entirely and return everything on page one, in
        // which case `books.len() >= count` below stops us after a single
        // request instead of looping into needless (and Cloudflare-
        // challenge-risking) follow-up requests.
        if page_len == 0 || books.len() as i64 >= count {
            break;
        }
        offset += page_len;
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
