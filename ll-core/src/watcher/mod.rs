use {
    crate::{
        config::Config, cse::CSE, epw::Epw, error::Result, format::Format, log_error, log_if_error,
        log_info, logger::Logger,
    },
    event::WatcherEvent,
    notify::{
        event::CreateKind as NotifyCreateKind, EventKind as NotifyEventKind,
        Watcher as NotifyWatcher,
    },
    std::{
        ffi::OsString,
        path::PathBuf,
        sync::{mpsc, Arc},
        thread::{self, JoinHandle},
    },
};

mod event;

/// Turn a core error into a human-readable string suitable for display in the log.
fn describe_error(e: &crate::error::Error) -> String {
    use crate::error::Error;
    match e {
        Error::ServerError(401) => {
            "Authentication failed (HTTP 401): ComponentSearchEngine rejected your credentials. \
             Please sign out and sign back in with your CSE username and password."
                .to_string()
        }
        Error::ServerError(403) => {
            "Access denied (HTTP 403): your CSE account may not have permission to \
             download components via the API."
                .to_string()
        }
        Error::ServerError(n) => format!("Server error: HTTP {}", n),
        Error::NoEpwInZipArchive => {
            "Not a CSE component file: no EPW metadata found inside the ZIP. \
             Only ZIP files downloaded from ComponentSearchEngine are supported."
                .to_string()
        }
        Error::ZipArchiveEmpty => "ZIP file is empty or corrupted.".to_string(),
        Error::WouldOverwrite => {
            "File already exists at the output path — skipping to avoid overwriting.".to_string()
        }
        Error::NoFilesInLibrary => {
            "No matching files found in the downloaded package for the configured format."
                .to_string()
        }
        Error::Other(msg) => msg.to_string(),
        _ => format!("{}", e),
    }
}

/// Process a single component ZIP file exactly as the watcher would do automatically.
/// Reads the EPW metadata from the zip, fetches the component from CSE, and
/// writes the output files to the configured paths.
pub fn import_file(path: PathBuf, config: &Config) -> Result<()> {
    let token = config.profile.token();
    let formats = Arc::new(config.formats()?);
    let epw = Epw::from_file(path)?;
    for res in CSE::new(token, formats).get(epw)? {
        res.save()?;
    }
    Ok(())
}


pub struct Watcher {
    token: String,
    watch_path: PathBuf,
    formats: Arc<Vec<Format>>,
    loggers: Arc<Vec<Box<dyn Logger>>>,
    thread: Option<(
        JoinHandle<()>,
        mpsc::Sender<WatcherEvent>,
        notify::RecommendedWatcher,
    )>,
    recursive: bool,
}

impl Watcher {
    pub fn new(config: Config, loggers: Vec<Box<dyn Logger>>) -> Result<Self> {
        Ok(Self {
            token: config.profile.token(),
            watch_path: PathBuf::from(shellexpand::full(&config.settings.watch_path)?.as_ref()),
            formats: Arc::new(config.formats()?),
            loggers: Arc::new(loggers),
            thread: None,
            recursive: config.settings.recursive,
        })
    }

    pub fn start(&mut self) -> Result<()> {
        let (tx, rx) = mpsc::channel();
        let ntx = tx.clone();

        let loggers = Arc::clone(&self.loggers);
        let mut w: notify::RecommendedWatcher = notify::Watcher::new(
            move |evt| match ntx.send(WatcherEvent::NotifyResult(evt)) {
                Ok(_) => {}
                Err(e) => log_error!(&*loggers, format!("{:?}", e)),
            },
            notify::Config::default(),
        )?;

        let token = self.token.clone();
        let formats = Arc::clone(&self.formats);
        let loggers = Arc::clone(&self.loggers);
        let jh = thread::spawn(move || loop {
            match rx.recv() {
                Ok(WatcherEvent::NotifyResult(Ok(event))) => {
                    // log_info!(&*loggers, format!("{:#?}", event));

                    if let NotifyEventKind::Create(NotifyCreateKind::File) = event.kind {
                        // println!("evt: {:#?}", event);
                        for file in event.paths {
                            if file.extension().map(|e| e.to_ascii_lowercase())
                                == Some(OsString::from("zip"))
                            {
                                log_info!(&*loggers, format!("Detected {:?}", file));
                                let token = token.clone();
                                let formats = Arc::clone(&formats);
                                let loggers_clone = Arc::clone(&loggers);
                                // uuuh
                                std::thread::sleep(std::time::Duration::from_millis(100));
                                match (move || -> Result<()> {
                                    let epw = Epw::from_file(file)?;
                                    for res in CSE::new(token, formats).get(epw)? {
                                        match res.save() {
                                            Ok(save_path) => {
                                                log_info!(
                                                    &*loggers_clone,
                                                    format!("Saved to {:?}", save_path)
                                                )
                                            }
                                            Err(e) => {
                                                log_error!(&*loggers_clone, describe_error(&e))
                                            }
                                        }
                                    }
                                    Ok(())
                                })() {
                                    Ok(()) => {
                                        log_info!(&*loggers, "Done");
                                    }
                                    Err(e) => {
                                        log_error!(&*loggers, describe_error(&e));
                                    }
                                }
                            }
                        }
                        // log_info!(&*loggers, format!("{:#?}", event));
                    }
                }
                Ok(WatcherEvent::NotifyResult(Err(error))) => {
                    log_error!(&*loggers, format!("{:#?}", error))
                }
                Ok(WatcherEvent::Stop) => break,
                Err(_recv_error) => {
                    log_error!(&*loggers, "TX has gone away")
                }
            }
        });

        w.watch(
            self.watch_path.as_path(),
            if self.recursive {
                notify::RecursiveMode::Recursive
            } else {
                notify::RecursiveMode::NonRecursive
            },
        )?;

        self.thread = Some((jh, tx, w));

        log_info!(
            &*self.loggers,
            format!("Started watching {:?}", self.watch_path)
        );

        log_info!(&*self.loggers, "Active formats:");
        for f in &*self.formats {
            log_info!(
                &*self.loggers,
                format!("\t{} => {:?}", f.ecad, f.output_path)
            )
        }

        Ok(())
    }

    pub fn stop(&mut self) {
        if let Some((jh, tx, mut w)) = self.thread.take() {
            log_if_error!(&*self.loggers, w.unwatch(self.watch_path.as_path()));
            log_if_error!(&*self.loggers, tx.send(WatcherEvent::Stop));
            log_if_error!(&*self.loggers, jh.join());
            log_info!(
                &*self.loggers,
                format!("Stopped watching {:?}", self.watch_path)
            );
        }
    }
}
