use serde::Serialize;
use tauri::{Manager, State};

use library_core::dedup::{self, DedupMatch};
use library_core::enrich::{self, EnrichSummary};
use library_core::model::{Book, Source};
use library_core::sources::capture::{self, CaptureSpec};
use library_core::sources::{self, Source as SourceFetcher};

use crate::state::AppState;

/// Fuzzy matches below this confidence, but at or above this one, are
/// returned separately as weaker, manually-reviewed candidates -- mirrors
/// the CLI's `check` command threshold. Set equal to the 0.90 strong-match
/// cutoff used below (real data check: every weak-bucket match sampled in
/// the 0.75-0.90 range was a false positive from generic shared title
/// words like "Mastering"/"Learning" scoring high on Jaro-Winkler's
/// prefix bonus -- every genuine duplicate already scored >= 0.90). This
/// makes the "weak" bucket structurally always empty for now; left in
/// place rather than removed since it's a one-line reversion if a real
/// gap between the two cutoffs turns out to be wanted later.
const CHECK_WEAK_THRESHOLD: f64 = 0.90;

/// Renders the full anyhow cause chain, not just the top-level message --
/// `anyhow::Error::to_string()` alone drops context added via `.context()`/
/// `.with_context()` further down the chain (e.g. a fetch error's response
/// body snippet), which would otherwise never reach the UI.
fn err(e: anyhow::Error) -> String {
    let mut msg = e.to_string();
    for cause in e.chain().skip(1) {
        msg.push_str(&format!(" -- caused by: {cause}"));
    }
    msg
}

#[derive(Serialize)]
pub struct BookListEntry {
    #[serde(flatten)]
    book: Book,
    /// Other sources that also appear to own this title, computed across
    /// the whole library -- surfaces the same signal `import_source`'s
    /// one-shot duplicate warning uses, but persistently in the list
    /// instead of only in a toast that's gone once dismissed.
    duplicate_sources: Vec<Source>,
}

#[tauri::command]
pub fn list_books(
    state: State<AppState>,
    source: Option<String>,
) -> Result<Vec<BookListEntry>, String> {
    let source_filter = source
        .map(|s| s.parse::<Source>())
        .transpose()
        .map_err(err)?;
    let db = state.db.lock();

    // Cross-source duplicates need the whole library to detect, even when
    // only one source's books are being displayed.
    let all_books = db.all_books().map_err(err)?;
    let dup_sources = dedup::cross_source_duplicates(&all_books);
    let books = match source_filter {
        Some(filter) => all_books
            .into_iter()
            .filter(|b| b.source == filter)
            .collect(),
        None => all_books,
    };

    Ok(books
        .into_iter()
        .map(|book| {
            let duplicate_sources = book
                .id
                .and_then(|id| dup_sources.get(&id))
                .cloned()
                .unwrap_or_default();
            BookListEntry {
                book,
                duplicate_sources,
            }
        })
        .collect())
}

#[tauri::command]
pub fn get_book(state: State<AppState>, id: i64) -> Result<Option<Book>, String> {
    let db = state.db.lock();
    db.get_book(id).map_err(err)
}

#[derive(Serialize)]
pub struct BookDetail {
    #[serde(flatten)]
    book: Book,
    /// Other books in the library that look like the same title from a
    /// different source -- the same signal `list_books`' compact
    /// `duplicate_sources` badge uses, but with full match detail (which
    /// book, confidence, why) for the detail page instead of just source
    /// names.
    duplicates: Vec<DedupMatch>,
}

#[tauri::command]
pub fn get_book_detail(state: State<AppState>, id: i64) -> Result<Option<BookDetail>, String> {
    let db = state.db.lock();
    let Some(book) = db.get_book(id).map_err(err)? else {
        return Ok(None);
    };
    let others: Vec<Book> = db
        .all_books()
        .map_err(err)?
        .into_iter()
        .filter(|b| b.id != Some(id))
        .collect();
    let duplicates = dedup::find_duplicates(&others, &book)
        .into_iter()
        .filter(|m| m.book.source != book.source)
        .collect();
    Ok(Some(BookDetail { book, duplicates }))
}

