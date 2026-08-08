//! Humble Bundle source.
//!
//! Auth is a single session cookie, `_simpleauth_sess`, sent as a raw
//! `Cookie` header (simpler than reqwest's cookie jar for one static value
//! and avoids its cookie-domain matching complexity).
//!
//! This hits the documented-but-unofficial order API. Humble Bundle has
//! reportedly deprecated this in favor of the embedded JSON on the
//! `/home/library` HTML page, so:
//! TODO: add an HTML-scrape fallback against `/home/library` if the
//! `/api/v1/user/order` endpoint stops returning data.

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::model::{Book, Source as BookSource};

const USER_AGENT: &str = "library-inventory/0.1";

#[derive(Debug, Deserialize)]
struct OrderStub {
    gamekey: String,
}

#[derive(Debug, Deserialize)]
struct OrderDetail {
    #[serde(default)]
    subproducts: Vec<Subproduct>,
}

#[derive(Debug, Deserialize)]
struct Subproduct {
    human_name: String,
    machine_name: String,
    #[serde(default)]
    downloads: Vec<Download>,
}

#[derive(Debug, Deserialize)]
struct Download {
    platform: String,
    #[serde(default)]
    download_struct: Vec<DownloadStruct>,
}

#[derive(Debug, Deserialize)]
struct DownloadStruct {
    name: String,
}

pub struct Humble {
    pub cookie: String,
}

impl super::Source for Humble {
    fn name(&self) -> &'static str {
        "humble_bundle"
    }

    fn fetch(&self) -> Result<Vec<Book>> {
        fetch_orders(&self.cookie)
    }
}

fn client() -> Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .user_agent(USER_AGENT)
        .build()
        .context("failed to build HTTP client")
}

pub fn fetch_orders(cookie: &str) -> Result<Vec<Book>> {
    let client = client()?;
    let cookie_header = format!("_simpleauth_sess={cookie}");

    let list_response = client
        .get("https://www.humblebundle.com/api/v1/user/order?ajax=true")
        .header("Cookie", &cookie_header)
        .header("Accept", "application/json")
        .header("X-Requested-By", "hb_android_app")
        .send()
        .context("failed to fetch Humble Bundle order list")?
        .text()
        .context("failed to read Humble Bundle order list response body")?;

    let gamekeys = parse_order_list_response(&list_response);

    let mut books = Vec::new();
    for gamekey in gamekeys {
        let url =
            format!("https://www.humblebundle.com/api/v1/order/{gamekey}?ajax=true&all_tpkds=true");
        let detail_response = client
            .get(&url)
            .header("Cookie", &cookie_header)
            .header("Accept", "application/json")
            .header("X-Requested-By", "hb_android_app")
            .send()
            .with_context(|| format!("failed to fetch Humble Bundle order {gamekey}"))?
            .text()
            .with_context(|| {
                format!("failed to read Humble Bundle order {gamekey} response body")
            })?;

        books.extend(parse_order_response(&detail_response)?);
    }

    Ok(books)
}

/// Returns an empty list rather than erroring on unexpected/empty payloads,
/// since this API is undocumented and may change shape without notice.
fn parse_order_list_response(json: &str) -> Vec<String> {
    serde_json::from_str::<Vec<OrderStub>>(json)
        .map(|stubs| stubs.into_iter().map(|s| s.gamekey).collect())
        .unwrap_or_default()
}

pub fn parse_order_response(json: &str) -> Result<Vec<Book>> {
    let detail: OrderDetail =
        serde_json::from_str(json).context("failed to parse Humble Bundle order detail JSON")?;

    let mut books = Vec::new();
    for subproduct in detail.subproducts {
        let is_ebook = subproduct.downloads.iter().any(|d| d.platform == "ebook");
        if !is_ebook {
            continue;
        }

        let mut formats: Vec<String> = subproduct
            .downloads
            .iter()
            .filter(|d| d.platform == "ebook")
            .flat_map(|d| d.download_struct.iter())
            .map(|ds| normalize_format(&ds.name))
            .collect();
        formats.sort();
        formats.dedup();

        books.push(Book {
            id: None,
            title: subproduct.human_name,
            // Not exposed by this API.
            authors: Vec::new(),
            // Humble Bundle doesn't expose ISBNs.
            isbn: None,
            source: BookSource::HumbleBundle,
            // The subproduct's machine_name, not the order gamekey: one
            // order/bundle can contain many books, so the gamekey doesn't
            // identify an individual book.
            source_id: Some(subproduct.machine_name),
            formats,
            acquired_at: None,
            raw_json: Some(json.to_string()),
        });
    }

    Ok(books)
}

