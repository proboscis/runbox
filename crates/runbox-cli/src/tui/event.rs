//! Event handling for the TUI

use anyhow::Result;
use crossterm::event::{self, KeyEvent, MouseEvent};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

/// Terminal events
#[derive(Debug)]
pub enum Event {
    /// Terminal tick (for auto-refresh)
    Tick,
    /// Keyboard event
    Key(KeyEvent),
    /// Mouse event
    #[allow(dead_code)]
    Mouse(MouseEvent),
    /// Terminal resize
    #[allow(dead_code)]
    Resize(u16, u16),
}

/// Handles terminal events in a separate thread
pub struct EventHandler {
    /// Event receiver
    rx: mpsc::Receiver<Event>,
    /// Event sender (kept for potential future use)
    _tx: mpsc::Sender<Event>,
}

impl EventHandler {
    /// Create a new event handler with the given tick rate
    pub fn new(tick_rate: Duration) -> Self {
        let (tx, rx) = mpsc::channel();
        let event_tx = tx.clone();

        // Spawn event polling thread
        thread::spawn(move || {
            loop {
                // Poll for events with timeout
                if event::poll(tick_rate).unwrap_or(false) {
                    match event::read() {
                        Ok(event::Event::Key(key)) => {
                            if event_tx.send(Event::Key(key)).is_err() {
                                break;
                            }
                        }
                        Ok(event::Event::Mouse(mouse)) => {
                            if event_tx.send(Event::Mouse(mouse)).is_err() {
                                break;
                            }
                        }
                        Ok(event::Event::Resize(w, h)) => {
                            if event_tx.send(Event::Resize(w, h)).is_err() {
                                break;
                            }
                        }
                        _ => {}
                    }
                } else {
                    // Timeout - send tick event
                    if event_tx.send(Event::Tick).is_err() {
                        break;
                    }
                }
            }
        });

        Self { rx, _tx: tx }
    }

    /// Get the next event (blocking)
    pub fn next(&self) -> Result<Event> {
        Ok(self.rx.recv()?)
    }
}