#[tauri::command]
pub fn add_book(
    state: State<AppState>,
    title: String,
    authors: Vec<String>,
    isbn: Option<String>,
    formats: Vec<String>,
    cover_url: Option<String>,
) -> Result<AddBookResult, String> {
    let db = state.db.lock();
    let existing = db.all_books().map_err(err)?;
    let candidate = sources::manual::build_manual_book(title, authors, isbn, formats, cover_url);
    let warnings = dedup::find_duplicates(&existing, &candidate);

    let outcome = db.upsert_book(&candidate).map_err(err)?;
    let id = match outcome {
        library_core::db::UpsertOutcome::Inserted(id) => id,
        library_core::db::UpsertOutcome::Updated(id) => id,
    };
    let book = db
        .get_book(id)
        .map_err(err)?
        .ok_or_else(|| "book vanished after insert".to_string())?;

    Ok(AddBookResult { book, warnings })
}

#[derive(Serialize)]
pub struct AddBookResult {
    book: Book,
    warnings: Vec<DedupMatch>,
}

#[tauri::command]
pub fn update_book(
    state: State<AppState>,
    id: i64,
    title: String,
    authors: Vec<String>,
    isbn: Option<String>,
    formats: Vec<String>,
    cover_url: Option<String>,
) -> Result<Book, String> {
    let db = state.db.lock();
    let changed = db
        .update_book(
            id,
            &title,
            &authors,
            isbn.as_deref(),
            &formats,
            cover_url.as_deref(),
        )
        .map_err(err)?;
    if !changed {
        return Err(format!("no book with id {id}"));
    }
    db.get_book(id)
        .map_err(err)?
        .ok_or_else(|| format!("no book with id {id}"))
}

#[tauri::command]
pub fn remove_book(state: State<AppState>, id: i64) -> Result<bool, String> {
    let db = state.db.lock();
    db.delete_book(id).map_err(err)
}

#[derive(Serialize)]
pub struct CheckResult {
    strong: Vec<DedupMatch>,
    weak: Vec<DedupMatch>,
}

#[tauri::command]
pub fn check_duplicates(state: State<AppState>, query: String) -> Result<CheckResult, String> {
    let digits: String = query.chars().filter(|c| c.is_ascii_digit()).collect();
    let isbn = if digits.len() == 10 || digits.len() == 13 {
        Some(digits)
    } else {
        None
    };

    let candidate = Book {
        id: None,
        title: query,
        authors: Vec::new(),
        isbn,
        source: Source::Manual,
        source_id: None,
        formats: Vec::new(),
        acquired_at: None,
        raw_json: None,
        cover_url: None,
    };

    let db = state.db.lock();
    let existing = db.all_books().map_err(err)?;
    let matches =
        dedup::find_duplicates_with_threshold(&existing, &candidate, CHECK_WEAK_THRESHOLD);
    let (strong, weak): (Vec<_>, Vec<_>) = matches.into_iter().partition(|m| m.confidence >= 0.90);

    Ok(CheckResult { strong, weak })
}

#[derive(Serialize)]
pub struct BundleCheckItem {
    title: String,
    authors: Vec<String>,
    strong: Vec<DedupMatch>,
    weak: Vec<DedupMatch>,
}

#[derive(Serialize)]
pub struct BundleCheckResult {
    url: String,
    bundle_name: String,
    items: Vec<BundleCheckItem>,
    /// Set when this specific bundle's fetch/parse failed -- `items` is
    /// empty in that case. Only meaningful from `check_active_bundles`,
    /// which tolerates one bad bundle without failing the whole batch;
    /// `check_bundle_url` fails the command outright instead, since
    /// there's nothing else to report alongside a single explicit check.
    error: Option<String>,
}

/// Scores every book in `contents` against `existing` with the same
/// title/ISBN dedup logic `check_duplicates` uses for a single query.
fn score_bundle(
    existing: &[Book],
    url: String,
    contents: sources::humble::BundleContents,
) -> BundleCheckResult {
    let items = contents
        .items
        .into_iter()
        .map(|item| {
            let candidate = Book {
                id: None,
                title: item.title.clone(),
                authors: item.authors.clone(),
                isbn: None,
                source: Source::Manual,
                source_id: None,
                formats: Vec::new(),
                acquired_at: None,
                raw_json: None,
                cover_url: None,
            };
            let matches =
                dedup::find_duplicates_with_threshold(existing, &candidate, CHECK_WEAK_THRESHOLD);
            let (strong, weak): (Vec<_>, Vec<_>) =
                matches.into_iter().partition(|m| m.confidence >= 0.90);
            BundleCheckItem {
                title: item.title,
                authors: item.authors,
                strong,
                weak,
            }
        })
        .collect();

    BundleCheckResult {
        url,
        bundle_name: contents.bundle_name,
        items,
        error: None,
    }
}

