use {
    crate::{get_obj, resource},
    event::UiEvent,
    gtk::{
        glib::{self, clone},
        prelude::*,
        AboutDialog, Align, Application, ApplicationWindow, Builder, Button, CheckButton, Entry,
        FileChooserAction, FileChooserNative, HeaderBar, InfoBar, Label, ListBox, MessageType,
        ResponseType, Stack, Switch, TextBuffer,
    },
    ll_core::{Config, Format, Watcher, ECAD},
    logger::GuiLogger,
    std::{
        cell::{Cell, RefCell},
        path::{Path, PathBuf},
        rc::Rc,
        thread,
    },
};

mod event;
mod logger;

pub struct Ui {
    config: Rc<RefCell<Config>>,
    info_bar: InfoBar,
    info_bar_label: Label,
    main_stack: Stack,
    username_entry: Entry,
    password_entry: Entry,
    login_button: Button,
    info_bar_update: InfoBar,
    watcher_switch: Switch,
    watcher_active_indicator: Label,
    watching_status_label: Label,
    user_name_label: Label,
    kicad_symbols_entry: Entry,
    kicad_footprints_entry: Entry,
    // Transfer page
    transfer_source_label: Label,
    transfer_dest_entry: Entry,
    transfer_component_list: ListBox,
    // Popover auth-state buttons
    popover_sign_in: Button,
    popover_sign_out: Button,
    popover_transfer: Button,
    // Shared state for tray icon colour
    watcher_active_flag: Rc<Cell<bool>>,
    tx: glib::Sender<UiEvent>,
    watcher: RefCell<Option<Watcher>>,
}

// ── Config helpers ────────────────────────────────────────────────────────────

fn find_kicad_key(config: &Config) -> Option<String> {
    config
        .formats
        .iter()
        .find(|(_, f)| f.format == ECAD::KiCad)
        .map(|(k, _)| k.clone())
}

fn kicad_path(config: &Config) -> String {
    find_kicad_key(config)
        .and_then(|k| config.formats.get(&k))
        .map(|f| f.output_path.clone())
        .unwrap_or_default()
}

fn set_kicad_path(config: &mut Config, path: String) {
    let key = find_kicad_key(config).unwrap_or_else(|| "KiCad".to_string());
    config
        .formats
        .entry(key)
        .and_modify(|f| f.output_path = path.clone())
        .or_insert(Format { format: ECAD::KiCad, output_path: path });
}


// ── Transfer helpers ──────────────────────────────────────────────────────────

/// List footprint names (.kicad_mod stems) from `{output_path}/{lib_name}.pretty/`.
fn list_kicad_components(output_path: &str, lib_name: &str) -> Vec<String> {
    let pretty = PathBuf::from(output_path).join(format!("{}.pretty", lib_name));
    if !pretty.is_dir() {
        return vec![];
    }
    let mut names: Vec<String> = std::fs::read_dir(&pretty)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            if p.extension()?.to_str()? == "kicad_mod" {
                p.file_stem()?.to_str().map(|s| s.to_string())
            } else {
                None
            }
        })
        .collect();
    names.sort_by(|a, b| a.to_lowercase().cmp(&b.to_lowercase()));
    names
}

/// Populate the transfer ListBox with component names.
fn populate_component_list(list: &ListBox, output_path: &str, lib_name: &str) {
    for child in list.children() {
        list.remove(&child);
    }
    for name in list_kicad_components(output_path, lib_name) {
        let label = Label::new(Some(&name));
        label.set_halign(Align::Start);
        label.set_margin_start(12);
        label.set_margin_end(12);
        label.set_margin_top(8);
        label.set_margin_bottom(8);
        label.style_context().add_class("format-title");
        list.add(&label);
    }
    list.show_all();
}

/// Copy all files whose stem matches `component` from source_pretty to dest_pretty.
fn copy_footprint_files(
    source_pretty: &Path,
    dest_pretty: &Path,
    component: &str,
) -> std::io::Result<usize> {
    std::fs::create_dir_all(dest_pretty)?;
    let mut count = 0;
    for entry in std::fs::read_dir(source_pretty)?.flatten() {
        let path = entry.path();
        if path.file_stem().and_then(|s| s.to_str()) == Some(component) {
            std::fs::copy(&path, dest_pretty.join(path.file_name().unwrap()))?;
            count += 1;
        }
    }
    Ok(count)
}

