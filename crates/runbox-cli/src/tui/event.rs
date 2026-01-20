#![allow(dead_code)]
//!
//! Manages keyboard events and tick events for auto-refresh.

use anyhow::Result;
use crossterm::event::{self, Event as CrosstermEvent, KeyCode, KeyEvent, KeyModifiers};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

/// Terminal events
#[derive(Debug, Clone)]
pub enum Event {
    /// Keyboard input
    Key(KeyEvent),
    /// Terminal tick for refresh
    Tick,
    /// Terminal resize
    Resize(u16, u16),
}

/// Event handler that spawns a thread to listen for events
pub struct EventHandler {
    receiver: mpsc::Receiver<Event>,
    _sender: mpsc::Sender<Event>,
}

impl EventHandler {
    /// Create a new event handler with the specified tick rate
    pub fn new(tick_rate: Duration) -> Self {
        let (sender, receiver) = mpsc::channel();
        let _sender = sender.clone();

        thread::spawn(move || {
            let mut last_tick = Instant::now();
            loop {
                // Calculate remaining time until next tick
                let timeout = tick_rate
                    .checked_sub(last_tick.elapsed())
                    .unwrap_or(Duration::ZERO);

                // Poll for events with timeout
                if event::poll(timeout).unwrap_or(false) {
                    match event::read() {
                        Ok(CrosstermEvent::Key(key)) => {
                            // Ignore key release events on some platforms
                            if key.kind == crossterm::event::KeyEventKind::Press {
                                if sender.send(Event::Key(key)).is_err() {
                                    return;
                                }
                            }
                        }
                        Ok(CrosstermEvent::Resize(w, h)) => {
                            if sender.send(Event::Resize(w, h)).is_err() {
                                return;
                            }
                        }
                        _ => {}
                    }
                }

                // Send tick event at regular intervals
                if last_tick.elapsed() >= tick_rate {
                    if sender.send(Event::Tick).is_err() {
                        return;
                    }
                    last_tick = Instant::now();
                }
            }
        });

        Self { receiver, _sender }
    }

    /// Wait for the next event
    pub fn next(&self) -> Result<Event> {
        Ok(self.receiver.recv()?)
    }
}

/// Key bindings helper
pub struct KeyBindings;

impl KeyBindings {
    /// Check if key is quit (q or Ctrl+C)
    pub fn is_quit(key: KeyEvent) -> bool {
        matches!(
            key,
            KeyEvent {
                code: KeyCode::Char('q'),
                modifiers: KeyModifiers::NONE,
                ..
            } | KeyEvent {
                code: KeyCode::Char('c'),
                modifiers: KeyModifiers::CONTROL,
                ..
            }
        )
    }

    /// Check if key is up (k or Up arrow)
    pub fn is_up(key: KeyEvent) -> bool {
        matches!(
            key.code,
            KeyCode::Up | KeyCode::Char('k')
        )
    }

    /// Check if key is down (j or Down arrow)
    pub fn is_down(key: KeyEvent) -> bool {
        matches!(
            key.code,
            KeyCode::Down | KeyCode::Char('j')
        )
    }

    /// Check if key is select/enter
    pub fn is_select(key: KeyEvent) -> bool {
        matches!(key.code, KeyCode::Enter)
    }

    /// Check if key is back/escape
    pub fn is_back(key: KeyEvent) -> bool {
        matches!(key.code, KeyCode::Esc | KeyCode::Backspace)
    }

    /// Check if key is stop (s)
    pub fn is_stop(key: KeyEvent) -> bool {
        matches!(
            key,
            KeyEvent {
                code: KeyCode::Char('s'),
                modifiers: KeyModifiers::NONE,
                ..
            }
        )
    }

    /// Check if key is attach (a)
    pub fn is_attach(key: KeyEvent) -> bool {
        matches!(
            key,
            KeyEvent {
                code: KeyCode::Char('a'),
                modifiers: KeyModifiers::NONE,
                ..
            }
        )
    }

    /// Check if key is refresh (r)
    pub fn is_refresh(key: KeyEvent) -> bool {
        matches!(
            key,
            KeyEvent {
                code: KeyCode::Char('r'),
                modifiers: KeyModifiers::NONE,
                ..
            }
        )
    }

    /// Check if key is search (/)
    pub fn is_search(key: KeyEvent) -> bool {
        matches!(
            key,
            KeyEvent {
                code: KeyCode::Char('/'),
                modifiers: KeyModifiers::NONE,
                ..
            }
        )
    }

    /// Check if key is follow mode toggle (f)
    pub fn is_follow(key: KeyEvent) -> bool {
        matches!(
            key,
            KeyEvent {
                code: KeyCode::Char('f'),
                modifiers: KeyModifiers::NONE,
                ..
            }
        )
    }

    /// Check if key is go to top (g)
    pub fn is_goto_top(key: KeyEvent) -> bool {
        matches!(
            key,
            KeyEvent {
                code: KeyCode::Char('g'),
                modifiers: KeyModifiers::NONE,
                ..
            }
        )
    }

    /// Check if key is go to bottom (G)
    pub fn is_goto_bottom(key: KeyEvent) -> bool {
        matches!(
            key,
            KeyEvent {
                code: KeyCode::Char('G'),
                modifiers: KeyModifiers::SHIFT,
                ..
            } | KeyEvent {
                code: KeyCode::Char('G'),
                modifiers: KeyModifiers::NONE,
                ..
            }
        )
    }

    /// Check if key is page up
    pub fn is_page_up(key: KeyEvent) -> bool {
        matches!(key.code, KeyCode::PageUp)
    }

    /// Check if key is page down
    pub fn is_page_down(key: KeyEvent) -> bool {
        matches!(key.code, KeyCode::PageDown)
    }

    /// Check if key is help (?)
    pub fn is_help(key: KeyEvent) -> bool {
        matches!(
            key,
            KeyEvent {
                code: KeyCode::Char('?'),
                ..
            }
        )
    }

    /// Check if key is tab (for pane switching)
    pub fn is_tab(key: KeyEvent) -> bool {
        matches!(key.code, KeyCode::Tab)
    }
}
