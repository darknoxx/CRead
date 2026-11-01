//! The update worker: a thread that talks to GitHub while the UI keeps drawing.
//!
//! It is the same shape as the sync worker: commands go down, events come back
//! on the next frame. Dropping it joins the thread, which is what a restart
//! relies on.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use crate::{Offer, PUBLIC_KEYS};

enum Command {
    Check,
    Install(Offer),
    Shutdown,
}

#[derive(Clone, Debug)]
pub enum Event {
    /// The running version is the newest one.
    UpToDate(String),
    /// A newer release exists; the shell asks before installing.
    Available(Offer),
    /// The binary was replaced; a restart is needed.
    Installed(String),
    Failed(String),
}

pub struct Worker {
    tx: Sender<Command>,
    rx: Receiver<Event>,
    handle: Option<thread::JoinHandle<()>>,
}

impl Worker {
    /// `current` is the running version, `target` the binary to replace.
    pub fn spawn(current: String, target: PathBuf, api: String) -> Self {
        let (tx, commands) = mpsc::channel();
        let (events, rx) = mpsc::channel();

        let handle = thread::spawn(move || {
            while let Ok(command) = commands.recv() {
                let event = match command {
                    Command::Shutdown => break,
                    Command::Check => match crate::check(&api, &current) {
                        Ok(Some(offer)) => Event::Available(offer),
                        Ok(None) => Event::UpToDate(current.clone()),
                        Err(e) => Event::Failed(e),
                    },
                    Command::Install(offer) => match crate::install(&offer, &target, PUBLIC_KEYS) {
                        Ok(_) => Event::Installed(offer.version),
                        Err(e) => Event::Failed(e),
                    },
                };
                let _ = events.send(event);
            }
        });

        Self {
            tx,
            rx,
            handle: Some(handle),
        }
    }

    pub fn check(&self) {
        let _ = self.tx.send(Command::Check);
    }

    pub fn install(&self, offer: Offer) {
        let _ = self.tx.send(Command::Install(offer));
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
