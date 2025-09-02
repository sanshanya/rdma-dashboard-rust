use crate::app::{App, SortKey};
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};

pub fn handle_key_event(key_event: KeyEvent, app: &mut App) -> Result<()> {
    match key_event.code {
        KeyCode::Char('q') => app.quit(),
        KeyCode::Char('n') => app.set_sort_key(SortKey::Name),
        KeyCode::Char('r') => app.set_sort_key(SortKey::Rx),
        KeyCode::Char('t') => app.set_sort_key(SortKey::Tx),
        _ => {}
    }
    Ok(())
}