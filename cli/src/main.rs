mod cli;

use anyhow::{bail, Context, Result};
use clap::Parser;

use cli::{Cli, Command, ConfigAction, ImportSource};
use library_core::config::Config;
use library_core::db::{Db, UpsertOutcome};
use library_core::dedup;
use library_core::model::{Book, Source};
use library_core::sources;
use library_core::sources::Source as SourceFetcher;

/// Fuzzy matches below this confidence, but at or above this one, are shown
/// separately in `check` output as weaker, manually-reviewed candidates.
/// See `desktop/src-tauri/src/commands.rs`'s copy of this constant for why
/// it's set equal to the strong-match cutoff.
const CHECK_WEAK_THRESHOLD: f64 = 0.90;

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err}");
        for cause in err.chain().skip(1) {
            eprintln!("  caused by: {cause}");
        }
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let config = Config::load().context("failed to load config")?;
    let db_path = cli.db.clone().unwrap_or_else(|| config.resolve_db_path());
    let db = Db::open(&db_path)
        .with_context(|| format!("failed to open database at {}", db_path.display()))?;

    match cli.command {
        Command::Import { source } => handle_import(&db, &config, source, cli.verbose),
        Command::Add {
            title,
            author,
            isbn,
            format,
            cover_url,
        } => handle_add(&db, title, author, isbn, format, cover_url),
        Command::List { source, json } => handle_list(&db, source, json),
        Command::Check { query } => handle_check(&db, &query),
        Command::CheckBundle {
            url,
            exclude_fiction,
        } => handle_check_bundle(&db, &url, exclude_fiction),
        Command::CheckBundles { exclude_fiction } => {
            handle_check_bundles(&db, &config, exclude_fiction)
        }
        Command::Stats => handle_stats(&db),
        Command::Enrich => handle_enrich(&db),
        Command::Config { action } => handle_config(action),
        Command::Remove { id } => handle_remove(&db, id),
    }
}

fn handle_import(db: &Db, config: &Config, source: ImportSource, verbose: bool) -> Result<()> {
    let fetcher: Box<dyn SourceFetcher> = match source {
        ImportSource::Humble => {
            let cookie = config.humble_cookie.clone().ok_or_else(|| {
                anyhow::anyhow!(
                    "no Humble Bundle cookie configured. Run: library config set --humble-cookie <value>"
                )
            })?;
            Box::new(sources::humble::Humble { cookie })
        }
        ImportSource::Packt => {
            let cookies = config.packt_cookies.clone().ok_or_else(|| {
                anyhow::anyhow!(
                    "no Packt cookies configured. Run: library config set --packt-cookies <value>"
                )
            })?;
            Box::new(sources::packt::Packt { cookies })
        }
        ImportSource::Manning => {
            let cookies = config.manning_cookies.clone().ok_or_else(|| {
                anyhow::anyhow!(
                    "no Manning cookies configured. Run: library config set --manning-cookies <value>"
                )
            })?;
            Box::new(sources::manning::Manning { cookies })
        }
        ImportSource::Kindle { file } => Box::new(sources::kindle::Kindle { path: file }),
    };

    if verbose {
        eprintln!("fetching from {}...", fetcher.name());
    }
    let books = fetcher.fetch()?;
    run_import(db, fetcher.name(), books)
}

fn run_import(db: &Db, source_name: &str, books: Vec<Book>) -> Result<()> {
    let baseline = db
        .all_books()
        .context("failed to load existing books for dedup check")?;

    let mut new_count = 0;
    let mut updated_count = 0;

    for book in books {
        let outcome = db.upsert_book(&book)?;
        match outcome {
            UpsertOutcome::Inserted(_) => {
                new_count += 1;
                warn_cross_source_duplicates(&baseline, &book);
            }
            UpsertOutcome::Updated(_) => {
                updated_count += 1;
            }
        }
    }

    println!("{source_name}: {new_count} new, {updated_count} updated");
    Ok(())
}

