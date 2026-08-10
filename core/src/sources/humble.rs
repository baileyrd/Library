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
//!
//! [`fetch_bundle_contents`] and [`fetch_active_bundle_urls`] are separate,
//! unauthenticated capabilities: they read public pages (a bundle's own
//! landing page, and the `/books` category page listing every bundle
//! currently for sale) rather than owned-order data, so "check before
//! buying" can list a bundle's books -- or check every bundle on sale
//! right now -- without ever needing the user's session cookie.

use std::collections::HashMap;
use std::thread;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use scraper::{Html, Selector};
use serde::Deserialize;

use crate::model::{Book, Source as BookSource};

const USER_AGENT: &str = "library-inventory/0.1";
/// Politeness pacing between successive requests to humblebundle.com when
/// fetching several bundle pages in a row (`fetch_all_active_bundles`) --
/// mirrors `sources::packt::PAGE_DELAY` and `enrich::REQUEST_DELAY`.
const BUNDLE_FETCH_DELAY: Duration = Duration::from_millis(400);

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
            // Humble's `subproducts[].icon` field exists, but it's a 70x70
            // square badge for their own compact library-grid UI, not book
            // cover art -- rendering it as one blows up to visible mush or
            // (worse) crops the edges off any cover text. No larger image
            // is available from this API. `enrich::enrich_missing` fills
            // this in later from Open Library by ISBN, same as Manning/
            // Kindle below.
            cover_url: None,
        });
    }

    Ok(books)
}

/// One ebook listed in a bundle's public landing page.
#[derive(Debug, Clone, PartialEq)]
pub struct BundleItem {
    pub title: String,
    pub authors: Vec<String>,
}

/// What a bundle landing page's contents look like, for "check before
/// buying" -- not an owned order, so no formats/ISBN/cover, just enough to
/// run through `dedup` against the existing library.
#[derive(Debug, Clone, PartialEq)]
pub struct BundleContents {
    pub bundle_name: String,
    pub items: Vec<BundleItem>,
}

/// Keyword list backing [`is_fiction_or_comic`] -- Humble Bundle's own
/// order API and bundle pages (see the module doc comment) expose no
/// genre/category field at all, so filtering fiction/comics out of
/// `check-bundle`/`check-bundles` results has nothing to go on but title
/// text. This is a best-effort heuristic, not a reliable classifier: it
/// will miss unlabeled fiction and can occasionally flag a non-fiction
/// title that happens to share a word (e.g. "Fantasy Football
/// Analytics"). Single words are matched whole-word (see
/// `is_fiction_or_comic`); multi-word phrases are matched as substrings.
const FICTION_OR_COMIC_KEYWORDS: &[&str] = &[
    "comic",
    "comics",
    "graphic novel",
    "manga",
    "novel",
    "trilogy",
    "saga",
    "chronicles",
    "fantasy",
    "science fiction",
    "sci-fi",
    "scifi",
    "thriller",
    "short stories",
];

/// Best-effort check for whether `title` reads as fiction or a
/// comic/graphic novel, from its title text alone. See
/// `FICTION_OR_COMIC_KEYWORDS`.
pub fn is_fiction_or_comic(title: &str) -> bool {
    let lower = title.to_lowercase();
    let words: std::collections::HashSet<&str> = lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    FICTION_OR_COMIC_KEYWORDS.iter().any(|kw| {
        if kw.chars().all(|c| c.is_alphanumeric()) {
            words.contains(kw)
        } else {
            lower.contains(kw)
        }
    })
}

/// Returns true if `bundle_name` contains any of `terms` (case-insensitive
/// substring match) -- backs the user-managed exclude list in
/// `Config::bundle_exclude_terms`, which skips whole bundles from
/// `check-bundles`/`check_active_bundles` results before they're ever
/// printed/shown (e.g. a term like "software" to hide recurring non-book
/// software bundles that also show up in the Books category listing).
/// Unlike `is_fiction_or_comic`'s fixed heuristic keyword list applied to
/// individual titles *within* a bundle, these terms are free-form and
/// user-supplied, and match against the whole bundle's own name -- so this
/// is a plain substring check, not a word-boundary heuristic. Blank terms
/// (e.g. from a stray empty string) never match anything.
pub fn matches_excluded_bundle(bundle_name: &str, terms: &[String]) -> bool {
    let lower = bundle_name.to_lowercase();
    terms
        .iter()
        .any(|term| !term.trim().is_empty() && lower.contains(&term.to_lowercase()))
}

#[derive(Debug, Deserialize)]
struct BundlePageData {
    #[serde(rename = "bundleData")]
    bundle_data: BundlePageBundleData,
}

