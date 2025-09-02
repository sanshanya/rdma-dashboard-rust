use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io::{self, Stdout}; // 'self' here imports the 'io' module itself
use std::ops::{Deref, DerefMut};

pub struct Tui(Terminal<CrosstermBackend<Stdout>>);

impl Tui {
    pub fn new() -> io::Result<Self> {
        execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;
        enable_raw_mode()?;

        let terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;
        Ok(Self(terminal))
    }

    fn restore() -> io::Result<()> {
        execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture)?;
        disable_raw_mode()?;
        Ok(())
    }
}

impl Deref for Tui {
    type Target = Terminal<CrosstermBackend<Stdout>>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for Tui {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Drop for Tui {
    fn drop(&mut self) {

        let _ = Tui::restore();
    }
}