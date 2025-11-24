use crate::app::App;
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};

pub fn handle_key_event(key_event: KeyEvent, app: &mut App) -> Result<()> {
    match key_event.code {
        // 退出
        KeyCode::Char('q') | KeyCode::Esc => {
            app.quit();
        }
        
        // 切换视图
        KeyCode::Tab => {
            app.toggle_view_mode();
        }

        // --- 新增：滚动操作 ---
        // 向上滚动
        KeyCode::Up | KeyCode::Char('k') => {
            app.on_up();
        }
        // 向下滚动
        KeyCode::Down | KeyCode::Char('j') => {
            app.on_down();
        }
        
        _ => {}
    }
    Ok(())
}