#[derive(Debug, Deserialize)]
struct BundlePageBundleData {
    basic_data: BundlePageBasicData,
    #[serde(default)]
    tier_item_data: HashMap<String, BundlePageItem>,
}

#[derive(Debug, Deserialize)]
struct BundlePageBasicData {
    human_name: String,
}

#[derive(Debug, Deserialize)]
struct BundlePageItem {
    human_name: String,
    /// Every real book observed live is `"ebook"`; non-book tier fillers
    /// (a charity donation slider, upsells) either omit this or use a
    /// different value, so filtering on it is what separates actual books
    /// from bundle chrome.
    #[serde(default)]
    item_content_type: Option<String>,
    #[serde(default)]
    developers: Vec<BundlePageDeveloper>,
}

#[derive(Debug, Deserialize)]
struct BundlePageDeveloper {
    #[serde(rename = "developer-name")]
    developer_name: String,
}

/// Fetches a public Humble Bundle "books" bundle landing page (e.g.
/// `https://www.humblebundle.com/books/<slug>`) and lists the ebooks in
/// it, for checking against the library before buying. Unlike
/// `fetch_orders`, this needs no session cookie -- a bundle's contents are
/// a public marketing page, not account data.
pub fn fetch_bundle_contents(url: &str) -> Result<BundleContents> {
    validate_bundle_url(url)?;

    let client = reqwest::blocking::Client::builder()
        .user_agent(USER_AGENT)
        .build()
        .context("failed to build HTTP client")?;
    let html = client
        .get(url)
        .send()
        .context("failed to fetch bundle page")?
        .text()
        .context("failed to read bundle page response body")?;
    parse_bundle_page(&html)
}

/// Rejects anything that isn't plausibly a `humblebundle.com` page before
/// making a request -- this URL comes straight from a text field in the
/// desktop app / CLI arg, so it's untrusted user input, not a URL this
/// program constructed itself. Uses `reqwest::Url` (already a dependency
/// for the HTTP client) rather than pulling in a dedicated `url` crate.
fn validate_bundle_url(url: &str) -> Result<()> {
    let parsed = reqwest::Url::parse(url).with_context(|| format!("not a valid URL: {url}"))?;
    let is_humble_host = parsed
        .host_str()
        .is_some_and(|h| h == "humblebundle.com" || h.ends_with(".humblebundle.com"));
    if !is_humble_host {
        bail!("not a humblebundle.com URL: {url}");
    }
    Ok(())
}

/// Pure parsing, split out from `fetch_bundle_contents` for testing
/// without a live network call -- same pattern as `parse_order_response`.
/// The page embeds its data as JSON in a `<script id="webpack-bundle-page-
/// data">` tag rather than exposing it via any API, so this is HTML
/// scraping (like `sources::manning`) down to that one tag, then plain
/// JSON parsing of its contents.
pub fn parse_bundle_page(html: &str) -> Result<BundleContents> {
    let document = Html::parse_document(html);
    let selector = Selector::parse("script#webpack-bundle-page-data")
        .map_err(|e| anyhow!("invalid selector: {e:?}"))?;
    let script = document.select(&selector).next().context(
        "bundle page has no webpack-bundle-page-data script tag -- \
         not a Humble Bundle books bundle URL, or Humble changed its page layout",
    )?;
    let json = script.text().collect::<String>();
    let page: BundlePageData =
        serde_json::from_str(&json).context("failed to parse bundle page JSON")?;

    let mut items: Vec<BundleItem> = page
        .bundle_data
        .tier_item_data
        .into_values()
        .filter(|item| item.item_content_type.as_deref() == Some("ebook"))
        .map(|item| BundleItem {
            title: item.human_name,
            authors: item
                .developers
                .into_iter()
                .map(|d| d.developer_name)
                .collect(),
        })
        .collect();
    // `tier_item_data` is a JSON object -- HashMap iteration order is
    // unspecified, so sort for deterministic, readable output.
    items.sort_by(|a, b| a.title.cmp(&b.title));

    Ok(BundleContents {
        bundle_name: page.bundle_data.basic_data.human_name,
        items,
    })
}

const BOOKS_LISTING_URL: &str = "https://www.humblebundle.com/books";

#[derive(Debug, Deserialize)]
struct LandingPageData {
    data: LandingPageInnerData,
}

#[derive(Debug, Deserialize)]
struct LandingPageInnerData {
    books: LandingPageBooks,
}

#[derive(Debug, Deserialize)]
struct LandingPageBooks {
    #[serde(default)]
    mosaic: Vec<LandingPageSection>,
}

#[derive(Debug, Deserialize)]
struct LandingPageSection {
    #[serde(default)]
    products: Vec<LandingPageProduct>,
}

#[derive(Debug, Deserialize)]
struct LandingPageProduct {
    category: String,
    product_url: String,
}

