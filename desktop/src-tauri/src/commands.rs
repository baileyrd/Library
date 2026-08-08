use serde::Serialize;
use tauri::State;

use library_core::dedup::{self, DedupMatch};
use library_core::model::{Book, Source};
use library_core::sources::{self, Source as SourceFetcher};

use crate::state::AppState;

/// Fuzzy matches below this confidence, but at or above this one, are
/// returned separately as weaker, manually-reviewed candidates -- mirrors
/// the CLI's `check` command threshold.
const CHECK_WEAK_THRESHOLD: f64 = 0.75;

fn err(e: anyhow::Error) -> String {
    e.to_string()
}

#[tauri::command]
pub fn list_books(state: State<AppState>, source: Option<String>) -> Result<Vec<Book>, String> {
    let source_filter = source.map(|s| s.parse::<Source>()).transpose().map_err(err)?;
    let db = state.db.lock().unwrap();
    db.list_books(source_filter).map_err(err)
}

#[tauri::command]
pub fn get_book(state: State<AppState>, id: i64) -> Result<Option<Book>, String> {
    let db = state.db.lock().unwrap();
    db.get_book(id).map_err(err)
}

#[tauri::command]
pub fn add_book(
    state: State<AppState>,
    title: String,
    authors: Vec<String>,
    isbn: Option<String>,
    formats: Vec<String>,
) -> Result<AddBookResult, String> {
    let db = state.db.lock().unwrap();
    let existing = db.all_books().map_err(err)?;
    let candidate = sources::manual::build_manual_book(title, authors, isbn, formats);
    let warnings = dedup::find_duplicates(&existing, &candidate);

    let outcome = db.upsert_book(&candidate).map_err(err)?;
    let id = match outcome {
        library_core::db::UpsertOutcome::Inserted(id) => id,
        library_core::db::UpsertOutcome::Updated(id) => id,
    };
    let book = db.get_book(id).map_err(err)?.ok_or_else(|| "book vanished after insert".to_string())?;

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
) -> Result<Book, String> {
    let db = state.db.lock().unwrap();
    let changed = db
        .update_book(id, &title, &authors, isbn.as_deref(), &formats)
        .map_err(err)?;
    if !changed {
        return Err(format!("no book with id {id}"));
    }
    db.get_book(id).map_err(err)?.ok_or_else(|| format!("no book with id {id}"))
}

#[tauri::command]
pub fn remove_book(state: State<AppState>, id: i64) -> Result<bool, String> {
    let db = state.db.lock().unwrap();
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
    let isbn = if digits.len() == 10 || digits.len() == 13 { Some(digits) } else { None };

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
    };

    let db = state.db.lock().unwrap();
    let existing = db.all_books().map_err(err)?;
    let matches = dedup::find_duplicates_with_threshold(&existing, &candidate, CHECK_WEAK_THRESHOLD);
    let (strong, weak): (Vec<_>, Vec<_>) = matches.into_iter().partition(|m| m.confidence >= 0.90);

    Ok(CheckResult { strong, weak })
}

#[tauri::command]
pub fn stats(state: State<AppState>) -> Result<Vec<(Source, i64)>, String> {
    let db = state.db.lock().unwrap();
    db.stats().map_err(err)
}

#[derive(Serialize)]
pub struct ConfigStatus {
    humble_cookie_set: bool,
    packt_token_set: bool,
    manning_cookies_set: bool,
    db_path: String,
}

#[tauri::command]
pub fn get_config_status(state: State<AppState>) -> ConfigStatus {
    let config = state.config.lock().unwrap();
    ConfigStatus {
        humble_cookie_set: config.humble_cookie.is_some(),
        packt_token_set: config.packt_token.is_some(),
        manning_cookies_set: config.manning_cookies.is_some(),
        db_path: config.resolve_db_path().display().to_string(),
    }
}

#[tauri::command]
pub fn set_credential(state: State<AppState>, field: String, value: String) -> Result<(), String> {
    let mut config = state.config.lock().unwrap();
    match field.as_str() {
        "humble_cookie" => config.humble_cookie = Some(value),
        "packt_token" => config.packt_token = Some(value),
        "manning_cookies" => config.manning_cookies = Some(value),
        other => return Err(format!("unknown credential field '{other}'")),
    }
    config.save().map_err(err)
}

#[derive(Serialize)]
pub struct ImportSummary {
    source: String,
    new_count: i64,
    updated_count: i64,
    warnings: Vec<String>,
}

#[tauri::command]
pub fn import_source(state: State<AppState>, source: String, file: Option<String>) -> Result<ImportSummary, String> {
    let fetcher: Box<dyn SourceFetcher> = match source.as_str() {
        "humble" => {
            let cookie = {
                let config = state.config.lock().unwrap();
                config.humble_cookie.clone()
            }
            .ok_or_else(|| "no Humble Bundle cookie configured".to_string())?;
            Box::new(sources::humble::Humble { cookie })
        }
        "packt" => {
            let token = {
                let config = state.config.lock().unwrap();
                config.packt_token.clone()
            }
            .ok_or_else(|| "no Packt token configured".to_string())?;
            Box::new(sources::packt::Packt { token })
        }
        "manning" => {
            let cookies = {
                let config = state.config.lock().unwrap();
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
    let db = state.db.lock().unwrap();
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

    Ok(ImportSummary { source, new_count, updated_count, warnings })
}
