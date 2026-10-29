//! Background sync worker: a plain thread with a tiny tokio runtime.
//!
//! The UI never blocks on the network. It drops requests in, and picks status
//! updates up on the next frame.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread;
use std::time::Duration;

use crate::{Config, Report, sync_once};

/// How long to wait after the last change before actually syncing, so a burst
/// of keystrokes becomes one round trip.
const DEBOUNCE: Duration = Duration::from_millis(700);

pub enum Command {
    SyncNow,
    Configure(Box<Config>),
    Shutdown,
}

#[derive(Clone, Debug)]
pub enum Event {
    Started,
    Done(Report),
    Failed(String),
}

pub struct Worker {
    tx: Sender<Command>,
    rx: Receiver<Event>,
    handle: Option<thread::JoinHandle<()>>,
}

impl Worker {
    /// Start the worker. `interval` is the idle poll period when syncing is on.
    pub fn spawn(vault: PathBuf, cfg: Config, interval: Duration) -> Self {
        let (tx, command_rx) = mpsc::channel();
        let (event_tx, rx) = mpsc::channel();

        let handle = thread::spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    let _ =
                        event_tx.send(Event::Failed(format!("cannot start async runtime: {e}")));
                    return;
                }
            };

            let mut cfg = cfg;
            loop {
                let command = match command_rx.recv_timeout(interval) {
                    Ok(c) => c,
                    Err(RecvTimeoutError::Timeout) => {
                        if cfg.enabled && cfg.is_configured() {
                            run(&rt, &vault, &cfg, &event_tx);
                        }
                        continue;
                    }
                    Err(RecvTimeoutError::Disconnected) => break,
                };

                match command {
                    Command::Shutdown => break,
                    Command::Configure(next) => {
                        cfg = *next;
                        if cfg.enabled && cfg.is_configured() {
                            run(&rt, &vault, &cfg, &event_tx);
                        }
                    }
                    Command::SyncNow => {
                        // Coalesce a burst into one round.
                        thread::sleep(DEBOUNCE);
                        while let Ok(cmd) = command_rx.try_recv() {
                            match cmd {
                                Command::SyncNow => {}
                                Command::Configure(next) => cfg = *next,
                                Command::Shutdown => return,
                            }
                        }
                        if cfg.enabled && cfg.is_configured() {
                            run(&rt, &vault, &cfg, &event_tx);
                        }
                    }
                }
            }
        });

        Self {
            tx,
            rx,
            handle: Some(handle),
        }
    }

    pub fn request_sync(&self) {
        let _ = self.tx.send(Command::SyncNow);
    }

    pub fn configure(&self, cfg: Config) {
        let _ = self.tx.send(Command::Configure(Box::new(cfg)));
    }

    pub fn try_recv(&self) -> Option<Event> {
        self.rx.try_recv().ok()
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.tx.send(Command::Shutdown);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn run(
    rt: &tokio::runtime::Runtime,
    vault: &std::path::Path,
    cfg: &Config,
    events: &Sender<Event>,
) {
    let _ = events.send(Event::Started);
    match rt.block_on(sync_once(vault, cfg)) {
        Ok(report) => {
            let _ = events.send(Event::Done(report));
        }
        Err(e) => {
            let _ = events.send(Event::Failed(e));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unconfigured_worker_does_not_touch_the_network() {
        let dir = std::env::temp_dir().join(format!("cread-worker-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let worker = Worker::spawn(dir.clone(), Config::default(), Duration::from_millis(50));
        worker.request_sync();
        thread::sleep(Duration::from_millis(200));
        assert!(
            worker.try_recv().is_none(),
            "disabled config must stay silent"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