fn warn_cross_source_duplicates(existing: &[Book], candidate: &Book) {
    for m in dedup::find_duplicates(existing, candidate) {
        if m.book.source == candidate.source {
            continue;
        }
        println!(
            "\u{26a0} '{}' from {} looks like a possible duplicate of '{}' already in your library from {} (confidence {:.2}, reason: {})",
            candidate.title,
            candidate.source,
            m.book.title,
            m.book.source,
            m.confidence,
            m.reason
        );
    }
}

fn handle_add(
    db: &Db,
    title: String,
    authors: Vec<String>,
    isbn: Option<String>,
    formats: Vec<String>,
    cover_url: Option<String>,
) -> Result<()> {
    let book = sources::manual::build_manual_book(title, authors, isbn, formats, cover_url);

    let existing = db
        .all_books()
        .context("failed to load existing books for dedup check")?;
    for m in dedup::find_duplicates(&existing, &book) {
        println!(
            "\u{26a0} possible duplicate of '{}' from {} (confidence {:.2}, reason: {})",
            m.book.title, m.book.source, m.confidence, m.reason
        );
    }

    let outcome = db.upsert_book(&book)?;
    match outcome {
        UpsertOutcome::Inserted(id) => println!("added '{}' as book #{id}", book.title),
        UpsertOutcome::Updated(id) => println!("updated '{}' as book #{id}", book.title),
    }
    Ok(())
}

fn handle_list(db: &Db, source: Option<String>, json: bool) -> Result<()> {
    let source_filter = source.map(|s| s.parse::<Source>()).transpose()?;

    // Cross-source duplicates need the whole library to detect, even when
    // only one source's books are being displayed (e.g. `list --source
    // packt` should still flag a book also owned via Humble Bundle).
    let all_books = db.all_books()?;
    let dup_sources = dedup::cross_source_duplicates(&all_books);
    let books: Vec<Book> = match source_filter {
        Some(filter) => all_books
            .into_iter()
            .filter(|b| b.source == filter)
            .collect(),
        None => all_books,
    };

    if json {
        let entries = books
            .iter()
            .map(|book| {
                let mut value = serde_json::to_value(book)?;
                let duplicate_sources: Vec<&str> = book
                    .id
                    .and_then(|id| dup_sources.get(&id))
                    .map(|sources| sources.iter().map(Source::as_str).collect())
                    .unwrap_or_default();
                if let serde_json::Value::Object(map) = &mut value {
                    map.insert(
                        "duplicate_sources".to_string(),
                        serde_json::json!(duplicate_sources),
                    );
                }
                Ok(value)
            })
            .collect::<Result<Vec<serde_json::Value>>>()?;
        println!("{}", serde_json::to_string_pretty(&entries)?);
        return Ok(());
    }

    if books.is_empty() {
        println!("no books found");
        return Ok(());
    }

    println!(
        "{:<6} {:<50} {:<14} {:<20} FORMATS",
        "ID", "TITLE", "SOURCE", "AUTHORS"
    );
    for book in &books {
        let id = book.id.map(|i| i.to_string()).unwrap_or_default();
        let authors = book.authors.join(", ");
        let formats = book.formats.join(",");
        let dup_note = book
            .id
            .and_then(|id| dup_sources.get(&id))
            .map(|sources| {
                format!(
                    "  [also owned via: {}]",
                    sources
                        .iter()
                        .map(Source::as_str)
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })
            .unwrap_or_default();
        println!(
            "{:<6} {:<50} {:<14} {:<20} {}{}",
            id,
            truncate(&book.title, 50),
            book.source.to_string(),
            truncate(&authors, 20),
            formats,
            dup_note
        );
    }
    Ok(())
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        s.chars().take(max.saturating_sub(1)).collect::<String>() + "\u{2026}"
    }
}