/// Extract the `(symbol ...)` block from a .kicad_sym file that references
/// `{lib_name}:{component}` in its Footprint property.
fn extract_symbol_block(lib_content: &str, lib_name: &str, component: &str) -> Option<String> {
    let target = format!("\"{}:{}\"", lib_name, component);
    let ref_pos = lib_content.find(&target)?;
    // Walk backwards to find the enclosing (symbol "..." block
    let before = &lib_content[..ref_pos];
    let sym_start = before.rfind("\n  (symbol \"")?;
    let sym_start = sym_start + 1; // skip the newline
    // Count parentheses to find the end
    let from = &lib_content[sym_start..];
    let mut depth = 0i32;
    let mut end = 0;
    for (i, ch) in from.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    end = i + 1;
                    break;
                }
            }
            _ => {}
        }
    }
    if end > 0 { Some(from[..end].to_string()) } else { None }
}

/// Append a symbol block to a .kicad_sym library, creating it if needed.
fn append_to_sym_lib(lib_path: &Path, block: &str) -> std::io::Result<()> {
    if !lib_path.exists() {
        std::fs::write(
            lib_path,
            "(kicad_symbol_lib (version 20211014) (generator library-loader)\n)\n",
        )?;
    }
    let content = std::fs::read_to_string(lib_path)?;
    let close = content.rfind(')').ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidData, "invalid .kicad_sym")
    })?;
    let updated = format!("{}  {}\n)", &content[..close], block.trim());
    std::fs::write(lib_path, updated)
}

/// Transfer one component (footprint + symbol + 3D) from source library to dest folder.
/// Mirrors the KiCad library layout: everything lives under one parent dir with a shared
/// lib name prefix: `{lib}.pretty/`, `{lib}.3dshapes/`, `{lib}.kicad_sym`.
fn do_transfer(
    component: &str,
    source_path: &str,
    source_lib: &str,
    dest_path: &str,
) -> std::io::Result<usize> {
    let src = PathBuf::from(source_path);
    let dst = PathBuf::from(dest_path);
    // Derive dest library name from the last path segment of dest_path
    let dest_lib = dst
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("MyLib")
        .to_string();

    // ── Footprint (.pretty/) ─────────────────────────────────────────────────
    let src_pretty = src.join(format!("{}.pretty", source_lib));
    let dst_pretty = dst.join(format!("{}.pretty", dest_lib));
    let mut count = copy_footprint_files(&src_pretty, &dst_pretty, component)?;

    // ── Symbol (.kicad_sym) ──────────────────────────────────────────────────
    let src_sym = src.join(format!("{}.kicad_sym", source_lib));
    if src_sym.exists() {
        if let Ok(content) = std::fs::read_to_string(&src_sym) {
            if let Some(block) = extract_symbol_block(&content, source_lib, component) {
                let dst_sym = dst.join(format!("{}.kicad_sym", dest_lib));
                let _ = append_to_sym_lib(&dst_sym, &block);
            }
        }
    }

    // ── 3D models (.3dshapes/) ───────────────────────────────────────────────
    // 3D files live flat in {source_path}/{source_lib}.3dshapes/{component}.stp etc.
    let src_shapes = src.join(format!("{}.3dshapes", source_lib));
    if src_shapes.is_dir() {
        let dst_shapes = dst.join(format!("{}.3dshapes", dest_lib));
        std::fs::create_dir_all(&dst_shapes)?;
        for entry in std::fs::read_dir(&src_shapes)?.flatten() {
            let from = entry.path();
            if from.is_file()
                && from.file_stem().and_then(|s| s.to_str()) == Some(component)
            {
                let to = dst_shapes.join(from.file_name().unwrap());
                if !to.exists() {
                    std::fs::copy(&from, &to)?;
                    count += 1;
                }
            }
        }
    }

    Ok(count)
}

