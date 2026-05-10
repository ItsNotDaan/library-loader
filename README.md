**NOTE: Yes, I used Claude for this fork. I dont know how to program in Rust but I know how the structures and program works. Work with the tools you have. Props to the original author!**

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

### The easy way — one script

Make sure you have [Homebrew](https://brew.sh) and [Rust](https://rustup.rs) installed. Then open a terminal in the `library-loader` folder (in Finder: right-click the folder → **New Terminal at Folder**) and run:

```shell
./setup.sh
```

This will:
1. Install any missing Homebrew dependencies (GTK3 etc.)
2. Build the release binary for your Mac (Apple Silicon or Intel)
3. Install **Library Loader** to `/Applications` with an icon, ready for Spotlight and Launchpad

If macOS shows an "unidentified developer" warning on first launch, right-click the app → **Open** → **Open** to bypass it once.

---

### Manual steps (if you prefer)

<details>
<summary>Expand manual instructions</summary>

**1. Install dependencies and build**

```shell
./macos-compile.sh
```

**2. Create the app bundle**

```shell
APP="/Applications/LibraryLoader.app"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"

# Apple Silicon:
cp target/aarch64-apple-darwin/release/library-loader-gui "$APP/Contents/MacOS/library-loader-gui"
# Intel: replace with target/x86_64-apple-darwin/release/library-loader-gui
```

**3. Create the launcher** — save this as `$APP/Contents/MacOS/LibraryLoader` and `chmod +x` it:

```shell
#!/bin/bash
export PATH="/opt/homebrew/bin:/opt/homebrew/sbin:$PATH"
export DYLD_FALLBACK_LIBRARY_PATH="/opt/homebrew/lib"
export XDG_DATA_DIRS="/opt/homebrew/share"
export GTK_DATA_PREFIX="/opt/homebrew"
export GTK_EXE_PREFIX="/opt/homebrew"
export GTK_PATH="/opt/homebrew"
export GDK_PIXBUF_MODULE_FILE="/opt/homebrew/lib/gdk-pixbuf-2.0/2.10.0/loaders.cache"
DIR="$(cd "$(dirname "$0")" && pwd)"
exec "$DIR/library-loader-gui"
```

</details>

---

### First launch

Sign in with your [componentsearchengine.com](https://componentsearchengine.com) credentials using the menu in the top-right corner. Your config is stored at:

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