fn handle_check(db: &Db, query: &str) -> Result<()> {
    let digits: String = query.chars().filter(|c| c.is_ascii_digit()).collect();
    let isbn = if digits.len() == 10 || digits.len() == 13 {
        Some(digits)
    } else {
        None
    };

    let candidate = Book {
        id: None,
        title: query.to_string(),
        authors: Vec::new(),
        isbn,
        source: Source::Manual,
        source_id: None,
        formats: Vec::new(),
        acquired_at: None,
        raw_json: None,
        cover_url: None,
    };

    let existing = db.all_books().context("failed to load existing books")?;
    let matches =
        dedup::find_duplicates_with_threshold(&existing, &candidate, CHECK_WEAK_THRESHOLD);

    let (strong, weak): (Vec<_>, Vec<_>) = matches.into_iter().partition(|m| m.confidence >= 0.90);

    if strong.is_empty() && weak.is_empty() {
        println!("No matches found \u{2014} looks safe to buy.");
        return Ok(());
    }

    if !strong.is_empty() {
        println!("Likely matches:");
        for m in &strong {
            println!(
                "  [{:.2}] '{}' ({}) \u{2014} {}",
                m.confidence, m.book.title, m.book.source, m.reason
            );
        }
    }

    if !weak.is_empty() {
        println!("Weaker matches (review manually):");
        for m in &weak {
            println!(
                "  [{:.2}] '{}' ({}) \u{2014} {}",
                m.confidence, m.book.title, m.book.source, m.reason
            );
        }
    }

    Ok(())
}

fn handle_check_bundle(db: &Db, url: &str, exclude_fiction: bool) -> Result<()> {
    let mut contents = sources::humble::fetch_bundle_contents(url)?;
    if exclude_fiction {
        contents
            .items
            .retain(|item| !sources::humble::is_fiction_or_comic(&item.title));
    }
    let existing = db.all_books().context("failed to load existing books")?;
    println!("{}", contents.bundle_name);
    print_bundle_check(&existing, &contents);
    Ok(())
}

fn handle_check_bundles(db: &Db, config: &Config, exclude_fiction: bool) -> Result<()> {
    let mut checks = sources::humble::fetch_all_active_bundles()?;
    let discovered = checks.len();
    checks.retain(|check| match &check.result {
        Ok(contents) => !sources::humble::matches_excluded_bundle(
            &contents.bundle_name,
            &config.bundle_exclude_terms,
        ),
        Err(_) => true,
    });
    let excluded = discovered - checks.len();
    let existing = db.all_books().context("failed to load existing books")?;

    if excluded > 0 {
        println!(
            "{} bundles currently on humblebundle.com/books ({excluded} excluded by your bundle-exclude terms)",
            checks.len()
        );
    } else {
        println!(
            "{} bundles currently on humblebundle.com/books",
            checks.len()
        );
    }
    for check in &mut checks {
        match &mut check.result {
            Ok(contents) => {
                if exclude_fiction {
                    contents
                        .items
                        .retain(|item| !sources::humble::is_fiction_or_comic(&item.title));
                }
                println!("\n{} ({})", contents.bundle_name, check.url);
                print_bundle_check(&existing, contents);
            }
            Err(e) => println!("\n{} \u{2014} error: {e}", check.url),
        }
    }
    Ok(())
}

/// Scores every item in `contents` against `existing` and prints one line
/// per book plus an owned-count summary -- shared by `handle_check_bundle`
/// and `handle_check_bundles` (one bundle vs. every bundle currently on
/// sale) so both print identically.
fn print_bundle_check(existing: &[Book], contents: &sources::humble::BundleContents) {
    let mut owned_count = 0;
    for item in &contents.items {
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

        if let Some(m) = strong.first() {
            owned_count += 1;
            println!(
                "  [owned] {} \u{2014} matches '{}' from {} (confidence {:.2})",
                item.title, m.book.title, m.book.source, m.confidence
            );
        } else if let Some(m) = weak.first() {
            println!(
                "  [maybe] {} \u{2014} possible match to '{}' from {} (confidence {:.2})",
                item.title, m.book.title, m.book.source, m.confidence
            );
        } else {
            println!("  [new]   {}", item.title);
        }
    }
    println!(
        "{owned_count} of {} books look like ones you already own",
        contents.items.len()
    );
}