/// Checks every book in a public Humble Bundle bundle page against the
/// library, so a whole bundle can be screened for "do I already own any of
/// these?" before buying instead of pasting titles into `check_duplicates`
/// one at a time. Needs no Humble session -- bundle contents are a public
/// page, unlike `import_source`'s owned-order fetch.
#[tauri::command]
pub fn check_bundle_url(
    state: State<AppState>,
    url: String,
    exclude_fiction: bool,
) -> Result<BundleCheckResult, String> {
    let mut contents = sources::humble::fetch_bundle_contents(&url).map_err(err)?;
    if exclude_fiction {
        contents
            .items
            .retain(|item| !sources::humble::is_fiction_or_comic(&item.title));
    }
    let db = state.db.lock();
    let existing = db.all_books().map_err(err)?;
    Ok(score_bundle(&existing, url, contents))
}

/// Discovers every bundle currently listed on humblebundle.com/books and
/// checks all of them at once -- the one-click version of
/// `check_bundle_url` for "is anything on sale right now something I
/// already own?" instead of finding and pasting each bundle's URL by hand.
/// One bundle's fetch/parse failure is reported inline via that bundle's
/// `error` field rather than failing the whole batch (see
/// `sources::humble::fetch_all_active_bundles`); only a failure to list
/// the bundles at all fails the command.
#[tauri::command]
pub fn check_active_bundles(
    state: State<AppState>,
    exclude_fiction: bool,
) -> Result<Vec<BundleCheckResult>, String> {
    let exclude_terms = state.config.lock().bundle_exclude_terms.clone();
    let mut checks = sources::humble::fetch_all_active_bundles().map_err(err)?;
    checks.retain(|check| match &check.result {
        Ok(contents) => {
            !sources::humble::matches_excluded_bundle(&contents.bundle_name, &exclude_terms)
        }
        Err(_) => true,
    });
    let db = state.db.lock();
    let existing = db.all_books().map_err(err)?;

    Ok(checks
        .into_iter()
        .map(|check| match check.result {
            Ok(mut contents) => {
                if exclude_fiction {
                    contents
                        .items
                        .retain(|item| !sources::humble::is_fiction_or_comic(&item.title));
                }
                score_bundle(&existing, check.url, contents)
            }
            Err(e) => BundleCheckResult {
                url: check.url,
                bundle_name: String::new(),
                items: Vec::new(),
                error: Some(err(e)),
            },
        })
        .collect())
}

/// Opens `url` in the OS's default browser -- used for "open this on
/// humblebundle.com" links from the bundle checkers, since a plain `<a
/// href>` inside the webview would navigate the app's own window away
/// instead of launching an external browser. Scheme-restricted to http(s)
/// so this can't be repurposed to open an arbitrary local path.
#[tauri::command]
pub fn open_url(url: String) -> Result<(), String> {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("refusing to open a non-http(s) URL".to_string());
    }
    open::that(&url).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn stats(state: State<AppState>) -> Result<Vec<(Source, i64)>, String> {
    let db = state.db.lock();
    db.stats().map_err(err)
}

#[derive(Serialize)]
pub struct ConfigStatus {
    humble_cookie_set: bool,
    packt_cookies_set: bool,
    manning_cookies_set: bool,
    db_path: String,
    bundle_exclude_terms: Vec<String>,
}

#[tauri::command]
pub fn get_config_status(state: State<AppState>) -> ConfigStatus {
    let config = state.config.lock();
    ConfigStatus {
        humble_cookie_set: config.humble_cookie.is_some(),
        packt_cookies_set: config.packt_cookies.is_some(),
        manning_cookies_set: config.manning_cookies.is_some(),
        db_path: config.resolve_db_path().display().to_string(),
        bundle_exclude_terms: config.bundle_exclude_terms.clone(),
    }
}

