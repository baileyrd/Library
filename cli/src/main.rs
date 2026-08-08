mod cli;

use anyhow::{bail, Context, Result};
use clap::Parser;

use cli::{Cli, Command, ConfigAction, ImportSource};
use library_core::config::Config;
use library_core::db::{Db, UpsertOutcome};
use library_core::model::{Book, Source};
use library_core::sources;
use library_core::sources::Source as SourceFetcher;
use library_core::dedup;

/// Fuzzy matches below this confidence, but at or above this one, are shown
/// separately in `check` output as weaker, manually-reviewed candidates.
const CHECK_WEAK_THRESHOLD: f64 = 0.75;

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
        Command::Add { title, author, isbn, format } => handle_add(&db, title, author, isbn, format),
        Command::List { source, json } => handle_list(&db, source, json),
        Command::Check { query } => handle_check(&db, &query),
        Command::Stats => handle_stats(&db),
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
            let token = config.packt_token.clone().ok_or_else(|| {
                anyhow::anyhow!("no Packt token configured. Run: library config set --packt-token <value>")
            })?;
            Box::new(sources::packt::Packt { token })
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
    let baseline = db.all_books().context("failed to load existing books for dedup check")?;

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
) -> Result<()> {
    let book = sources::manual::build_manual_book(title, authors, isbn, formats);

    let existing = db.all_books().context("failed to load existing books for dedup check")?;
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
    let books = db.list_books(source_filter)?;

    if json {
        println!("{}", serde_json::to_string_pretty(&books)?);
        return Ok(());
    }

    if books.is_empty() {
        println!("no books found");
        return Ok(());
    }

    println!("{:<6} {:<50} {:<14} {:<20} {}", "ID", "TITLE", "SOURCE", "AUTHORS", "FORMATS");
    for book in &books {
        let id = book.id.map(|i| i.to_string()).unwrap_or_default();
        let authors = book.authors.join(", ");
        let formats = book.formats.join(",");
        println!(
            "{:<6} {:<50} {:<14} {:<20} {}",
            id,
            truncate(&book.title, 50),
            book.source.to_string(),
            truncate(&authors, 20),
            formats
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
    };

    let existing = db.all_books().context("failed to load existing books")?;
    let matches = dedup::find_duplicates_with_threshold(&existing, &candidate, CHECK_WEAK_THRESHOLD);

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

fn handle_config(action: ConfigAction) -> Result<()> {
    let ConfigAction::Set { humble_cookie, packt_token, manning_cookies } = action;

    if humble_cookie.is_none() && packt_token.is_none() && manning_cookies.is_none() {
        bail!("nothing to set: pass at least one of --humble-cookie, --packt-token, --manning-cookies");
    }

    let mut config = Config::load()?;
    let mut changed: Vec<&str> = Vec::new();

    if let Some(value) = humble_cookie {
        config.humble_cookie = Some(value);
        changed.push("humble_cookie");
    }
    if let Some(value) = packt_token {
        config.packt_token = Some(value);
        changed.push("packt_token");
    }
    if let Some(value) = manning_cookies {
        config.manning_cookies = Some(value);
        changed.push("manning_cookies");
    }

    config.save()?;
    println!("updated: {} (file permissions set to 0600)", changed.join(", "));
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