/// Fetches `humblebundle.com/books` (the "Books" storefront category page)
/// and lists the full URL of every bundle currently on sale there, so
/// "check before buying" can screen all of them at once instead of
/// requiring a pasted URL per bundle.
pub fn fetch_active_bundle_urls() -> Result<Vec<String>> {
    let client = reqwest::blocking::Client::builder()
        .user_agent(USER_AGENT)
        .build()
        .context("failed to build HTTP client")?;
    let html = client
        .get(BOOKS_LISTING_URL)
        .send()
        .context("failed to fetch the books bundle listing page")?
        .text()
        .context("failed to read books bundle listing response body")?;
    parse_active_bundle_urls(&html)
}

/// Pure parsing, split out from `fetch_active_bundle_urls` for testing
/// without a live network call. Like the bundle page itself, the listing
/// embeds its data as JSON in a `<script id="landingPage-json-data">` tag
/// -- `data.books.mosaic[].products[]`, filtered to `category == "bundle"`
/// (the same category page can list ebooks in a bundle vs. sold
/// individually; only bundles have a `parse_bundle_page`-compatible page).
fn parse_active_bundle_urls(html: &str) -> Result<Vec<String>> {
    let document = Html::parse_document(html);
    let selector = Selector::parse("script#landingPage-json-data")
        .map_err(|e| anyhow!("invalid selector: {e:?}"))?;
    let script = document.select(&selector).next().context(
        "books listing page has no landingPage-json-data script tag -- \
         Humble may have changed its page layout",
    )?;
    let json = script.text().collect::<String>();
    let page: LandingPageData =
        serde_json::from_str(&json).context("failed to parse books listing page JSON")?;

    let mut urls: Vec<String> = page
        .data
        .books
        .mosaic
        .into_iter()
        .flat_map(|section| section.products)
        .filter(|p| p.category == "bundle")
        .map(|p| format!("https://www.humblebundle.com{}", p.product_url))
        .collect();
    urls.sort();
    urls.dedup();
    Ok(urls)
}

/// One bundle's fetch/parse outcome as part of a batch -- `Err` here means
/// that specific bundle failed, not the whole batch.
pub struct ActiveBundleCheck {
    pub url: String,
    pub result: Result<BundleContents>,
}

