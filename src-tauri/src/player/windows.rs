use crate::player::EmbeddedPlayerBounds;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::ptr;
use tauri::{AppHandle, Manager};
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, MoveWindow, ShowWindow, SW_HIDE, SW_SHOW, WINDOW_EX_STYLE, WS_CHILD,
    WS_VISIBLE,
};

pub struct WindowsHost {
    hwnd: HWND,
}

impl WindowsHost {
    pub fn new(app: &AppHandle) -> Result<Self, String> {
        let window = app
            .get_webview_window("main")
            .ok_or_else(|| "未找到主窗口".to_string())?;
        let raw = window
            .window_handle()
            .map_err(|err| err.to_string())?
            .as_raw();
        let RawWindowHandle::Win32(handle) = raw else {
            return Err("当前窗口不是 Win32 句柄".to_string());
        };
        let parent = handle.hwnd.get() as HWND;
        let class_name: Vec<u16> = "STATIC\0".encode_utf16().collect();
        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                class_name.as_ptr(),
                ptr::null(),
                WS_CHILD | WS_VISIBLE,
                0,
                0,
                1,
                1,
                parent,
                0,
                handle
                    .hinstance
                    .map(|value| value.get())
                    .unwrap_or_default() as _,
                ptr::null(),
            )
        };
        if hwnd == 0 {
            return Err("创建播放器宿主窗口失败".to_string());
        }
        Ok(Self { hwnd })
    }

    pub fn embed_id(&self) -> isize {
        self.hwnd as isize
    }

    pub fn set_bounds(
        &mut self,
        _app: &AppHandle,
        bounds: &EmbeddedPlayerBounds,
    ) -> Result<(), String> {
        let moved = unsafe {
            MoveWindow(
                self.hwnd,
                bounds.x.round() as i32,
                bounds.y.round() as i32,
                bounds.width.round() as i32,
                bounds.height.round() as i32,
                1,
            )
        };
        if moved == 0 {
            Err("更新播放器位置失败".to_string())
        } else {
            Ok(())
        }
    }

    pub fn set_visible(&mut self, _app: &AppHandle, visible: bool) -> Result<(), String> {
        unsafe {
            ShowWindow(self.hwnd, if visible { SW_SHOW } else { SW_HIDE });
        }
        Ok(())
    }
}
