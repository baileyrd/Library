//! Manning source.
//!
//! Manning has no public JSON API for entitlements, so this scrapes the
//! account dashboard HTML. Auth is the user's full `manning.com` +
//! `login.manning.com` cookie jar (semicolon-separated `name=value` pairs
//! copied from browser devtools), sent as a raw `Cookie` header.
//!
//! Reverse-engineered from a third-party tool, not official docs -- the
//! selectors and attribute names here should be verified against a real
//! dashboard page and adjusted if Manning's markup has changed.

use anyhow::{anyhow, Context, Result};
use scraper::{Html, Selector};

use crate::model::{Book, Source as BookSource};

const DASHBOARD_URL: &str =
    "https://www.manning.com/dashboard/index?filter=book&max=999&order=lastUpdated&sort=desc";

pub struct Manning {
    pub cookies: String,
}

impl super::Source for Manning {
    fn name(&self) -> &'static str {
        "manning"
    }

    fn fetch(&self) -> Result<Vec<Book>> {
        fetch_dashboard(&self.cookies)
    }
}

pub fn fetch_dashboard(cookies: &str) -> Result<Vec<Book>> {
    let client = reqwest::blocking::Client::new();
    let html = client
        .get(DASHBOARD_URL)
        .header("Cookie", cookies)
        .header("User-Agent", "library-inventory/0.1")
        .send()
        .context("failed to fetch Manning dashboard")?
        .text()
        .context("failed to read Manning dashboard response body")?;

    parse_dashboard_html(&html)
}

pub fn parse_dashboard_html(html: &str) -> Result<Vec<Book>> {
    let document = Html::parse_document(html);
    let row_selector = Selector::parse("#productTable > tbody > tr.license-row")
        .map_err(|e| anyhow!("invalid row selector: {e:?}"))?;
    let title_selector =
        Selector::parse(".product-title").map_err(|e| anyhow!("invalid title selector: {e:?}"))?;
    let form_selector = Selector::parse("form.download-form")
        .map_err(|e| anyhow!("invalid form selector: {e:?}"))?;

    let mut books = Vec::new();
    for row in document.select(&row_selector) {
        let title = row
            .select(&title_selector)
            .next()
            .map(|el| el.text().collect::<String>().trim().to_string())
            .unwrap_or_default();
        if title.is_empty() {
            continue;
        }

        // Exact attribute name reverse-engineered, not confirmed against
        // official docs -- adjust if Manning's markup uses something else.
        let is_meap = row
            .value()
            .attr("data-is-meap")
            .map(|v| v == "true" || v == "1")
            .unwrap_or(false);

        let source_id = row
            .select(&form_selector)
            .next()
            .and_then(|form| form.value().attr("name"))
            .and_then(|name| name.strip_prefix("downloadForm-"))
            .map(|slug| slug.to_string());

        books.push(Book {
            id: None,
            title,
            // Not reliably available on the dashboard page.
            authors: Vec::new(),
            // Not confirmed present on the dashboard page.
            isbn: None,
            source: BookSource::Manning,
            source_id,
            // Not scraped from this page; would need a per-book request.
            formats: Vec::new(),
            acquired_at: None,
            raw_json: Some(format!("is_meap={is_meap}")),
            // Not scraped from this page -- no live sample of the
            // dashboard's markup was available to confirm a cover-image
            // selector, unlike Packt/Humble Bundle where the API response
            // shape was confirmed directly.
            cover_url: None,
        });
    }

    Ok(books)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DASHBOARD_FIXTURE: &str = r#"
    <html>
      <body>
        <table id="productTable">
          <tbody>
            <tr class="license-row" data-is-meap="false">
              <td class="product-title">Rust in Action</td>
              <td>
                <form class="download-form" name="downloadForm-rust-in-action"></form>
              </td>
            </tr>
            <tr class="license-row" data-is-meap="true">
              <td class="product-title">Zero To Production In Rust</td>
              <td>
                <form class="download-form" name="downloadForm-zero-to-production-in-rust"></form>
              </td>
            </tr>
          </tbody>
        </table>
      </body>
    </html>
    "#;

    #[test]
    fn parses_both_book_and_meap_rows() {
        let books = parse_dashboard_html(DASHBOARD_FIXTURE).unwrap();
        assert_eq!(books.len(), 2);
    }

    #[test]
    fn extracts_title_and_source_id() {
        let books = parse_dashboard_html(DASHBOARD_FIXTURE).unwrap();
        let ria = books.iter().find(|b| b.title == "Rust in Action").unwrap();
        assert_eq!(ria.source_id, Some("rust-in-action".to_string()));

        let zero_to_prod = books
            .iter()
            .find(|b| b.title == "Zero To Production In Rust")
            .unwrap();
        assert_eq!(
            zero_to_prod.source_id,
            Some("zero-to-production-in-rust".to_string())
        );
    }

    #[test]
    fn formats_are_left_empty() {
        let books = parse_dashboard_html(DASHBOARD_FIXTURE).unwrap();
        assert!(books.iter().all(|b| b.formats.is_empty()));
        assert!(books.iter().all(|b| b.authors.is_empty()));
        assert!(books.iter().all(|b| b.isbn.is_none()));
    }
}