fn handle_stats(db: &Db) -> Result<()> {
    let stats = db.stats()?;
    let total: i64 = stats.iter().map(|(_, count)| count).sum();

    if stats.is_empty() {
        println!("no books in your library yet");
        return Ok(());
    }

    for (source, count) in &stats {
        println!("{source:<14} {count}");
    }
    println!("{:<14} {}", "total", total);
    Ok(())
}

fn handle_enrich(db: &Db) -> Result<()> {
    let summary = library_core::enrich::enrich_missing(db)?;
    if summary.checked == 0 {
        println!("nothing missing authors or an ISBN");
        return Ok(());
    }
    println!(
        "checked {}, updated {}, no match {}, errors {}",
        summary.checked, summary.updated, summary.not_found, summary.errors
    );
    Ok(())
}

fn handle_config(action: ConfigAction) -> Result<()> {
    match action {
        ConfigAction::Set {
            humble_cookie,
            packt_cookies,
            manning_cookies,
        } => handle_config_set(humble_cookie, packt_cookies, manning_cookies),
        ConfigAction::BundleExcludeAdd { term } => handle_bundle_exclude_add(term),
        ConfigAction::BundleExcludeRemove { term } => handle_bundle_exclude_remove(term),
        ConfigAction::BundleExcludeList => handle_bundle_exclude_list(),
    }
}

fn handle_config_set(
    humble_cookie: Option<String>,
    packt_cookies: Option<String>,
    manning_cookies: Option<String>,
) -> Result<()> {
    if humble_cookie.is_none() && packt_cookies.is_none() && manning_cookies.is_none() {
        bail!("nothing to set: pass at least one of --humble-cookie, --packt-cookies, --manning-cookies");
    }

    let mut config = Config::load()?;
    let mut changed: Vec<&str> = Vec::new();

    if let Some(value) = humble_cookie {
        config.humble_cookie = Some(value);
        changed.push("humble_cookie");
    }
    if let Some(value) = packt_cookies {
        config.packt_cookies = Some(value);
        changed.push("packt_cookies");
    }
    if let Some(value) = manning_cookies {
        config.manning_cookies = Some(value);
        changed.push("manning_cookies");
    }

    config.save()?;
    println!(
        "updated: {} (file permissions set to 0600)",
        changed.join(", ")
    );
    Ok(())
}

fn handle_bundle_exclude_add(term: String) -> Result<()> {
    let trimmed = term.trim();
    if trimmed.is_empty() {
        bail!("exclude term must not be empty");
    }

    let mut config = Config::load()?;
    if config
        .bundle_exclude_terms
        .iter()
        .any(|t| t.eq_ignore_ascii_case(trimmed))
    {
        println!("'{trimmed}' is already in the bundle exclude list");
        return Ok(());
    }
    config.bundle_exclude_terms.push(trimmed.to_string());
    config.save()?;
    println!(
        "added '{trimmed}' to the bundle exclude list ({} total)",
        config.bundle_exclude_terms.len()
    );
    Ok(())
}

fn handle_bundle_exclude_remove(term: String) -> Result<()> {
    let mut config = Config::load()?;
    let before = config.bundle_exclude_terms.len();
    config
        .bundle_exclude_terms
        .retain(|t| !t.eq_ignore_ascii_case(term.trim()));

    if config.bundle_exclude_terms.len() == before {
        println!("'{term}' was not in the bundle exclude list");
        return Ok(());
    }
    config.save()?;
    println!(
        "removed '{term}' from the bundle exclude list ({} remaining)",
        config.bundle_exclude_terms.len()
    );
    Ok(())
}

fn handle_bundle_exclude_list() -> Result<()> {
    let config = Config::load()?;
    if config.bundle_exclude_terms.is_empty() {
        println!("no bundle exclude terms configured");
    } else {
        for term in &config.bundle_exclude_terms {
            println!("{term}");
        }
    }
    Ok(())
}

fn handle_remove(db: &Db, id: i64) -> Result<()> {
    if db.delete_book(id)? {
        println!("removed book #{id}");
    } else {
        println!("no book with id {id}");
    }
    Ok(())
}
