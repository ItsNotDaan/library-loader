use {
    gtk::{
        gdk::{self, prelude::*},
        gio,
        glib::{self, clone},
        prelude::*,
        CssProvider, StyleContext, STYLE_PROVIDER_PRIORITY_APPLICATION,
    },
    ll_core::{Config, Result},
    std::{cell::{Cell, RefCell}, rc::Rc},
    ui::Ui,
};

mod consts;
mod macros;
mod ui;

fn main() -> Result<()> {
    // When launched as a .app bundle from Finder, Homebrew paths are not in the
    // environment. Set them here so GTK and GDK can find resources at runtime.
    #[cfg(target_os = "macos")]
    {
        use std::env;
        if env::var("GTK_PATH").is_err() {
            let path = env::var("PATH").unwrap_or_default();
            env::set_var("PATH", format!("/opt/homebrew/bin:/opt/homebrew/sbin:{path}"));
            env::set_var("XDG_DATA_DIRS", "/opt/homebrew/share");
            env::set_var("GTK_DATA_PREFIX", "/opt/homebrew");
            env::set_var("GTK_EXE_PREFIX", "/opt/homebrew");
            env::set_var("GTK_PATH", "/opt/homebrew");
            env::set_var("GDK_PIXBUF_MODULE_FILE", "/opt/homebrew/lib/gdk-pixbuf-2.0/2.10.0/loaders.cache");
        }
    }

    let config_path = match Config::get_path()? {
        Some(p) => p,
        None => {
            Config::default().save(None)?;
            Config::default_path().expect("Failed to get default path")
        }
    };

    println!("Using config: {:?}", config_path);

    let config = Rc::new(RefCell::new(Config::read(Some(config_path.clone()))?));

    load_resources();

    let application = gtk::Application::new(Some("net.olback.library-loader"), Default::default());

    application.connect_activate(clone!(@weak config => move |app| {
        // ── CSS ──────────────────────────────────────────────────────
        let provider = CssProvider::new();
        provider.load_from_resource(resource!("app.css"));
        StyleContext::add_provider_for_screen(
            &gdk::Screen::default().expect("Error initializing gtk css provider."),
            &provider,
            STYLE_PROVIDER_PRIORITY_APPLICATION,
        );

        // ── Shared watcher-active flag (drives tray icon colour) ─────
        let watcher_flag = Rc::new(Cell::new(false));
        let watcher_flag_for_tray = watcher_flag.clone();

        // ── Build UI ─────────────────────────────────────────────────
        let _u = Ui::new(app, config, config_path.clone(), watcher_flag);
        #[cfg(not(debug_assertions))]
        _u.check_logged_in();
        #[cfg(not(debug_assertions))]
        _u.check_updates();

        // ── Menu bar status icon ──────────────────────────────────────
        // Build exit menu
        use tray_icon::menu::{Menu, MenuItem};
        let exit_item = MenuItem::new("Exit Library Loader", true, None);
        let exit_id   = exit_item.id().clone();
        let tray_menu = Menu::new();
        let _ = tray_menu.append(&exit_item);

        // Initial icon — blue (watcher inactive)
        match tray_icon::Icon::from_rgba(circle_rgba(22, (0x89, 0xb4, 0xfa)), 22, 22) {
            Ok(icon) => {
                match tray_icon::TrayIconBuilder::new()
                    .with_tooltip("Library Loader")
                    .with_icon(icon)
                    .with_menu(Box::new(tray_menu))
                    .build()
                {
                    Ok(tray) => {
                        let app_ref = app.clone();
                        let mut prev_active = false;

                        // Poll tray events every 100 ms on the GTK main thread.
                        glib::timeout_add_local(
                            std::time::Duration::from_millis(100),
                            move || {
                                // ① Keep tray alive inside this closure.
                                let _ = &tray;

                                // ② Update icon colour when watcher state changes.
                                let active = watcher_flag_for_tray.get();
                                if active != prev_active {
                                    prev_active = active;
                                    let color = if active {
                                        (0xa6u8, 0xe3u8, 0xa1u8) // Catppuccin green
                                    } else {
                                        (0x89u8, 0xb4u8, 0xfau8) // Catppuccin blue
                                    };
                                    if let Ok(new_icon) =
                                        tray_icon::Icon::from_rgba(circle_rgba(22, color), 22, 22)
                                    {
                                        let _ = tray.set_icon(Some(new_icon));
                                    }
                                }

                                // ③ Left-click → show window.
                                if let Ok(tray_icon::TrayIconEvent::Click {
                                    button: tray_icon::MouseButton::Left,
                                    button_state: tray_icon::MouseButtonState::Up,
                                    ..
                                }) = tray_icon::TrayIconEvent::receiver().try_recv()
                                {
                                    if let Some(w) = app_ref.windows().first() {
                                        w.present();
                                    }
                                }

                                // ④ Menu event → Exit.
                                if let Ok(event) =
                                    tray_icon::menu::MenuEvent::receiver().try_recv()
                                {
                                    if event.id == exit_id {
                                        app_ref.quit();
                                    }
                                }

                                glib::ControlFlow::Continue
                            },
                        );
                    }
                    Err(e) => eprintln!("Could not create tray icon: {e}"),
                }
            }
            Err(e) => eprintln!("Could not create tray icon image: {e}"),
        }
    }));

    application.run();

    // Save config on exit
    config.borrow().save(None)?;
    Ok(())
}

/// Generate RGBA bytes for a filled circle of `size` pixels in the given RGB colour.
fn circle_rgba(size: u32, (r, g, b): (u8, u8, u8)) -> Vec<u8> {
    let mut rgba = vec![0u8; (size * size * 4) as usize];
    let cx = size as f32 / 2.0;
    let radius = cx - 1.5;
    for y in 0..size {
        for x in 0..size {
            let dx = x as f32 - cx;
            let dy = y as f32 - cx;
            if (dx * dx + dy * dy).sqrt() <= radius {
                let i = ((y * size + x) * 4) as usize;
                rgba[i]     = r;
                rgba[i + 1] = g;
                rgba[i + 2] = b;
                rgba[i + 3] = 0xff;
            }
        }
    }
    rgba
}

pub fn load_resources() {
    let glib_resource_bytes = glib::Bytes::from_static(consts::RESOURCES_BYTES);
    let resources =
        gio::Resource::from_data(&glib_resource_bytes).expect("Failed to load resources");
    gio::resources_register(&resources);
}