#[tauri::command]
pub fn set_credential(state: State<AppState>, field: String, value: String) -> Result<(), String> {
    let mut config = state.config.lock();
    match field.as_str() {
        "humble_cookie" => config.humble_cookie = Some(value),
        "packt_cookies" => config.packt_cookies = Some(value),
        "manning_cookies" => config.manning_cookies = Some(value),
        other => return Err(format!("unknown credential field '{other}'")),
    }
    config.save().map_err(err)
}

/// Adds a term to the bundle-name exclude list (see
/// `library_core::sources::humble::matches_excluded_bundle`), no-op if
/// already present (case-insensitive). Returns the full list afterward so
/// the frontend can just re-render it, instead of round-tripping a second
/// `get_config_status` call.
#[tauri::command]
pub fn add_bundle_exclude_term(
    state: State<AppState>,
    term: String,
) -> Result<Vec<String>, String> {
    let trimmed = term.trim();
    if trimmed.is_empty() {
        return Err("exclude term must not be empty".to_string());
    }
    let mut config = state.config.lock();
    if !config
        .bundle_exclude_terms
        .iter()
        .any(|t| t.eq_ignore_ascii_case(trimmed))
    {
        config.bundle_exclude_terms.push(trimmed.to_string());
        config.save().map_err(err)?;
    }
    Ok(config.bundle_exclude_terms.clone())
}

/// Removes a term from the bundle-name exclude list (case-insensitive
/// match), no-op if not present. Returns the full list afterward, like
/// `add_bundle_exclude_term`.
#[tauri::command]
pub fn remove_bundle_exclude_term(
    state: State<AppState>,
    term: String,
) -> Result<Vec<String>, String> {
    let mut config = state.config.lock();
    let before = config.bundle_exclude_terms.len();
    config
        .bundle_exclude_terms
        .retain(|t| !t.eq_ignore_ascii_case(term.trim()));
    if config.bundle_exclude_terms.len() != before {
        config.save().map_err(err)?;
    }
    Ok(config.bundle_exclude_terms.clone())
}

#[derive(Serialize)]
pub struct ImportSummary {
    source: String,
    new_count: i64,
    updated_count: i64,
    warnings: Vec<String>,
}

#[tauri::command]
pub fn import_source(
    state: State<AppState>,
    source: String,
    file: Option<String>,
) -> Result<ImportSummary, String> {
    let fetcher: Box<dyn SourceFetcher> = match source.as_str() {
        "humble" => {
            let cookie = {
                let config = state.config.lock();
                config.humble_cookie.clone()
            }
            .ok_or_else(|| "no Humble Bundle cookie configured".to_string())?;
            Box::new(sources::humble::Humble { cookie })
        }
        "packt" => {
            let cookies = {
                let config = state.config.lock();
                config.packt_cookies.clone()
            }
            .ok_or_else(|| "no Packt cookies configured".to_string())?;
            Box::new(sources::packt::Packt { cookies })
        }
        "manning" => {
            let cookies = {
                let config = state.config.lock();
                config.manning_cookies.clone()
            }
            .ok_or_else(|| "no Manning cookies configured".to_string())?;
            Box::new(sources::manning::Manning { cookies })
        }
        "kindle" => {
            let path = file.ok_or_else(|| "no CSV file selected".to_string())?;
            Box::new(sources::kindle::Kindle { path: path.into() })
        }
        other => return Err(format!("unknown source '{other}'")),
    };

    let books = fetcher.fetch().map_err(err)?;
    let db = state.db.lock();
    let baseline = db.all_books().map_err(err)?;

    let mut new_count = 0;
    let mut updated_count = 0;
    let mut warnings = Vec::new();

    for book in books {
        let outcome = db.upsert_book(&book).map_err(err)?;
        match outcome {
            library_core::db::UpsertOutcome::Inserted(_) => {
                new_count += 1;
                for m in dedup::find_duplicates(&baseline, &book) {
                    if m.book.source == book.source {
                        continue;
                    }
                    warnings.push(format!(
                        "'{}' looks like a possible duplicate of '{}' already in your library from {} (confidence {:.2}, {})",
                        book.title, m.book.title, m.book.source, m.confidence, m.reason
                    ));
                }
            }
            library_core::db::UpsertOutcome::Updated(_) => updated_count += 1,
        }
    }

    Ok(ImportSummary {
        source,
        new_count,
        updated_count,
        warnings,
    })
}

