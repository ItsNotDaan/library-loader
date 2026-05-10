# Library Loader — Claude Context

## Project overview

A Rust desktop app that watches a downloads folder for EDA component ZIP files from
[ComponentSearchEngine (CSE)](https://componentsearchengine.com) and imports them into
KiCad library folders. There is also a GUI-based Transfer feature for copying components
between KiCad libraries.

## Workspace layout

```
library-loader/
├── ll-core/          # Library: all download/extract/save logic (no UI)
├── ll-cli/           # CLI binary wrapping ll-core
└── ll-gui/           # GTK3 GUI binary (the main app used by the user)
    ├── src/
    │   ├── main.rs           # GTK app init, tray icon, startup
    │   ├── ui/mod.rs         # All UI logic — wiring Glade widgets + handlers
    │   ├── ui/event.rs       # UiEvent enum used to talk from threads → GTK
    │   ├── ui/logger.rs      # GuiLogger — pipes ll-core log messages into the text view
    │   ├── macros.rs         # get_obj! and resource! macros
    │   └── consts.rs         # Embedded resource bytes
    ├── assets/
    │   ├── library-loader.glade  # ALL widget layout — edit this for UI changes
    │   └── app.css               # Catppuccin Mocha dark theme styles
    └── Cargo.toml
```

## Build

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo build           # debug build of entire workspace
cargo build --release # release build
```

The GUI binary is `target/debug/library-loader-gui`.
Requires GTK3 dev libraries (already present on this machine via Homebrew/macOS).

## How CSE import works (important — non-obvious)

The `.zip` file downloaded from CSE is **not** a simple archive of KiCad files.
It contains a small `.epw` metadata file with a numeric component ID.
`ll-core` reads that ID, then makes an authenticated HTTP request to
`https://componentsearchengine.com/ga/model.php?partID={id}` using the user's
credentials (Basic Auth, base64 of `username:password`). The server streams back
the actual KiCad package.

→ **ServerError(401)** means the stored username/password was rejected. Fix: sign out
and sign back in. The `try_auth` URL check (`partID=` empty) does not validate
credentials reliably — it can return 200 even with wrong credentials.

## KiCad library folder structure

All three artefacts share **one parent folder** and the **same lib name prefix**:

```
{output_dir}/
├── {lib_name}.kicad_sym          # symbols (all components appended into one file)
├── {lib_name}.pretty/
│   └── {component}.kicad_mod    # footprints, one file per component
└── {lib_name}.3dshapes/
    └── {component}.stp          # 3D models, flat — NO per-component subfolders
```

The lib name comes from the HashMap key in `config.formats` (e.g. `"LibLoader"`).
There is **no separate D3 output path** — 3D files go into `.3dshapes/` alongside
`.pretty/` inside the same KiCad output folder.

## Config file

Stored at `~/Library/Application Support/LibraryLoader.toml` (macOS default).
The GUI reads and writes it on every change. Relevant excerpt for KiCad-only setup:

```toml
[settings]
watch_path = "~/Downloads"
recursive = false

[formats.LibLoader]   # key becomes the lib name prefix
format = "kicad"
output_path = "/Users/you/Documents/KiCad Lib/LibLoader"

[profile]
username = "cse_username"
password = "cse_password"
```

**Do not add a `[formats.*]` entry with `format = "3d"`** — the KiCad extractor
already writes `.stp`/`.wrl`/`.step` into `.3dshapes/`. A separate D3 entry causes
duplicate files and stray per-component subfolders. The GUI strips any D3 entries
from the config on startup (migration code in `ui/mod.rs` near the event loop setup).

## UI architecture

- **Glade file** (`assets/library-loader.glade`) defines all widgets. Widget IDs are
  the only coupling between Glade and Rust — keep them in sync.
- **`Ui` struct** (`ui/mod.rs`) holds `Rc<>` references to every widget that needs
  runtime interaction. Built in `Ui::new`, never mutated structurally after that.
- **`UiEvent` channel** (`ui/event.rs`): background threads send `UiEvent` values;
  `rx.attach(None, ...)` processes them on the GTK main thread.
- **Stack pages**: `"spinner"`, `"login"`, `"watch"`, `"transfer"` — switched via
  `UiEvent::SwitchStack`.
- **Watcher active flag**: `Rc<Cell<bool>>` shared between `Ui` and the tray polling
  closure. Both run on GTK's main thread so `Rc` (not `Arc`) is fine.

## Tray icon

Uses the `tray-icon = "0.24"` crate. Polled every 100 ms via `glib::timeout_add_local`.
- Blue circle → watcher inactive
- Green circle → watcher active
- Left-click → show/present window
- Menu "Exit Library Loader" → `app.quit()`

Color constants (Catppuccin Mocha):
- Blue:  `(0x89, 0xb4, 0xfa)`
- Green: `(0xa6, 0xe3, 0xa1)`

## Popover menu auth state

The hamburger menu has two mutually exclusive buttons managed by `set_auth_state(bool)`:
- Logged out: "Sign In" visible, "Sign Out" + "Transfer Components…" hidden
- Logged in:  "Sign Out" + "Transfer Components…" visible, "Sign In" hidden

`set_auth_state` is called from: `SetProfile` event, sign-out handler, initial state check.

## Transfer feature

Copies components between KiCad libraries. Source = configured KiCad output path.
`do_transfer(component, source_path, source_lib, dest_path)` copies:
1. `{source_path}/{source_lib}.pretty/{component}.kicad_mod` → `{dest_path}/{dest_lib}.pretty/`
2. Symbol block from `{source_lib}.kicad_sym` → `{dest_lib}.kicad_sym` (created if absent)
3. All `{source_path}/{source_lib}.3dshapes/{component}.*` → `{dest_path}/{dest_lib}.3dshapes/`

Component name = footprint stem (from `.kicad_mod` filename).
Symbol is found by searching the `.kicad_sym` file for `"{lib_name}:{component}"` in
Footprint property, then walking back to the enclosing `(symbol ...)` block.

List box selection: click = single select, shift+click = range. Implemented via
`connect_button_press_event` that calls `list.unselect_all()` when Shift is not held.

## Key ll-core files

| File | Purpose |
|------|---------|
| `ll-core/src/watcher/mod.rs` | File watcher + `pub fn import_file(path, config)` |
| `ll-core/src/format/extractors/kicad.rs` | KiCad extractor — writes `.kicad_sym`, `.pretty/`, `.3dshapes/` |
| `ll-core/src/cse/mod.rs` | HTTP call to CSE API, unzips response |
| `ll-core/src/epw.rs` | Reads `.epw` metadata from the downloaded zip |
| `ll-core/src/config/profile.rs` | `token()` = base64 of `user:pass`, `try_auth()` |
| `ll-core/src/error.rs` | `Error` enum — use `{}` format (Display) not `{:?}` (Debug) |

## CSS classes (Catppuccin Mocha)

Defined in `assets/app.css`. Notable ones:
- `.primary-btn` — blue filled button; requires `button.primary-btn label { color: #1e1e2e }` to override GTK's inner-label colour
- `.card` — dark rounded frame used for sections
- `.chip-active` — static "KiCad" pill badge
- `.folder-btn` — transparent icon button for browse actions
- `.section-heading` — uppercase dimmed section titles

## Gotchas

- `FileChooserNative` is used everywhere for file/folder pickers — gives the native
  macOS sheet instead of the GTK dialog. No `add_button()` — pass accept/cancel labels
  to the constructor. `add_filter()` takes the filter by value (not `&filter`).
- `glib::Propagation::Proceed` replaces `gtk::Inhibit(false)` in gtk-rs 0.18 event handlers.
- `connect_button_press_event` closure must return `glib::Propagation`, not `bool`.
- All `Watcher` operations that need to report back to the UI must go through the
  `UiEvent` channel — never touch GTK widgets from a non-main thread.
- The KiCad extractor returns an empty `Files::new()` map — it writes files directly
  to disk rather than going through the normal `Result::save()` path.
- The extractor reads the existing `.kicad_sym` content before appending, and skips
  any symbol whose name already appears in the file. This prevents duplicates which
  would cause KiCad to silently refuse to load the library.
