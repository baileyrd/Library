# Library Desktop

A native desktop app (Tauri) for the `library` book inventory: browse/search your
library, add/edit/remove books, run imports, and manage source credentials — all
from a window instead of the command line.

It shares the same SQLite database and config file as the `library` CLI
(`crates/core`, aka `library-core`), so the two are interchangeable — import
from the CLI, browse in the app, or vice versa.

## Running it

You need the Tauri Linux prerequisites installed first (Debian/Ubuntu):

```
sudo apt install libwebkit2gtk-4.1-dev build-essential curl wget file \
  libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev
```

(macOS/Windows: see https://v2.tauri.app/start/prerequisites/ — no extra
packages needed beyond Xcode Command Line Tools / the MSVC build tools.)

Then, from the repo root:

```
cargo run -p library-desktop
```

This builds and launches the app directly — no Node.js/npm involved, since
the frontend (`desktop/frontend/`) is plain HTML/CSS/JS with no build step,
loaded via Tauri's `withGlobalTauri` bridge (`window.__TAURI__.core.invoke`).

For a hot-reloading dev loop or to build installers (`.deb`/`.AppImage`/`.dmg`/
`.msi`), install the Tauri CLI and use it instead:

```
cargo install tauri-cli --version "^2"
cargo tauri dev      # hot reload (frontend is static, so this mainly matters for Rust changes)
cargo tauri build     # produces an installer/bundle under desktop/src-tauri/target/release/bundle/
```

## What it can do

- **Books tab**: list/filter your library by source, add a book, edit or
  remove an existing one.
- **Check before buying**: search by title or ISBN before you buy — same
  duplicate-detection logic as `library check` on the CLI.
- **Import tab**: trigger `import humble` / `import packt` / `import manning`
  (using credentials from Settings) or `import kindle` (pick a CSV file path).
- **Settings tab**: set/update the Humble Bundle cookie, Packt token, and
  Manning cookie jar — same config file the CLI reads
  (`~/.config/library-inventory/config.toml`, `0600` permissions). See the
  root README for how to obtain each one from your browser.

## Notes

- The window itself has no menu bar/icon polish beyond a placeholder icon
  (`src-tauri/icons/icon.png`) — swap it for a real one if you build
  installers for distribution.
- Rust command handlers in `src-tauri/src/commands.rs` are thin wrappers
  around `library-core` — the actual dedup/parsing/storage logic lives there
  and is shared (and unit-tested) with the CLI.
