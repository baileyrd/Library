use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "library",
    about = "Track ebooks you own across Humble Bundle, Packt, Manning, Kindle (via CSV import), and manual entries, so you can spot duplicates before buying a book again."
)]
pub struct Cli {
    /// Override the SQLite database path (defaults to the config-resolved path).
    #[arg(long, global = true)]
    pub db: Option<PathBuf>,

    /// Print extra diagnostics to stderr.
    #[arg(short, long, global = true)]
    pub verbose: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Import books from a storefront and store/update them in the local database.
    Import {
        #[command(subcommand)]
        source: ImportSource,
    },

    /// Add a book manually.
    Add {
        #[arg(long)]
        title: String,

        /// Repeatable: --author "Name One" --author "Name Two"
        #[arg(long = "author")]
        author: Vec<String>,

        #[arg(long)]
        isbn: Option<String>,

        /// Repeatable: --format epub --format pdf
        #[arg(long = "format")]
        format: Vec<String>,
    },

    /// List books, optionally filtered by source.
    List {
        /// One of: humble_bundle, packt, manning, kindle, manual
        #[arg(long)]
        source: Option<String>,

        #[arg(long)]
        json: bool,
    },

    /// Check whether a title or ISBN looks like something you already own.
    Check {
        /// A title or an ISBN (10 or 13 digits, punctuation is stripped automatically).
        query: String,
    },

    /// Print per-source book counts and the total.
    Stats,

    /// View or update stored credentials/settings.
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },

    /// Delete a book by id.
    Remove { id: i64 },
}

#[derive(Subcommand)]
pub enum ImportSource {
    /// Import from Humble Bundle (requires `humble_cookie` in config).
    Humble,
    /// Import from Packt (requires `packt_token` in config).
    Packt,
    /// Import from Manning (requires `manning_cookies` in config).
    Manning,
    /// Import from a CSV file (e.g. an Amazon Kindle library export).
    /// Expected columns: title,authors,isbn,formats (authors/formats are
    /// `;`-separated; only title is required).
    Kindle {
        #[arg(long)]
        file: PathBuf,
    },
}

#[derive(Subcommand)]
pub enum ConfigAction {
    /// Update one or more config fields. Only the fields you pass are changed.
    Set {
        /// The `_simpleauth_sess` cookie value from humblebundle.com, obtained via
        /// your browser's devtools (Application/Storage -> Cookies).
        #[arg(long = "humble-cookie")]
        humble_cookie: Option<String>,

        /// A Packt API bearer token (JWT), obtained via your browser's devtools
        /// (Application/Storage -> Cookies/Local Storage, or the Network tab's
        /// request headers on services.packtpub.com). Packt username/password
        /// login is not performed by this tool.
        #[arg(long = "packt-token")]
        packt_token: Option<String>,

        /// The full manning.com + login.manning.com cookie jar string
        /// (semicolon-separated name=value pairs), obtained via your browser's
        /// devtools (Application/Storage -> Cookies).
        #[arg(long = "manning-cookies")]
        manning_cookies: Option<String>,
    },
}