// ── Error messages ────────────────────────────────────────────────────────────

fn describe_import_error(e: &ll_core::Error) -> String {
    match e {
        ll_core::Error::ServerError(401) => {
            "Authentication failed (HTTP 401): ComponentSearchEngine rejected your \
             credentials. Sign out and sign back in with your CSE username and password."
                .to_string()
        }
        ll_core::Error::ServerError(403) => {
            "Access denied (HTTP 403): your CSE account may not have permission to \
             download components via the API."
                .to_string()
        }
        ll_core::Error::ServerError(n) => format!("Server error: HTTP {}", n),
        ll_core::Error::NoEpwInZipArchive => {
            "Not a CSE component file: no EPW metadata found in the ZIP. \
             Only files downloaded from ComponentSearchEngine are supported."
                .to_string()
        }
        ll_core::Error::ZipArchiveEmpty => "ZIP file is empty or corrupted.".to_string(),
        ll_core::Error::WouldOverwrite => {
            "A file already exists at the output path. \
             Delete or move the existing component first."
                .to_string()
        }
        ll_core::Error::NoFilesInLibrary => {
            "No matching files found in the downloaded package for the configured format."
                .to_string()
        }
        _ => format!("{}", e),
    }
}

// ─────────────────────────────────────────────────────────────────────────────

