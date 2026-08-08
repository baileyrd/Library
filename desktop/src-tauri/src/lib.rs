mod commands;
mod state;

use tauri::Manager;

use state::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let state = AppState::init().map_err(|e| e.to_string())?;
            app.manage(state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_books,
            commands::get_book,
            commands::add_book,
            commands::update_book,
            commands::remove_book,
            commands::check_duplicates,
            commands::stats,
            commands::get_config_status,
            commands::set_credential,
            commands::import_source,
            commands::capture_credential,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
