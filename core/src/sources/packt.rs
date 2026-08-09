//! Packt source.
//!
//! Packt's storefront API has moved at least twice over the years:
//!   - The old scheme (still described in several 2020-era blog posts) hit
//!     `services.packtpub.com/entitlements-v1/users/me/products` with a JWT
//!     in an `Authorization: Bearer` header. That's what this module used to
//!     implement -- it now returns HTTP 403/402 unconditionally, which is
//!     what sent us looking for a replacement rather than a rate-limit fix.
//!   - The current scheme (corroborated by an actively-maintained
//!     community downloader as of January 2026) hits
//!     `subscription.packtpub.com/api/entitlements/users/me/owned` and
//!     authenticates via the same two cookies the site's own frontend uses:
//!     `packt_session` (the session id) and `XSRF-TOKEN` (whose value is
//!     also echoed back in an `X-Xsrf-Token` request header -- a standard
//!     double-submit CSRF pattern). Both travel together in the captured
//!     cookie jar; see `sources::capture::PACKT_CAPTURE`.
//!
//! Not official Packt documentation -- reverse-engineered from third-party
//! research, so the exact endpoint/response shape may drift again. Auth is
//! the user's own browser session (captured via the desktop app's embedded
//! login window, or pasted manually from devtools); this tool does not
//! perform Packt's username/password login flow itself.

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

/// Name of the cookie whose value doubles as the CSRF token Packt's API
/// expects echoed back in the `X-Xsrf-Token` request header. Present (and
/// required) alongside `packt_session` in the captured/pasted cookie jar.
const XSRF_COOKIE_NAME: &str = "XSRF-TOKEN";

#[derive(Debug, Deserialize)]
struct ProductsResponse {
    /// Total owned-product count, when the API includes it. Confirmed
    /// live responses have omitted it (or placed it after a `data` array
    /// too large to see in a truncated diagnostic dump) -- treated as an
    /// optimization only, never required for correct pagination. See the
    /// termination check in `fetch_products`.
    #[serde(default)]
    count: Option<i64>,
    data: Vec<ProductEntry>,
}

#[derive(Debug, Deserialize)]
struct ProductEntry {
    /// Confirmed live (2026-08): a JSON *string*, Packt's own product
    /// identifier (often, but not necessarily always, an ISBN13 -- e.g.
    /// `"9781835466759"`). Distinct from this entry's top-level `id`,
    /// which is an unrelated *integer* entitlement id (e.g.
    /// `43312892`) -- do not alias the two together, a prior version of
    /// this code did and serde rejected the integer where a string was
    /// expected. A genuinely missing `productId` degrades to `None`
    /// rather than failing the whole import.
    #[serde(rename = "productId", default)]
    product_id: Option<String>,
    #[serde(rename = "productName")]
    product_name: String,
    #[serde(rename = "simplifiedProduct", default)]
    simplified_product: Option<SimplifiedProduct>,
}

/// Confirmed live (2026-08): a nested object carrying most of the
/// browsable-catalog metadata (title, cover images, category, ...); this
/// only pulls `isbn13` and the two cover-image URLs -- the fields with an
/// observable use here. `authors` is present too, but only as opaque
/// author-id strings, not names -- resolving those would need a further
/// per-author lookup against an unconfirmed endpoint, so (like per-book
/// formats, see the TODO below) it's left alone rather than guessed at.
#[derive(Debug, Deserialize, Default)]
struct SimplifiedProduct {
    #[serde(default)]
    isbn13: Option<String>,
    #[serde(rename = "smallImage", default)]
    small_image: Option<String>,
    #[serde(rename = "coverImage", default)]
    cover_image: Option<String>,
}

pub struct Packt {
    /// Full cookie jar string (semicolon-separated `name=value` pairs)
    /// from an authenticated subscription.packtpub.com session, same shape
    /// as `Manning::cookies`. Must include `packt_session` and
    /// `XSRF-TOKEN`.
    pub cookies: String,
}

impl super::Source for Packt {
    fn name(&self) -> &'static str {
        "packt"
    }

    fn fetch(&self) -> Result<Vec<Book>> {
        fetch_products(&self.cookies)
    }
}

fn client() -> Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .user_agent(USER_AGENT)
        .build()
        .context("failed to build HTTP client")
}

/// Pulls the `XSRF-TOKEN` cookie's value out of a `name=value; name=value`
/// jar string.
fn xsrf_token(cookies: &str) -> Option<&str> {
    cookies.split(';').find_map(|pair| {
        let (name, value) = pair.trim().split_once('=')?;
        (name == XSRF_COOKIE_NAME).then_some(value)
    })
}