/// Discovers every bundle currently listed on `humblebundle.com/books` and
/// fetches each one's contents, so the whole storefront can be screened
/// for "do I already own any of these?" with a single click. Best-effort
/// per bundle, like `enrich::enrich_missing`: one bundle's fetch/parse
/// failure (e.g. an unusually-shaped promo page) is reported alongside the
/// rest rather than aborting the whole batch. Only the initial discovery
/// step (listing the URLs at all) is fatal.
pub fn fetch_all_active_bundles() -> Result<Vec<ActiveBundleCheck>> {
    let urls = fetch_active_bundle_urls()?;
    let mut results = Vec::with_capacity(urls.len());
    for (i, url) in urls.into_iter().enumerate() {
        if i > 0 {
            thread::sleep(BUNDLE_FETCH_DELAY);
        }
        let result = fetch_bundle_contents(&url);
        results.push(ActiveBundleCheck { url, result });
    }
    Ok(results)
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

    const BUNDLE_PAGE_FIXTURE: &str = r#"
    <html><body>
    <script id="webpack-bundle-page-data" type="application/json">
    {
        "bundleData": {
            "machine_name": "rust_book_bundle",
            "basic_data": { "human_name": "Humble Tech Book Bundle: Rust" },
            "tier_item_data": {
                "programming_rust": {
                    "human_name": "Programming Rust",
                    "item_content_type": "ebook",
                    "developers": [
                        { "developer-name": "Jim Blandy" },
                        { "developer-name": "Jason Orendorff" }
                    ]
                },
                "rust_in_action": {
                    "human_name": "Rust in Action",
                    "item_content_type": "ebook",
                    "developers": [{ "developer-name": "Tim McNamara" }]
                },
                "savethechildren": {
                    "human_name": "Save the Children",
                    "developers": []
                }
            }
        }
    }
    </script>
    </body></html>
    "#;

    #[test]
    fn parses_bundle_name_and_ebook_items_only() {
        let contents = parse_bundle_page(BUNDLE_PAGE_FIXTURE).unwrap();
        assert_eq!(contents.bundle_name, "Humble Tech Book Bundle: Rust");
        assert_eq!(contents.items.len(), 2);
        assert!(contents.items.iter().all(|i| i.title != "Save the Children"));
    }

    #[test]
    fn bundle_items_sorted_by_title_with_authors() {
        let contents = parse_bundle_page(BUNDLE_PAGE_FIXTURE).unwrap();
        assert_eq!(
            contents.items[0],
            BundleItem {
                title: "Programming Rust".to_string(),
                authors: vec!["Jim Blandy".to_string(), "Jason Orendorff".to_string()],
            }
        );
        assert_eq!(
            contents.items[1],
            BundleItem {
                title: "Rust in Action".to_string(),
                authors: vec!["Tim McNamara".to_string()],
            }
        );
    }

    #[test]
    fn non_bundle_page_errors_instead_of_panicking() {
        let result = parse_bundle_page("<html><body>not a bundle page</body></html>");
        assert!(result.is_err());
    }

    #[test]
    fn accepts_humblebundle_dot_com_urls() {
        assert!(validate_bundle_url("https://www.humblebundle.com/books/some-bundle").is_ok());
        assert!(validate_bundle_url("https://humblebundle.com/books/some-bundle").is_ok());
    }

    #[test]
    fn rejects_other_hosts_and_garbage() {
        assert!(validate_bundle_url("https://evil.example.com/books/some-bundle").is_err());
        assert!(validate_bundle_url("not a url").is_err());
    }

    const LANDING_PAGE_FIXTURE: &str = r#"
    <html><body>
    <script id="landingPage-json-data" type="application/json">
    {
        "data": {
            "books": {
                "mosaic": [
                    {
                        "products": [
                            {
                                "category": "bundle",
                                "product_url": "/books/tech-career-library-in-age-ai-apress-books"
                            },
                            {
                                "category": "bundle",
                                "product_url": "/books/software-architecture-apress-books"
                            },
                            {
                                "category": "storefront",
                                "product_url": "/store/some-single-book"
                            }
                        ]
                    },
                    {
                        "products": [
                            {
                                "category": "bundle",
                                "product_url": "/books/tech-career-library-in-age-ai-apress-books"
                            }
                        ]
                    }
                ]
            }
        }
    }
    </script>
    </body></html>
    "#;

    #[test]
    fn parses_bundle_urls_across_sections_deduped_and_sorted() {
        let urls = parse_active_bundle_urls(LANDING_PAGE_FIXTURE).unwrap();
        assert_eq!(
            urls,
            vec![
                "https://www.humblebundle.com/books/software-architecture-apress-books"
                    .to_string(),
                "https://www.humblebundle.com/books/tech-career-library-in-age-ai-apress-books"
                    .to_string(),
            ]
        );
    }

    #[test]
    fn non_bundle_categories_excluded_from_listing() {
        let urls = parse_active_bundle_urls(LANDING_PAGE_FIXTURE).unwrap();
        assert!(urls.iter().all(|u| !u.contains("/store/")));
    }

    #[test]
    fn non_listing_page_errors_instead_of_panicking() {
        let result = parse_active_bundle_urls("<html><body>not a listing page</body></html>");
        assert!(result.is_err());
    }

    #[test]
    fn flags_comics_and_fiction_titles() {
        assert!(is_fiction_or_comic("Batman: The Long Halloween (Comics)"));
        assert!(is_fiction_or_comic("Saga, Vol. 1 (Graphic Novel)"));
        assert!(is_fiction_or_comic("Neuromancer: A Novel"));
        assert!(is_fiction_or_comic("The Fantasy & Science Fiction Megapack"));
        assert!(is_fiction_or_comic("Attack on Titan Manga Collection"));
        assert!(is_fiction_or_comic("Best Sci-Fi Short Story Collection"));
    }

    #[test]
    fn leaves_technical_titles_alone() {
        // "fantasy" is a known false-positive source by design (see
        // `FICTION_OR_COMIC_KEYWORDS`'s doc comment) -- a title like
        // "Fantasy Football Analytics" is intentionally not covered here.
        assert!(!is_fiction_or_comic("Programming Rust"));
        assert!(!is_fiction_or_comic("Rust in Action"));
        assert!(!is_fiction_or_comic("Kubernetes Patterns"));
    }

    #[test]
    fn matches_excluded_bundle_finds_case_insensitive_substring() {
        let terms = vec!["software".to_string(), "board game".to_string()];
        assert!(matches_excluded_bundle("Humble Software Bundle", &terms));
        assert!(matches_excluded_bundle(
            "The Humble Board Game Design Bundle",
            &terms
        ));
        assert!(!matches_excluded_bundle("Humble Tech Book Bundle: Rust", &terms));
    }

    #[test]
    fn matches_excluded_bundle_ignores_blank_terms_and_empty_list() {
        assert!(!matches_excluded_bundle("Any Bundle Name", &[]));
        assert!(!matches_excluded_bundle(
            "Any Bundle Name",
            &["   ".to_string(), "".to_string()]
        ));
    }
}
