# Library Loader

A Rust desktop app that watches your Downloads folder for EDA component ZIP files from [ComponentSearchEngine (CSE)](https://componentsearchengine.com) and automatically imports them into your KiCad libraries.

> **This is a fork of [olback/library-loader](https://github.com/olback/library-loader), focused on macOS with a modernised UI and additional features.**

---

## What's new in this fork

The original project is a solid foundation but the UI was showing its age and macOS support was incomplete. This fork focuses on making Library Loader a first-class macOS app:

- **Modern UI** — complete visual overhaul with a Catppuccin Mocha dark theme
- **Transfer utility** — copy components between KiCad libraries directly from the app
- **Sign in / Sign out** — manage your ComponentSearchEngine credentials from the GUI, no manual config editing required
- **Import from website** — download a component on CSE, drop the ZIP in your watch folder, and it's imported automatically
- **Import from compressed folder** — manually import a `.zip` file containing a KiCad component package
- **System tray icon** — live indicator showing whether the watcher is active (green) or idle (blue)
- **macOS first** — built and tested on macOS; native file/folder pickers, correct library paths, Homebrew-based build

---

## Getting started

### 1. Build from source (macOS)

Required: `brew`, `rustc`, `cargo` (install Rust via [rustup](https://rustup.rs)).

```shell
./macos-compile.sh
```

Or manually:

```shell
cargo build --release
```

The GUI binary is at `target/release/library-loader-gui`.

### 2. First launch

Run the app and sign in with your [componentsearchengine.com](https://componentsearchengine.com) credentials using the menu in the top-right corner. Your config is stored at:

```
~/Library/Application Support/LibraryLoader.toml
```

You can also seed it manually from the example:

```shell
cp LibraryLoader.example.toml ~/Library/Application\ Support/LibraryLoader.toml
```

### 3. Configure your KiCad library folder

In the app settings, set your KiCad output folder. The app will create and maintain:

```
{output_dir}/
├── {lib_name}.kicad_sym
├── {lib_name}.pretty/
└── {lib_name}.3dshapes/
```

### 4. Import a component

**From the website:** download a component ZIP from CSE — if your Downloads folder is set as the watch path, it's imported automatically.

**Manually:** use the import button to select a `.zip` file directly.

**Transfer between libraries:** open the Transfer dialog from the menu to copy components from one KiCad library to another.

---

## Running

```shell
# GUI (recommended)
cargo run --bin library-loader-gui

# CLI
cargo run --bin library-loader-cli
```

---

## Why this fork?

The original library-loader fills a real gap — SamacSys only ships a Windows app, and KiCad users on macOS and Linux have no official tooling. This fork picks up where the original left off, with a focus on making it feel like a proper macOS app rather than a Linux port.

---

## Original project

[olback/library-loader](https://github.com/olback/library-loader) — all credit for the core architecture and CSE integration goes to the original author.

---

## License

[GNU Affero General Public License v3.0](LICENSE)