/// Truncates a response body to at most `max_chars` characters (UTF-8
/// boundary safe) for inclusion in error messages -- long enough to show a
/// meaningful error page/JSON snippet, short enough to keep the error
/// readable.
fn truncate_body(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max_chars).collect();
    out.push_str("...");
    out
}

pub fn fetch_products(cookies: &str) -> Result<Vec<Book>> {
    let xsrf = xsrf_token(cookies)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Packt cookie jar is missing the '{XSRF_COOKIE_NAME}' cookie -- make sure the \
                 captured/pasted value includes it alongside 'packt_session' (re-run \"Sign in \
                 automatically\", or re-copy the full cookie jar from devtools)"
            )
        })?
        .to_string();

    let client = client()?;
    let mut offset: u32 = 0;
    let mut books = Vec::new();

    loop {
        if offset > 0 {
            thread::sleep(PAGE_DELAY);
        }

        let url = format!(
            "https://subscription.packtpub.com/api/entitlements/users/me/owned?sort=createdAt:desc&search=&offset={offset}&limit={PAGE_LIMIT}"
        );
        let response = client
            .get(&url)
            .header("Cookie", cookies)
            .header("X-Xsrf-Token", &xsrf)
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
                 (cookie jar may be invalid/expired, or the request was rate-limited/blocked -- \
                 try re-capturing your session). Response body: {}",
                truncate_body(&response_body, 500)
            );
        }

        let (count, page_books) = parse_products_page(&response_body).with_context(|| {
            format!("response body was: {}", truncate_body(&response_body, 2000))
        })?;
        let page_len = page_books.len() as u32;
        books.extend(page_books);

        // A short (or empty) page is the reliable "no more pages" signal
        // -- it holds whether or not `count` was even present in this
        // response, and doesn't depend on trusting a total that (per the
        // Cloudflare-avoidance note above) the API has been observed to
        // report inconsistently with what pagination actually returns.
        // `count`, when present, only lets us skip one redundant final
        // request a page early.
        if page_len == 0 || page_len < PAGE_LIMIT || count.is_some_and(|c| books.len() as i64 >= c)
        {
            break;
        }
        offset += page_len;
    }

    Ok(books)
}

