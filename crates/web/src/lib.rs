//! The engine under Wanderlust: a headless Chromium, steered over a pipe.
//!
//! The browser renders a page at exactly the size the shell shows and sends
//! pictures of it; the shell draws them at emulated resolution and puts the
//! CRT pass over them. Nothing of Chromium's own window or chrome is ever on
//! screen. Blocking calls and decoding happen on worker threads, never on the
//! UI thread.

pub mod address;
pub mod cdp;
mod engine;
pub mod input;

pub use engine::{Engine, Event, Options, Request, arguments, find_binary};
pub use input::Mouse;