impl Ui {
    pub fn new(
        app: &Application,
        config: Rc<RefCell<Config>>,
        config_path: PathBuf,
        watcher_active_flag: Rc<Cell<bool>>,
    ) -> Rc<Self> {
        if let Some(settings) = gtk::Settings::default() {
            settings.set_gtk_application_prefer_dark_theme(true);
        }

        let b = Builder::from_resource(resource!("ui"));

        let main_window = get_obj!(b, ApplicationWindow, "main-window");
        main_window.set_application(Some(app));
        get_obj!(b, HeaderBar, "header-bar").set_subtitle(config_path.to_str());

        // About dialog
        let about_dialog = get_obj!(b, AboutDialog, "about-dialog");
        get_obj!(b, Button, "open-about-dialog").connect_clicked(move |_| {
            about_dialog.run();
            about_dialog.hide();
        });

        let (tx, rx) = glib::MainContext::channel::<UiEvent>(glib::Priority::default());

        let inner = Rc::new(Self {
            config,
            info_bar: get_obj!(b, "info-bar"),
            info_bar_label: get_obj!(b, "info-bar-label"),
            main_stack: get_obj!(b, "main-stack"),
            username_entry: get_obj!(b, "username-entry"),
            password_entry: get_obj!(b, "password-entry"),
            login_button: get_obj!(b, "login-button"),
            info_bar_update: get_obj!(b, "info-bar-update"),
            watcher_switch: get_obj!(b, "watcher-switch"),
            watcher_active_indicator: get_obj!(b, "watcher-active-indicator"),
            watching_status_label: get_obj!(b, "watching-status-label"),
            user_name_label: get_obj!(b, "user-name-label"),
            kicad_symbols_entry: get_obj!(b, "kicad-symbols-entry"),
            kicad_footprints_entry: get_obj!(b, "kicad-footprints-entry"),
            transfer_source_label: get_obj!(b, "transfer-source-label"),
            transfer_dest_entry: get_obj!(b, "transfer-dest-entry"),
            transfer_component_list: get_obj!(b, "transfer-component-list"),
            popover_sign_in: get_obj!(b, "sign-in-button"),
            popover_sign_out: get_obj!(b, "sign-out-button"),
            popover_transfer: get_obj!(b, "open-transfer-page"),
            watcher_active_flag,
            tx,
            watcher: RefCell::new(None),
        });

        // ── Info bar ────────────────────────────────────────────────
        inner.info_bar.connect_response(|ib, _| ib.set_revealed(false));
        inner.info_bar_update.connect_response(|ib, _| ib.set_revealed(false));

        // ── Logger ───────────────────────────────────────────────────
        let text_buffer = get_obj!(b, TextBuffer, "output-log-buffer");
        let mut bounds = text_buffer.bounds();
        text_buffer.delete(&mut bounds.0, &mut bounds.1);
        let (logger_rx, logger) = GuiLogger::new();
        let loaded_count = Rc::new(Cell::new(0u32));
        let log_count_label = get_obj!(b, Label, "log-count-label");
        let loaded_count_c = loaded_count.clone();
        let log_count_c = log_count_label.clone();
        logger_rx.attach(
            None,
            clone!(@strong inner => move |msg| {
                text_buffer.insert(&mut text_buffer.end_iter(), &format!("{}\n", msg));
                if msg.contains("loaded") || msg.contains("success") {
                    let n = loaded_count_c.get() + 1;
                    loaded_count_c.set(n);
                    log_count_c.set_text(&format!("{} loaded today", n));
                }
                glib::ControlFlow::Continue
            }),
        );

        // ── Migration: remove legacy D3 format entries ───────────────
        // 3D files are now handled by the KiCad extractor (.3dshapes/).
        // Remove any standalone D3 entries so the old extractor doesn't
        // create stray per-component subfolders anymore.
        inner.config.borrow_mut().formats.retain(|_, f| f.format != ECAD::D3);

        // ── Event loop ───────────────────────────────────────────────
        rx.attach(
            None,
            clone!(@strong inner => move |event| {
                match event {
                    UiEvent::ShowInfoBar(msg, msg_type) => {
                        inner.info_bar_label.set_text(&msg);
                        inner.info_bar.set_message_type(msg_type);
                        inner.info_bar.set_revealed(true);
                    }
                    UiEvent::SwitchStack(name) => {
                        inner.main_stack.set_visible_child_name(name);
                    }
                    UiEvent::SetProfile(profile) => {
                        let username = &profile.username;
                        inner.user_name_label.set_text(username);
                        inner.config.borrow_mut().profile = profile;
                        inner.set_auth_state(true);
                        // #5 — open CSE in browser after successful login
                        let _ = gtk::show_uri_on_window(
                            gtk::Window::NONE,
                            "https://componentsearchengine.com/",
                            gtk::current_event_time(),
                        );
                    }
                    UiEvent::UpdateAvailable => {
                        inner.info_bar_update.set_revealed(true);
                    }
                }
                glib::ControlFlow::Continue
            }),
        );

        // ── Login ────────────────────────────────────────────────────
        inner.login_button.connect_clicked(clone!(@strong inner => move |_| {
            let tx = inner.tx.clone();
            drop(tx.send(UiEvent::SwitchStack("spinner")));
            let profile = ll_core::Profile {
                username: inner.username_entry.text().to_string(),
                password: inner.password_entry.text().to_string(),
            };
            thread::spawn(move || {
                match profile.try_auth() {
                    Ok(true) => {
                        drop(tx.send(UiEvent::SetProfile(profile)));
                        drop(tx.send(UiEvent::SwitchStack("watch")));
                    }
                    Ok(false) => {
                        drop(tx.send(UiEvent::SwitchStack("login")));
                        drop(tx.send(UiEvent::ShowInfoBar(
                            "Login failed".into(),
                            MessageType::Error,
                        )));
                    }
                    Err(e) => {
                        drop(tx.send(UiEvent::SwitchStack("login")));
                        drop(tx.send(UiEvent::ShowInfoBar(
                            format!("Login failed: {:?}", e),
                            MessageType::Error,
                        )));
                    }
                }
            });
        }));

        // ── Watch path ───────────────────────────────────────────────
        let watch_path_entry = get_obj!(b, Entry, "watch-path-entry");
        watch_path_entry.set_text(&inner.config.borrow().settings.watch_path);
        watch_path_entry.connect_changed(clone!(@strong inner => move |e| {
            inner.config.borrow_mut().settings.watch_path = e.text().to_string();
        }));

        let watch_entry_ref = watch_path_entry.clone();
        let browse_win = main_window.clone();
        get_obj!(b, Button, "watch-path-browse").connect_clicked(
            clone!(@strong inner => move |_| {
                let current = inner.config.borrow().settings.watch_path.clone();
                let expanded = shellexpand::full(&current)
                    .map(|s| s.into_owned())
                    .unwrap_or(current);
                if let Some(s) = Self::pick_folder(&browse_win, &expanded) {
                    watch_entry_ref.set_text(&s);
                    inner.config.borrow_mut().settings.watch_path = s;
                }
            }),
        );

        // ── Recursive ────────────────────────────────────────────────
        let recursive = get_obj!(b, CheckButton, "watch-recursive");
        recursive.set_active(inner.config.borrow().settings.recursive);
        recursive.connect_toggled(clone!(@strong inner => move |btn| {
            inner.config.borrow_mut().settings.recursive = btn.is_active();
        }));

        // ── Output path entries — populate from config ────────────────
        {
            let cfg = inner.config.borrow();
            let kp = kicad_path(&cfg);
            inner.kicad_symbols_entry.set_text(&kp);
            inner.kicad_footprints_entry.set_text(&kp);
        }

        inner.kicad_symbols_entry.connect_changed(
            clone!(@strong inner => move |e| {
                let s = e.text().to_string();
                set_kicad_path(&mut inner.config.borrow_mut(), s.clone());
                if inner.kicad_footprints_entry.text().as_str() != s {
                    inner.kicad_footprints_entry.set_text(&s);
                }
            }),
        );
        inner.kicad_footprints_entry.connect_changed(
            clone!(@strong inner => move |e| {
                let s = e.text().to_string();
                set_kicad_path(&mut inner.config.borrow_mut(), s.clone());
                if inner.kicad_symbols_entry.text().as_str() != s {
                    inner.kicad_symbols_entry.set_text(&s);
                }
            }),
        );
        // Browse buttons for the output paths
        let sym_win = main_window.clone();
        let sym_ref = inner.kicad_symbols_entry.clone();
        get_obj!(b, Button, "kicad-symbols-browse").connect_clicked(
            clone!(@strong inner => move |_| {
                let cur = kicad_path(&inner.config.borrow());
                if let Some(s) = Self::pick_folder(&sym_win, &cur) {
                    sym_ref.set_text(&s);
                }
            }),
        );
        let fp_win = main_window.clone();
        let fp_ref = inner.kicad_footprints_entry.clone();
        get_obj!(b, Button, "kicad-footprints-browse").connect_clicked(
            clone!(@strong inner => move |_| {
                let cur = kicad_path(&inner.config.borrow());
                if let Some(s) = Self::pick_folder(&fp_win, &cur) {
                    fp_ref.set_text(&s);
                }
            }),
        );
        // ── Quick actions ────────────────────────────────────────────
        let import_win = main_window.clone();
        get_obj!(b, Button, "import-ecad-button").connect_clicked(
            clone!(@strong inner => move |_| {
                let dialog = FileChooserNative::new(
                    Some("Import ECAD Package"),
                    Some(&import_win),
                    FileChooserAction::Open,
                    Some("Import"),
                    Some("Cancel"),
                );
                let filter = gtk::FileFilter::new();
                filter.add_pattern("*.zip");
                filter.set_name(Some("ZIP archives (*.zip)"));
                dialog.add_filter(filter);
                if dialog.run() == ResponseType::Accept {
                    if let Some(zip_path) = dialog.file().and_then(|f| f.path()) {
                        let config = inner.config.borrow().clone();
                        let tx = inner.tx.clone();
                        thread::spawn(move || {
                            match ll_core::import_file(zip_path, &config) {
                                Ok(()) => {
                                    drop(tx.send(UiEvent::ShowInfoBar(
                                        "Import successful!".into(),
                                        MessageType::Info,
                                    )));
                                }
                                Err(e) => {
                                    let msg = describe_import_error(&e);
                                    drop(tx.send(UiEvent::ShowInfoBar(
                                        msg,
                                        MessageType::Error,
                                    )));
                                }
                            }
                        });
                    }
                }
            }),
        );

        get_obj!(b, Button, "search-parts-button").connect_clicked(|_| {
            let _ = gtk::show_uri_on_window(
                gtk::Window::NONE,
                "https://componentsearchengine.com/",
                gtk::current_event_time(),
            );
        });

        // ── Popover: Sign In (shown when logged out) ─────────────────
        inner.popover_sign_in.connect_clicked(
            clone!(@strong inner => move |_| {
                drop(inner.tx.send(UiEvent::SwitchStack("login")));
            }),
        );

        // ── Popover: Sign Out ────────────────────────────────────────
        inner.popover_sign_out.connect_clicked(
            clone!(@strong inner => move |_| {
                inner.config.borrow_mut().profile = ll_core::Profile {
                    username: String::new(),
                    password: String::new(),
                };
                let _ = inner.config.borrow().save(None);
                inner.user_name_label.set_text("");
                inner.set_auth_state(false);
                drop(inner.tx.send(UiEvent::SwitchStack("login")));
            }),
        );

        // ── Popover: Transfer page (#6) ──────────────────────────────
        inner.popover_transfer.connect_clicked(
            clone!(@strong inner => move |_| {
                let source_path = kicad_path(&inner.config.borrow());
                let lib_name = find_kicad_key(&inner.config.borrow()).unwrap_or_default();
                inner.transfer_source_label.set_text(
                    if source_path.is_empty() { "No KiCad path configured" } else { &source_path },
                );
                populate_component_list(
                    &inner.transfer_component_list,
                    &source_path,
                    &lib_name,
                );
                drop(inner.tx.send(UiEvent::SwitchStack("transfer")));
            }),
        );

        // Transfer page: click = select only this row; shift+click = extend range
        inner.transfer_component_list.connect_button_press_event(|list, event| {
            let shift_held = event
                .state()
                .contains(gtk::gdk::ModifierType::SHIFT_MASK);
            if !shift_held {
                list.unselect_all();
            }
            glib::Propagation::Proceed
        });

        // Transfer page: Back button
        get_obj!(b, Button, "transfer-back-button").connect_clicked(
            clone!(@strong inner => move |_| {
                drop(inner.tx.send(UiEvent::SwitchStack("watch")));
            }),
        );

        // Transfer page: Browse destination
        let dest_win = main_window.clone();
        let dest_ref = inner.transfer_dest_entry.clone();
        get_obj!(b, Button, "transfer-dest-browse").connect_clicked(move |_| {
            if let Some(s) = Self::pick_folder(&dest_win, "") {
                dest_ref.set_text(&s);
            }
        });

        // Transfer page: Transfer button
        get_obj!(b, Button, "transfer-button").connect_clicked(
            clone!(@strong inner => move |_| {
                let dest = inner.transfer_dest_entry.text().to_string();
                if dest.is_empty() {
                    drop(inner.tx.send(UiEvent::ShowInfoBar(
                        "Please select a destination folder first.".into(),
                        MessageType::Warning,
                    )));
                    return;
                }
                let selected = inner.transfer_component_list.selected_rows();
                if selected.is_empty() {
                    drop(inner.tx.send(UiEvent::ShowInfoBar(
                        "Select at least one component.".into(),
                        MessageType::Warning,
                    )));
                    return;
                }

                // Collect component names from row labels
                let components: Vec<String> = selected
                    .iter()
                    .filter_map(|row| {
                        row.child()
                            .and_then(|w| w.downcast::<Label>().ok())
                            .map(|l| l.text().to_string())
                    })
                    .collect();

                let source_path = kicad_path(&inner.config.borrow());
                let lib_name = find_kicad_key(&inner.config.borrow()).unwrap_or_default();
                let tx = inner.tx.clone();

                thread::spawn(move || {
                    let mut total_files = 0;
                    let mut errors: Vec<String> = vec![];
                    for comp in &components {
                        match do_transfer(comp, &source_path, &lib_name, &dest) {
                            Ok(n) => total_files += n,
                            Err(e) => errors.push(format!("{}: {}", comp, e)),
                        }
                    }
                    if errors.is_empty() {
                        drop(tx.send(UiEvent::ShowInfoBar(
                            format!(
                                "Transferred {} component(s) ({} files copied).",
                                components.len(),
                                total_files
                            ),
                            MessageType::Info,
                        )));
                    } else {
                        drop(tx.send(UiEvent::ShowInfoBar(
                            format!("Transfer errors: {}", errors.join("; ")),
                            MessageType::Error,
                        )));
                    }
                });
            }),
        );

        // ── Watcher switch ───────────────────────────────────────────
        let switch_guard = Rc::new(Cell::new(false));
        inner.watcher_switch.connect_active_notify(
            clone!(@strong inner, @strong switch_guard, @strong watch_path_entry => move |sw| {
                if switch_guard.get() { return; }
                if sw.is_active() {
                    inner.config.borrow_mut().settings.watch_path =
                        watch_path_entry.text().to_string();
                    let config = inner.config.borrow().clone();
                    match Watcher::new(
                        config,
                        vec![ll_core::ConsoleLogger::new(), logger.clone()],
                    )
                    .and_then(|mut w| { w.start()?; Ok(w) })
                    {
                        Ok(w) => {
                            *inner.watcher.borrow_mut() = Some(w);
                            inner.watcher_active_flag.set(true);  // #1 tray colour
                            inner.watcher_active_indicator.show();
                            inner.watching_status_label.set_text("● Watching");
                        }
                        Err(e) => {
                            switch_guard.set(true);
                            sw.set_active(false);
                            switch_guard.set(false);
                            drop(inner.tx.send(UiEvent::ShowInfoBar(
                                format!("Could not start watcher: {:?}", e),
                                MessageType::Error,
                            )));
                        }
                    }
                } else {
                    if let Some(mut w) = inner.watcher.borrow_mut().take() {
                        w.stop();
                    }
                    inner.watcher_active_flag.set(false);  // #1 tray colour
                    inner.watcher_active_indicator.hide();
                    inner.watching_status_label.set_text("");
                }
            }),
        );

        // ── Initial state ────────────────────────────────────────────
        inner.main_stack.set_visible_child_name("watch");

        {
            let cfg = inner.config.borrow();
            let logged_in = !cfg.profile.username.is_empty();
            inner.set_auth_state(logged_in);
            if logged_in {
                inner.user_name_label.set_text(&cfg.profile.username);
            }
        }

        inner
    }