pub fn parse_products_page(json: &str) -> Result<(Option<i64>, Vec<Book>)> {
    let response: ProductsResponse =
        serde_json::from_str(json).context("failed to parse Packt products page JSON")?;

    let books = response
        .data
        .into_iter()
        .map(|entry| {
            let isbn = entry
                .simplified_product
                .as_ref()
                .and_then(|sp| sp.isbn13.as_deref())
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            // Prefer the full-size `coverImage` over the pre-scaled
            // `smallImage` thumbnail: the desktop app's grid view renders
            // covers well above list-row thumbnail size, and upscaling a
            // small source image looks visibly blurry there, whereas a
            // full-size image downscales cleanly for the list view's
            // 36x48 thumbnail. Fall back to `smallImage` only when Packt
            // doesn't provide a full-size image at all.
            let cover_url = entry
                .simplified_product
                .as_ref()
                .and_then(|sp| sp.cover_image.as_deref().or(sp.small_image.as_deref()))
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            Book {
                id: None,
                title: entry.product_name,
                // Not available from this endpoint (author ids only, see
                // `SimplifiedProduct`'s doc comment).
                authors: Vec::new(),
                isbn,
                source: BookSource::Packt,
                source_id: entry.product_id,
                // TODO: could be fetched per-book via
                // /products-v1/products/{id}/types, but that's an extra
                // round-trip per book — skipped for now to keep imports fast.
                formats: Vec::new(),
                acquired_at: None,
                raw_json: None,
                cover_url,
            }
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
        assert_eq!(count, Some(2));
        assert_eq!(books.len(), 2);
        assert_eq!(books[0].title, "Rust Web Programming");
        assert_eq!(books[0].source_id, Some("abc-123".to_string()));
        assert!(matches!(books[0].source, BookSource::Packt));
        assert!(books[0].authors.is_empty());
        assert!(books[0].isbn.is_none());
        assert!(books[0].formats.is_empty());
    }

    #[test]
    fn parses_real_world_entry_ignoring_unrelated_integer_id() {
        // Trimmed from a real `subscription.packtpub.com/api/entitlements/
        // users/me/owned` response. Note the top-level integer `id` (an
        // unrelated entitlement id) alongside the string `productId` we
        // actually want, plus fields this parser doesn't model
        // (`userId`, `simplifiedProduct`, ...) that must be silently
        // ignored rather than rejected. `count` is absent entirely here,
        // matching what was actually observed.
        let json = r#"{
            "message": "success",
            "data": [{
                "id": 43312892,
                "userId": "a766ea59-1e92-4c56-9ac7-ae6ba4f82d93",
                "productId": "9781835466759",
                "productName": "Build Apps and Fine-Tune LLMs Using the OpenAI API",
                "simplifiedProduct": { "title": "irrelevant nested field" }
            }]
        }"#;
        let (count, books) = parse_products_page(json).unwrap();
        assert_eq!(count, None);
        assert_eq!(books.len(), 1);
        assert_eq!(books[0].source_id, Some("9781835466759".to_string()));
        assert_eq!(
            books[0].title,
            "Build Apps and Fine-Tune LLMs Using the OpenAI API"
        );
        // No `isbn13` in this fixture's `simplifiedProduct` -- must degrade
        // to `None`, not error.
        assert_eq!(books[0].isbn, None);
    }

    #[test]
    fn parses_isbn_from_simplified_product() {
        let json = r#"{
            "data": [{
                "productId": "9781835466759",
                "productName": "Build Apps and Fine-Tune LLMs Using the OpenAI API",
                "simplifiedProduct": { "isbn10": "", "isbn13": "9781835466759" }
            }]
        }"#;
        let (_, books) = parse_products_page(json).unwrap();
        assert_eq!(books[0].isbn, Some("9781835466759".to_string()));
    }

    #[test]
    fn parses_cover_url_preferring_full_size_image() {
        let json = r#"{
            "data": [{
                "productName": "Some Book",
                "simplifiedProduct": {
                    "smallImage": "https://content.packt.com/V1/cover_small.jpg",
                    "coverImage": "https://content.packt.com/V1/cover.jpg"
                }
            }]
        }"#;
        let (_, books) = parse_products_page(json).unwrap();
        assert_eq!(
            books[0].cover_url,
            Some("https://content.packt.com/V1/cover.jpg".to_string())
        );
    }

    #[test]
    fn falls_back_to_small_image_when_cover_image_missing() {
        let json = r#"{
            "data": [{
                "productName": "Some Book",
                "simplifiedProduct": { "smallImage": "https://content.packt.com/V1/cover_small.jpg" }
            }]
        }"#;
        let (_, books) = parse_products_page(json).unwrap();
        assert_eq!(
            books[0].cover_url,
            Some("https://content.packt.com/V1/cover_small.jpg".to_string())
        );
    }

    #[test]
    fn missing_simplified_product_leaves_cover_url_unset() {
        let json = r#"{"data": [{ "productName": "Some Book" }]}"#;
        let (_, books) = parse_products_page(json).unwrap();
        assert_eq!(books[0].cover_url, None);
    }

    #[test]
    fn empty_isbn13_treated_as_absent() {
        // Observed live: non-book entries (e.g. video courses) can have an
        // empty-string `isbn13` rather than omitting the key entirely.
        let json = r#"{
            "data": [{
                "productName": "Some Video Course",
                "simplifiedProduct": { "isbn13": "" }
            }]
        }"#;
        let (_, books) = parse_products_page(json).unwrap();
        assert_eq!(books[0].isbn, None);
    }

    #[test]
    fn missing_id_degrades_to_none_instead_of_failing() {
        let json = r#"{"count": 1, "data": [{ "productName": "Some Book" }]}"#;
        let (_, books) = parse_products_page(json).unwrap();
        assert_eq!(books[0].source_id, None);
        assert_eq!(books[0].title, "Some Book");
    }

    #[test]
    fn empty_page_parses_cleanly() {
        let json = r#"{"count": 0, "data": []}"#;
        let (count, books) = parse_products_page(json).unwrap();
        assert_eq!(count, Some(0));
        assert!(books.is_empty());
    }

    #[test]
    fn xsrf_token_extracts_from_jar() {
        assert_eq!(
            xsrf_token("packt_session=sess123; XSRF-TOKEN=tok456"),
            Some("tok456")
        );
        assert_eq!(
            xsrf_token("XSRF-TOKEN=tok456; packt_session=sess123"),
            Some("tok456")
        );
        assert_eq!(xsrf_token("packt_session=sess123"), None);
        assert_eq!(xsrf_token(""), None);
    }

    #[test]
    fn fetch_products_without_xsrf_cookie_errors_before_any_request() {
        // No network I/O should happen -- the missing-cookie check must run
        // first, so this is safe to run without a live server.
        let err = fetch_products("packt_session=sess123").unwrap_err();
        assert!(err.to_string().contains("XSRF-TOKEN"));
    }
}
