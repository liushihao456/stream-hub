use crate::player::EmbeddedPlayerBounds;
use tauri::AppHandle;

pub struct UnsupportedHost;

impl UnsupportedHost {
    pub fn new(_app: &AppHandle) -> Result<Self, String> {
        Err("当前平台暂不支持内嵌 libmpv 播放器".to_string())
    }

    pub fn embed_id(&self) -> isize {
        0
    }

    pub fn set_bounds(
        &mut self,
        _app: &AppHandle,
        _bounds: &EmbeddedPlayerBounds,
    ) -> Result<(), String> {
        Ok(())
    }

    pub fn set_visible(&mut self, _app: &AppHandle, _visible: bool) -> Result<(), String> {
        Ok(())
    }
}