    fn pick_folder(parent: &ApplicationWindow, current: &str) -> Option<String> {
        let dialog = FileChooserNative::new(
            Some("Select Folder"),
            Some(parent),
            FileChooserAction::SelectFolder,
            Some("Select"),
            Some("Cancel"),
        );
        if !current.is_empty() {
            let _ = dialog.set_current_folder(Path::new(current));
        }
        if dialog.run() == ResponseType::Accept {
            dialog
                .file()
                .and_then(|f| f.path())
                .and_then(|p| p.to_str().map(|s| s.to_string()))
        } else {
            None
        }
    }

    pub fn check_logged_in(&self) {
        if self.config.borrow().profile.is_empty() {
            let _ = self.tx.send(UiEvent::SwitchStack("login"));
        } else {
            let tx = self.tx.clone();
            let profile = self.config.borrow().profile.clone();
            thread::spawn(move || match profile.try_auth() {
                Ok(true) => drop(tx.send(UiEvent::SwitchStack("watch"))),
                Ok(false) => drop(tx.send(UiEvent::SwitchStack("login"))),
                Err(e) => eprintln!("{:#?}", e),
            });
        }
    }

    /// Toggle the popover menu items and quick-action buttons based on login state.
    fn set_auth_state(&self, logged_in: bool) {
        self.popover_sign_in.set_visible(!logged_in);
        self.popover_sign_out.set_visible(logged_in);
        self.popover_transfer.set_visible(logged_in);
    }

    pub fn check_updates(&self) {
        let tx = self.tx.clone();
        thread::spawn(move || {
            match ll_core::check_updates(env!("CARGO_PKG_VERSION"), ll_core::ClientKind::GUI) {
                Ok(None) => {}
                Ok(Some(_)) => drop(tx.send(UiEvent::UpdateAvailable)),
                Err(e) => eprintln!("{:#?}", e),
            }
        });
    }
}