fn normalize_format(name: &str) -> String {
    let lower = name.to_lowercase();
    match lower.find('(') {
        Some(idx) => lower[..idx].trim().to_string(),
        None => lower.trim().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ORDER_FIXTURE: &str = r#"
    {
        "gamekey": "abc123",
        "product": { "human_name": "Rust Books Bundle", "machine_name": "rust_bundle", "category": "bundle" },
        "subproducts": [
            {
                "human_name": "Programming Rust",
                "machine_name": "programming_rust",
                "downloads": [
                    {
                        "platform": "ebook",
                        "download_struct": [
                            { "name": "epub", "url": { "web": "https://dl.humble.com/programming_rust.epub" } },
                            { "name": "pdf (hd)", "url": { "web": "https://dl.humble.com/programming_rust.pdf" } }
                        ]
                    }
                ]
            },
            {
                "human_name": "Some Game Soundtrack",
                "machine_name": "game_soundtrack",
                "downloads": [
                    {
                        "platform": "android",
                        "download_struct": [
                            { "name": "apk", "url": { "web": "https://dl.humble.com/game.apk" } }
                        ]
                    }
                ]
            },
            {
                "human_name": "Rust in Action",
                "machine_name": "rust_in_action",
                "downloads": [
                    {
                        "platform": "ebook",
                        "download_struct": [
                            { "name": "pdf", "url": { "web": "https://dl.humble.com/ria.pdf" } },
                            { "name": "epub", "url": { "web": "https://dl.humble.com/ria.epub" } },
                            { "name": "mobi", "url": { "web": "https://dl.humble.com/ria.mobi" } }
                        ]
                    },
                    {
                        "platform": "android",
                        "download_struct": [
                            { "name": "apk", "url": { "web": "https://dl.humble.com/ria.apk" } }
                        ]
                    }
                ]
            }
        ]
    }
    "#;

    #[test]
    fn parses_ebook_subproducts_and_skips_non_ebooks() {
        let books = parse_order_response(ORDER_FIXTURE).unwrap();
        assert_eq!(books.len(), 2);
        assert!(books.iter().all(|b| b.title != "Some Game Soundtrack"));
    }

    #[test]
    fn normalizes_format_variants() {
        let books = parse_order_response(ORDER_FIXTURE).unwrap();
        let programming_rust = books
            .iter()
            .find(|b| b.title == "Programming Rust")
            .unwrap();
        assert_eq!(
            programming_rust.formats,
            vec!["epub".to_string(), "pdf".to_string()]
        );
    }

    #[test]
    fn uses_machine_name_as_source_id() {
        let books = parse_order_response(ORDER_FIXTURE).unwrap();
        let ria = books.iter().find(|b| b.title == "Rust in Action").unwrap();
        assert_eq!(ria.source_id, Some("rust_in_action".to_string()));
        assert_eq!(
            ria.formats,
            vec!["epub".to_string(), "mobi".to_string(), "pdf".to_string()]
        );
        assert!(ria.isbn.is_none());
        assert!(ria.authors.is_empty());
    }

    #[test]
    fn order_list_parses_gamekeys() {
        let json = r#"[{"gamekey": "abc"}, {"gamekey": "def"}]"#;
        assert_eq!(
            parse_order_list_response(json),
            vec!["abc".to_string(), "def".to_string()]
        );
    }

    #[test]
    fn order_list_handles_unexpected_shape_gracefully() {
        let json = r#"{"unexpected": "shape"}"#;
        assert!(parse_order_list_response(json).is_empty());
    }
}