#[derive(Serialize)]
pub struct CaptureResult {
    /// True when the login window was closed (or the 10-minute timeout hit)
    /// before capture completed -- nothing was saved.
    cancelled: bool,
}

/// Opens an embedded login window for `source`, waits for the resulting
/// session to appear in its cookie jar (see `library_core::sources::capture`
/// for exactly what each source waits for), and saves it straight into
/// config -- no devtools, no copy-pasting.
#[tauri::command]
pub async fn capture_credential(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    source: String,
) -> Result<CaptureResult, String> {
    let spec: &'static CaptureSpec =
        capture::spec_for(&source).ok_or_else(|| format!("unknown capture source '{source}'"))?;

    let label = format!("login-{source}");
    if let Some(existing) = app.get_webview_window(&label) {
        let _ = existing.close();
    }

    let login_url = spec
        .login_url
        .parse::<tauri::Url>()
        .map_err(|e| e.to_string())?;
    let window =
        tauri::WebviewWindowBuilder::new(&app, &label, tauri::WebviewUrl::External(login_url))
            .title(format!("Library \u{2014} sign in to {}", spec.label))
            .inner_size(480.0, 760.0)
            .initialization_script(&capture::injected_script(spec))
            .build()
            .map_err(|e| e.to_string())?;

    let domains = spec
        .cookie_domains
        .iter()
        .map(|d| d.parse::<tauri::Url>().map_err(|e| e.to_string()))
        .collect::<Result<Vec<_>, _>>()?;

    let app_for_poll = app.clone();
    let label_for_poll = label.clone();
    let captured = tauri::async_runtime::spawn_blocking(move || -> Option<String> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(600);
        loop {
            if app_for_poll.get_webview_window(&label_for_poll).is_none() {
                // The user closed the window before finishing login.
                return None;
            }

            // `WebviewWindow::cookies_for_url` dispatches to the main thread
            // and blocks on a channel reply; if the window gets torn down in
            // the gap between the check above and this call,
            // tauri-runtime-wry's dispatcher drops the sender without
            // replying and its `rx.recv().unwrap()` panics. That race is
            // rare but real (more exposure the more domains a source polls),
            // so treat a caught panic/error the same as a transient miss --
            // skip this cycle and let the next iteration's window check
            // above catch a real close.
            let mut cookies: Vec<capture::CookiePair> = Vec::new();
            let mut poll_ok = true;
            for domain in &domains {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    window.cookies_for_url(domain.clone())
                }));
                match result {
                    Ok(Ok(found)) => cookies.extend(
                        found
                            .into_iter()
                            .map(|c| (c.name().to_string(), c.value().to_string())),
                    ),
                    _ => {
                        poll_ok = false;
                        break;
                    }
                }
            }

            if poll_ok {
                if let Some(value) = capture::evaluate_capture(spec, &cookies) {
                    return Some(value);
                }
            }
            if std::time::Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(std::time::Duration::from_millis(600));
        }
    })
    .await
    .map_err(|e| e.to_string())?;

    if let Some(win) = app.get_webview_window(&label) {
        let _ = win.close();
    }

    match captured {
        Some(value) => {
            let mut config = state.config.lock();
            match spec.source {
                "humble_bundle" => config.humble_cookie = Some(value),
                "packt" => config.packt_cookies = Some(value),
                "manning" => config.manning_cookies = Some(value),
                other => return Err(format!("no config field wired up for source '{other}'")),
            }
            config.save().map_err(err)?;
            Ok(CaptureResult { cancelled: false })
        }
        None => Ok(CaptureResult { cancelled: true }),
    }
}

/// Looks up authors/ISBN for every book missing either, via
/// `library_core::enrich`. Synchronous like `import_source` -- Tauri runs
/// commands off the main thread by default, and this can take a while for
/// a large library (paced network requests), so the frontend shows a
/// busy state while it runs rather than blocking the UI thread.
#[tauri::command]
pub fn enrich_metadata(state: State<AppState>) -> Result<EnrichSummary, String> {
    let db = state.db.lock();
    enrich::enrich_missing(&db).map_err(err)
}
