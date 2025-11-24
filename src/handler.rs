use crate::app::App;
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};

pub fn handle_key_event(key_event: KeyEvent, app: &mut App) -> Result<()> {
    match key_event.code {
        // 退出
        KeyCode::Char('q') | KeyCode::Esc => {
            app.quit();
        }
        
        // 切换视图模式 (表格 <-> 示波器)
        KeyCode::Tab => {
            app.toggle_view_mode();
        }

        // 你可以在这里保留之前的排序键，但需要在 App 中重新实现排序逻辑
        // 目前为了专注于监控性能，暂时移除排序
        // KeyCode::Char('n') => app.set_sort_key(SortKey::Name),
        
        _ => {}
    }
    Ok(())
}