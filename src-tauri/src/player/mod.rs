use crate::{ExtractResult, Settings, Streamer};
use libmpv_sys::{
    mpv_command, mpv_create, mpv_destroy, mpv_end_file_reason_MPV_END_FILE_REASON_ERROR,
    mpv_error_string, mpv_event_end_file, mpv_event_id_MPV_EVENT_END_FILE,
    mpv_event_id_MPV_EVENT_FILE_LOADED, mpv_event_id_MPV_EVENT_IDLE,
    mpv_event_id_MPV_EVENT_LOG_MESSAGE, mpv_event_id_MPV_EVENT_PAUSE,
    mpv_event_id_MPV_EVENT_PLAYBACK_RESTART, mpv_event_id_MPV_EVENT_SHUTDOWN,
    mpv_event_id_MPV_EVENT_START_FILE, mpv_event_id_MPV_EVENT_UNPAUSE, mpv_event_log_message,
    mpv_format_MPV_FORMAT_DOUBLE, mpv_format_MPV_FORMAT_FLAG, mpv_format_MPV_FORMAT_INT64,
    mpv_format_MPV_FORMAT_STRING, mpv_get_property, mpv_handle, mpv_initialize,
    mpv_request_log_messages, mpv_set_option, mpv_set_option_string, mpv_set_property,
    mpv_wait_event,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::ffi::{c_void, CStr, CString};
use std::ptr;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use tauri::{AppHandle, Emitter, Manager};

#[cfg(target_os = "macos")]
mod macos;
#[cfg(not(any(target_os = "macos", windows)))]
mod unsupported;
#[cfg(windows)]
mod windows;

#[cfg(target_os = "macos")]
type PlatformHost = macos::MacHost;
#[cfg(not(any(target_os = "macos", windows)))]
type PlatformHost = unsupported::UnsupportedHost;
#[cfg(windows)]
type PlatformHost = windows::WindowsHost;

const PLAYER_EVENT_STATE: &str = "embedded-player-state";
const PLAYER_EVENT_ERROR: &str = "embedded-player-error";
const MPV_EVENT_START_FILE: u32 = mpv_event_id_MPV_EVENT_START_FILE;
const MPV_EVENT_FILE_LOADED: u32 = mpv_event_id_MPV_EVENT_FILE_LOADED;
const MPV_EVENT_PLAYBACK_RESTART: u32 = mpv_event_id_MPV_EVENT_PLAYBACK_RESTART;
const MPV_EVENT_PAUSE: u32 = mpv_event_id_MPV_EVENT_PAUSE;
const MPV_EVENT_UNPAUSE: u32 = mpv_event_id_MPV_EVENT_UNPAUSE;
const MPV_EVENT_IDLE: u32 = mpv_event_id_MPV_EVENT_IDLE;
const MPV_EVENT_END_FILE: u32 = mpv_event_id_MPV_EVENT_END_FILE;
const MPV_EVENT_SHUTDOWN: u32 = mpv_event_id_MPV_EVENT_SHUTDOWN;
const MPV_EVENT_LOG_MESSAGE: u32 = mpv_event_id_MPV_EVENT_LOG_MESSAGE;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddedPlayerSnapshot {
    phase: String,
    title: String,
    streamer_name: String,
    platform: String,
    visible: bool,
    paused: bool,
    muted: bool,
    volume: f64,
    using_external_player: bool,
    error_message: String,
}

impl Default for EmbeddedPlayerSnapshot {
    fn default() -> Self {
        Self {
            phase: "idle".to_string(),
            title: String::new(),
            streamer_name: String::new(),
            platform: String::new(),
            visible: false,
            paused: false,
            muted: false,
            volume: 100.0,
            using_external_player: false,
            error_message: String::new(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddedPlayerBounds {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    scale_factor: f64,
    #[serde(default)]
    viewport_height: f64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddedPlayerCommandPayload {
    kind: String,
    #[serde(default)]
    value: Option<Value>,
}

#[derive(Clone)]
pub struct EmbeddedPlayerManager {
    inner: Arc<Mutex<EmbeddedPlayerState>>,
}

struct EmbeddedPlayerState {
    snapshot: EmbeddedPlayerSnapshot,
    last_bounds: Option<EmbeddedPlayerBounds>,
    last_streamer: Option<Streamer>,
    last_settings: Option<Settings>,
    last_load_debug: Option<LastLoadDebug>,
    media_title: String,
    backend: Option<MpvBackend>,
}

struct MpvBackend {
    ctx: usize,
    host: PlatformHost,
}

impl MpvBackend {
    fn ctx_ptr(&self) -> *mut mpv_handle {
        self.ctx as *mut mpv_handle
    }
}

#[derive(Debug, Clone)]
struct LastLoadDebug {
    platform: String,
    first_url: String,
    option_string: String,
    url_count: usize,
}

impl EmbeddedPlayerManager {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(EmbeddedPlayerState {
                snapshot: EmbeddedPlayerSnapshot::default(),
                last_bounds: None,
                last_streamer: None,
                last_settings: None,
                last_load_debug: None,
                media_title: String::new(),
                backend: None,
            })),
        }
    }

    pub fn get_state(&self) -> EmbeddedPlayerSnapshot {
        self.inner
            .lock()
            .expect("embedded player state poisoned")
            .snapshot
            .clone()
    }

    pub fn set_bounds(
        &self,
        app: &AppHandle,
        bounds: EmbeddedPlayerBounds,
    ) -> Result<EmbeddedPlayerSnapshot, String> {
        let snapshot = {
            let mut state = self
                .inner
                .lock()
                .map_err(|_| "播放器状态锁已损坏".to_string())?;
            state.last_bounds = Some(bounds.clone());
            let visible = state.snapshot.visible;
            if let Some(backend) = state.backend.as_mut() {
                backend.host.set_bounds(app, &bounds)?;
                backend.host.set_visible(app, visible)?;
            }
            state.snapshot.clone()
        };
        Ok(snapshot)
    }

    pub fn play(
        &self,
        app: &AppHandle,
        streamer: Streamer,
        settings: Settings,
        data: ExtractResult,
        media_title: String,
    ) -> Result<(), String> {
        let snapshot = {
            let mut state = self
                .inner
                .lock()
                .map_err(|_| "播放器状态锁已损坏".to_string())?;
            state.last_streamer = Some(streamer);
            state.last_settings = Some(settings);
            state.last_load_debug = Some(build_last_load_debug(&data));
            state.media_title = media_title.clone();
            state.snapshot.phase = "loading".to_string();
            state.snapshot.title = media_title.clone();
            state.snapshot.streamer_name = data.streamer_name.clone();
            state.snapshot.platform = data.platform.clone();
            state.snapshot.visible = true;
            state.snapshot.error_message.clear();
            let last_bounds = state.last_bounds.clone();
            let ctx = {
                let backend = self.ensure_backend(&mut state, app)?;
                if let Some(bounds) = last_bounds {
                    backend.host.set_bounds(app, &bounds)?;
                }
                backend.host.set_visible(app, true)?;
                configure_mpv_for_playback(backend.ctx_ptr(), &data, &media_title)?;
                load_stream_urls(backend.ctx_ptr(), &data)?;
                #[cfg(target_os = "macos")]
                backend.host.request_redraw();
                backend.ctx_ptr()
            };
            refresh_snapshot_from_mpv(&mut state.snapshot, ctx);
            state.snapshot.clone()
        };

        self.emit_state(app, snapshot);
        Ok(())
    }

    pub fn command(
        &self,
        app: &AppHandle,
        payload: EmbeddedPlayerCommandPayload,
    ) -> Result<EmbeddedPlayerSnapshot, String> {
        let command = ParsedCommand::from_payload(payload)?;

        match command {
            ParsedCommand::ReloadCurrent => self.reload_current(app),
            ParsedCommand::SetFullscreen(value) => {
                let window = app
                    .get_webview_window("main")
                    .ok_or_else(|| "未找到主窗口".to_string())?;
                window
                    .set_fullscreen(value)
                    .map_err(|err| format!("切换全屏失败：{err}"))?;
                let snapshot = self.get_state();
                self.emit_state(app, snapshot.clone());
                Ok(snapshot)
            }
            other => self.run_simple_command(app, other),
        }
    }

    fn reload_current(&self, app: &AppHandle) -> Result<EmbeddedPlayerSnapshot, String> {
        let (streamer, settings) = {
            let state = self
                .inner
                .lock()
                .map_err(|_| "播放器状态锁已损坏".to_string())?;
            let streamer = state
                .last_streamer
                .clone()
                .ok_or_else(|| "当前没有可重新加载的直播".to_string())?;
            let settings = state
                .last_settings
                .clone()
                .ok_or_else(|| "当前没有可重新加载的播放器配置".to_string())?;
            (streamer, settings)
        };

        let platform = if streamer.platform.trim().is_empty() {
            crate::infer_platform_from_target(&streamer.target).to_string()
        } else {
            crate::normalize_platform(&streamer.platform).to_string()
        };

        let data = crate::extract_play_info_for_platform_with_app(
            Some(app),
            &platform,
            &streamer.target,
            &settings.bilibili_cookie,
        )?;
        if !data.is_online {
            return Err("主播当前未开播".to_string());
        }
        if data.urls.is_empty() {
            return Err("未获取到可播放的直播地址".to_string());
        }
        let media_title = if data.title.trim().is_empty() {
            "Stream Hub Live".to_string()
        } else {
            data.title.clone()
        };
        self.play(app, streamer, settings, data, media_title)?;
        Ok(self.get_state())
    }

    fn run_simple_command(
        &self,
        app: &AppHandle,
        command: ParsedCommand,
    ) -> Result<EmbeddedPlayerSnapshot, String> {
        let snapshot = {
            let mut state = self
                .inner
                .lock()
                .map_err(|_| "播放器状态锁已损坏".to_string())?;
            let Some(ctx) = state.backend.as_ref().map(MpvBackend::ctx_ptr) else {
                return Ok(state.snapshot.clone());
            };
            match command {
                ParsedCommand::TogglePause => {
                    run_mpv_command(ctx, &["cycle", "pause"])?;
                }
                ParsedCommand::Stop => {
                    state.snapshot.visible = false;
                    state.snapshot.phase = "idle".to_string();
                    state.snapshot.paused = false;
                    if let Some(backend) = state.backend.as_mut() {
                        backend.host.set_visible(app, false)?;
                    }
                    run_mpv_command(ctx, &["stop"])?;
                }
                ParsedCommand::ToggleMute => {
                    run_mpv_command(ctx, &["cycle", "mute"])?;
                }
                ParsedCommand::SetVolume(value) => {
                    set_mpv_double_property(ctx, "volume", value)?;
                }
                ParsedCommand::ReloadCurrent | ParsedCommand::SetFullscreen(_) => {}
            }
            refresh_snapshot_from_mpv(&mut state.snapshot, ctx);
            state.snapshot.clone()
        };

        self.emit_state(app, snapshot.clone());
        Ok(snapshot)
    }

    fn ensure_backend<'a>(
        &self,
        state: &'a mut EmbeddedPlayerState,
        app: &AppHandle,
    ) -> Result<&'a mut MpvBackend, String> {
        if state.backend.is_none() {
            let mut host = PlatformHost::new(app)?;
            if let Some(bounds) = state.last_bounds.clone() {
                host.set_bounds(app, &bounds)?;
            }
            host.set_visible(app, false)?;
            let ctx = create_backend_context(app, host.embed_id())?;
            let manager = self.clone();
            let app_handle = app.clone();
            let ctx_for_thread = ctx as usize;
            thread::Builder::new()
                .name("embedded-mpv-events".to_string())
                .spawn(move || {
                    manager.event_loop(app_handle, ctx_for_thread as *mut mpv_handle);
                })
                .map_err(|err| format!("启动播放器事件线程失败：{err}"))?;
            #[cfg(target_os = "macos")]
            host.attach_render_context(app, ctx)?;
            state.backend = Some(MpvBackend {
                ctx: ctx as usize,
                host,
            });
        }
        Ok(state.backend.as_mut().expect("backend initialized"))
    }

    fn event_loop(&self, app: AppHandle, ctx: *mut mpv_handle) {
        loop {
            let event = unsafe { mpv_wait_event(ctx, -1.0) };
            if event.is_null() {
                break;
            }
            let event = unsafe { &*event };
            if event.event_id == MPV_EVENT_SHUTDOWN {
                self.with_state(|state| {
                    state.snapshot.phase = "idle".to_string();
                    state.snapshot.visible = false;
                    state.snapshot.paused = false;
                    state.snapshot.error_message.clear();
                    state.snapshot.clone()
                })
                .map(|snapshot| self.emit_state(&app, snapshot))
                .ok();
                break;
            }

            let mut emitted_error = None;
            let snapshot = self.with_state(|state| {
                match event.event_id {
                    MPV_EVENT_START_FILE => {
                        state.snapshot.phase = "loading".to_string();
                        state.snapshot.visible = true;
                    }
                    MPV_EVENT_FILE_LOADED | MPV_EVENT_PLAYBACK_RESTART => {
                        state.snapshot.phase = if state.snapshot.paused {
                            "paused".to_string()
                        } else {
                            "playing".to_string()
                        };
                        state.snapshot.visible = true;
                    }
                    MPV_EVENT_PAUSE => {
                        state.snapshot.phase = "paused".to_string();
                        state.snapshot.paused = true;
                    }
                    MPV_EVENT_UNPAUSE => {
                        state.snapshot.phase = "playing".to_string();
                        state.snapshot.paused = false;
                    }
                    MPV_EVENT_IDLE => {
                        if !state.snapshot.visible {
                            state.snapshot.phase = "idle".to_string();
                        }
                    }
                    MPV_EVENT_END_FILE => {
                        let end_file = unsafe { &*(event.data as *const mpv_event_end_file) };
                        if !state.snapshot.visible {
                            state.snapshot.phase = "idle".to_string();
                        } else if end_file.reason
                            == mpv_end_file_reason_MPV_END_FILE_REASON_ERROR as i32
                        {
                            state.snapshot.phase = "ended".to_string();
                            emitted_error = Some(format_end_file_error(
                                end_file,
                                state.last_load_debug.as_ref(),
                            ));
                        } else {
                            state.snapshot.phase = "ended".to_string();
                        }
                    }
                    MPV_EVENT_LOG_MESSAGE => {
                        let log = unsafe { &*(event.data as *const mpv_event_log_message) };
                        emitted_error = mpv_log_message_text(log);
                    }
                    _ => {}
                }
                refresh_snapshot_from_mpv(&mut state.snapshot, ctx);
                state.snapshot.clone()
            });

            if let Some(message) = emitted_error.filter(|value| !value.trim().is_empty()) {
                let _ = app.emit(PLAYER_EVENT_ERROR, message.clone());
                let _ = self.with_state(|state| {
                    state.snapshot.error_message = message;
                    state.snapshot.clone()
                });
            }

            if let Ok(snapshot) = snapshot {
                let _ = app.emit(PLAYER_EVENT_STATE, snapshot);
            }
        }
    }

    fn with_state<T>(&self, f: impl FnOnce(&mut EmbeddedPlayerState) -> T) -> Result<T, String> {
        let mut state = self
            .inner
            .lock()
            .map_err(|_| "播放器状态锁已损坏".to_string())?;
        Ok(f(&mut state))
    }

    fn emit_state(&self, app: &AppHandle, snapshot: EmbeddedPlayerSnapshot) {
        let _ = app.emit(PLAYER_EVENT_STATE, snapshot);
    }
}

impl Default for EmbeddedPlayerManager {
    fn default() -> Self {
        Self::new()
    }
}

enum ParsedCommand {
    TogglePause,
    Stop,
    ToggleMute,
    SetVolume(f64),
    ReloadCurrent,
    SetFullscreen(bool),
}

impl ParsedCommand {
    fn from_payload(payload: EmbeddedPlayerCommandPayload) -> Result<Self, String> {
        match payload.kind.as_str() {
            "togglePause" => Ok(Self::TogglePause),
            "stop" => Ok(Self::Stop),
            "toggleMute" => Ok(Self::ToggleMute),
            "setVolume" => Ok(Self::SetVolume(
                payload
                    .value
                    .as_ref()
                    .and_then(Value::as_f64)
                    .ok_or_else(|| "setVolume 需要数值 value".to_string())?
                    .clamp(0.0, 100.0),
            )),
            "reloadCurrent" => Ok(Self::ReloadCurrent),
            "setFullscreen" => Ok(Self::SetFullscreen(
                payload
                    .value
                    .as_ref()
                    .and_then(Value::as_bool)
                    .ok_or_else(|| "setFullscreen 需要布尔 value".to_string())?,
            )),
            other => Err(format!("不支持的播放器命令：{other}")),
        }
    }
}

fn create_mpv(_embed_id: isize) -> Result<*mut mpv_handle, String> {
    let ctx = unsafe { mpv_create() };
    if ctx.is_null() {
        return Err("初始化 libmpv 失败".to_string());
    }

    set_mpv_option_string(ctx, "terminal", "no")?;
    set_mpv_option_string(ctx, "osc", "no")?;
    set_mpv_option_string(ctx, "input-default-bindings", "no")?;
    set_mpv_option_string(ctx, "input-vo-keyboard", "no")?;
    set_mpv_option_string(ctx, "ytdl", "no")?;
    set_mpv_option_string(ctx, "idle", "yes")?;

    #[cfg(target_os = "macos")]
    set_mpv_option_string(ctx, "vo", "libmpv")?;

    #[cfg(not(target_os = "macos"))]
    {
        let wid = embed_id as i64;
        set_mpv_option_int64(ctx, "wid", wid)?;
    }

    let init_status = unsafe { mpv_initialize(ctx) };
    if init_status < 0 {
        unsafe { mpv_destroy(ctx) };
        return Err(format!(
            "初始化 libmpv 失败：{}",
            mpv_error_message(init_status)
        ));
    }

    request_mpv_log_messages(ctx, "error")?;
    Ok(ctx)
}

fn create_backend_context(app: &AppHandle, embed_id: isize) -> Result<*mut mpv_handle, String> {
    #[cfg(target_os = "macos")]
    {
        let window = app
            .get_webview_window("main")
            .ok_or_else(|| "未找到主窗口".to_string())?;
        let (tx, rx) = mpsc::channel();
        window
            .run_on_main_thread(move || {
                let result = create_mpv(embed_id).map(|ctx| ctx as usize);
                let _ = tx.send(result);
            })
            .map_err(|err| format!("初始化 libmpv 失败：{err}"))?;
        let ctx = rx
            .recv()
            .map_err(|_| "libmpv 初始化结果丢失".to_string())??;
        Ok(ctx as *mut mpv_handle)
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        create_mpv(embed_id)
    }
}

fn configure_mpv_for_playback(
    ctx: *mut mpv_handle,
    data: &ExtractResult,
    media_title: &str,
) -> Result<(), String> {
    set_mpv_string_property(ctx, "force-media-title", media_title)?;
    if data.platform == crate::PLATFORM_BILIBILI_LIVE {
        clear_mpv_string_property(ctx, "user-agent")?;
    } else if data.platform == crate::PLATFORM_HUYA {
        set_mpv_string_property(ctx, "user-agent", crate::HUYA_USER_AGENT)?;
    } else if data.platform == crate::PLATFORM_DOUYIN_LIVE {
        set_mpv_string_property(ctx, "user-agent", crate::DOUYIN_USER_AGENT)?;
    } else {
        clear_mpv_string_property(ctx, "user-agent")?;
    }

    Ok(())
}

fn load_stream_urls(ctx: *mut mpv_handle, data: &ExtractResult) -> Result<(), String> {
    let option_string = build_loadfile_option_string(data);

    for (index, url) in data.urls.iter().enumerate() {
        let mut args = vec![
            "loadfile".to_string(),
            url.clone(),
            if index == 0 {
                "replace".to_string()
            } else {
                "append".to_string()
            },
        ];

        if !option_string.is_empty() {
            args.push("-1".to_string());
            args.push(option_string.clone());
        }

        run_mpv_command_dynamic(ctx, &args).map_err(|err| {
            format!(
                "mpv loadfile 失败：{err} | platform={} | index={} | url={} | options={}",
                data.platform, index, url, option_string
            )
        })?;
    }

    Ok(())
}

fn build_last_load_debug(data: &ExtractResult) -> LastLoadDebug {
    LastLoadDebug {
        platform: data.platform.clone(),
        first_url: data.urls.first().cloned().unwrap_or_default(),
        option_string: build_loadfile_option_string(data),
        url_count: data.urls.len(),
    }
}

fn build_loadfile_option_string(data: &ExtractResult) -> String {
    let mut options = vec![
        "ytdl=no".to_string(),
        "stream-lavf-o=reconnect_streamed=yes".to_string(),
    ];

    match data.platform.as_str() {
        crate::PLATFORM_BILIBILI_LIVE => {
            options.push("referrer=https://live.bilibili.com/".to_string());
        }
        crate::PLATFORM_HUYA => {
            options.push("referrer=https://www.huya.com/".to_string());
            options.push(format!("user-agent={}", crate::HUYA_USER_AGENT));
        }
        crate::PLATFORM_DOUYIN_LIVE => {
            options.push("referrer=https://live.douyin.com/".to_string());
            options.push(format!("user-agent={}", crate::DOUYIN_USER_AGENT));
        }
        _ => {}
    }

    options.join(",")
}

fn run_mpv_command(ctx: *mut mpv_handle, args: &[&str]) -> Result<(), String> {
    let strings = args
        .iter()
        .map(|value| CString::new(*value).map_err(|_| format!("无效的 mpv 命令参数：{value}")))
        .collect::<Result<Vec<_>, _>>()?;
    let mut raw = strings
        .iter()
        .map(|value| value.as_ptr())
        .collect::<Vec<_>>();
    raw.push(ptr::null());

    let status = unsafe { mpv_command(ctx, raw.as_mut_ptr()) };
    if status < 0 {
        Err(mpv_error_message(status))
    } else {
        Ok(())
    }
}

fn run_mpv_command_dynamic(ctx: *mut mpv_handle, args: &[String]) -> Result<(), String> {
    let borrowed = args.iter().map(String::as_str).collect::<Vec<_>>();
    run_mpv_command(ctx, &borrowed)
}

fn set_mpv_option_string(ctx: *mut mpv_handle, name: &str, value: &str) -> Result<(), String> {
    let name = CString::new(name).map_err(|_| format!("无效的 mpv 选项名：{name}"))?;
    let value = CString::new(value).map_err(|_| format!("无效的 mpv 选项值：{value}"))?;
    let status = unsafe { mpv_set_option_string(ctx, name.as_ptr(), value.as_ptr()) };
    if status < 0 {
        Err(format!("{name:?} 设置失败：{}", mpv_error_message(status)))
    } else {
        Ok(())
    }
}

fn set_mpv_option_int64(ctx: *mut mpv_handle, name: &str, value: i64) -> Result<(), String> {
    let name = CString::new(name).map_err(|_| format!("无效的 mpv 选项名：{name}"))?;
    let status = unsafe {
        mpv_set_option(
            ctx,
            name.as_ptr(),
            mpv_format_MPV_FORMAT_INT64,
            (&value as *const i64).cast::<c_void>().cast_mut(),
        )
    };
    if status < 0 {
        Err(format!("{name:?} 设置失败：{}", mpv_error_message(status)))
    } else {
        Ok(())
    }
}

fn request_mpv_log_messages(ctx: *mut mpv_handle, level: &str) -> Result<(), String> {
    let level = CString::new(level).map_err(|_| format!("无效的日志级别：{level}"))?;
    let status = unsafe { mpv_request_log_messages(ctx, level.as_ptr()) };
    if status < 0 {
        Err(format!("启用 mpv 日志失败：{}", mpv_error_message(status)))
    } else {
        Ok(())
    }
}

fn set_mpv_string_property(ctx: *mut mpv_handle, name: &str, value: &str) -> Result<(), String> {
    let name = CString::new(name).map_err(|_| format!("无效的 mpv 属性名：{name}"))?;
    let value = CString::new(value).map_err(|_| format!("无效的 mpv 属性值：{value}"))?;
    let status = unsafe {
        mpv_set_property(
            ctx,
            name.as_ptr(),
            mpv_format_MPV_FORMAT_STRING,
            (&value.as_ptr() as *const *const i8)
                .cast::<c_void>()
                .cast_mut(),
        )
    };
    if status < 0 {
        Err(format!("{name:?} 设置失败：{}", mpv_error_message(status)))
    } else {
        Ok(())
    }
}

fn clear_mpv_string_property(ctx: *mut mpv_handle, name: &str) -> Result<(), String> {
    set_mpv_string_property(ctx, name, "")
}

fn set_mpv_double_property(ctx: *mut mpv_handle, name: &str, value: f64) -> Result<(), String> {
    let name = CString::new(name).map_err(|_| format!("无效的 mpv 属性名：{name}"))?;
    let status = unsafe {
        mpv_set_property(
            ctx,
            name.as_ptr(),
            mpv_format_MPV_FORMAT_DOUBLE,
            (&value as *const f64).cast::<c_void>().cast_mut(),
        )
    };
    if status < 0 {
        Err(format!("{name:?} 设置失败：{}", mpv_error_message(status)))
    } else {
        Ok(())
    }
}

fn refresh_snapshot_from_mpv(snapshot: &mut EmbeddedPlayerSnapshot, ctx: *mut mpv_handle) {
    if let Some(paused) = get_mpv_flag_property(ctx, "pause") {
        snapshot.paused = paused;
        if snapshot.visible {
            snapshot.phase = if paused {
                "paused".to_string()
            } else if snapshot.phase == "loading" {
                "loading".to_string()
            } else {
                "playing".to_string()
            };
        }
    }

    if let Some(muted) = get_mpv_flag_property(ctx, "mute") {
        snapshot.muted = muted;
    }

    if let Some(volume) = get_mpv_double_property(ctx, "volume") {
        snapshot.volume = volume.clamp(0.0, 100.0);
    }

    if let Some(title) =
        get_mpv_string_property(ctx, "media-title").filter(|value| !value.is_empty())
    {
        snapshot.title = title;
    }
}

fn get_mpv_flag_property(ctx: *mut mpv_handle, name: &str) -> Option<bool> {
    let name = CString::new(name).ok()?;
    let mut value = 0i32;
    let status = unsafe {
        mpv_get_property(
            ctx,
            name.as_ptr(),
            mpv_format_MPV_FORMAT_FLAG,
            (&mut value as *mut i32).cast(),
        )
    };
    (status >= 0).then_some(value != 0)
}

fn get_mpv_double_property(ctx: *mut mpv_handle, name: &str) -> Option<f64> {
    let name = CString::new(name).ok()?;
    let mut value = 0.0f64;
    let status = unsafe {
        mpv_get_property(
            ctx,
            name.as_ptr(),
            mpv_format_MPV_FORMAT_DOUBLE,
            (&mut value as *mut f64).cast(),
        )
    };
    (status >= 0).then_some(value)
}

fn get_mpv_string_property(ctx: *mut mpv_handle, name: &str) -> Option<String> {
    let name = CString::new(name).ok()?;
    let mut value = ptr::null_mut::<i8>();
    let status = unsafe {
        mpv_get_property(
            ctx,
            name.as_ptr(),
            mpv_format_MPV_FORMAT_STRING,
            (&mut value as *mut *mut i8).cast(),
        )
    };
    if status < 0 || value.is_null() {
        return None;
    }

    let owned = unsafe { CStr::from_ptr(value).to_string_lossy().to_string() };
    unsafe { libmpv_sys::mpv_free(value.cast()) };
    Some(owned)
}

fn mpv_error_message(status: i32) -> String {
    unsafe {
        let raw = mpv_error_string(status);
        if raw.is_null() {
            "未知 mpv 错误".to_string()
        } else {
            CStr::from_ptr(raw).to_string_lossy().to_string()
        }
    }
}

fn mpv_log_message_text(log: &mpv_event_log_message) -> Option<String> {
    let level = unsafe {
        (!log.level.is_null()).then(|| CStr::from_ptr(log.level).to_string_lossy().to_string())
    }
    .unwrap_or_else(|| "error".to_string());
    let prefix = unsafe {
        (!log.prefix.is_null()).then(|| CStr::from_ptr(log.prefix).to_string_lossy().to_string())
    }
    .unwrap_or_else(|| "mpv".to_string());
    let text = unsafe {
        (!log.text.is_null()).then(|| {
            CStr::from_ptr(log.text)
                .to_string_lossy()
                .trim()
                .to_string()
        })
    }?;

    if text.is_empty() {
        None
    } else {
        Some(format!("[{prefix}/{level}] {text}"))
    }
}

fn format_end_file_error(
    end_file: &mpv_event_end_file,
    load_debug: Option<&LastLoadDebug>,
) -> String {
    let mut message = format!(
        "mpv 播放失败：{} | end_reason={} | error_code={}",
        mpv_error_message(end_file.error),
        end_file.reason,
        end_file.error
    );

    if let Some(debug) = load_debug {
        message.push_str(&format!(
            " | platform={} | url_count={} | first_url={}",
            debug.platform, debug.url_count, debug.first_url
        ));
        if !debug.option_string.is_empty() {
            message.push_str(&format!(" | options={}", debug.option_string));
        }
    }

    message
}

#[cfg(test)]
mod tests {
    use super::{EmbeddedPlayerCommandPayload, ParsedCommand};

    #[test]
    fn parses_libmpv_commands() {
        let payload = EmbeddedPlayerCommandPayload {
            kind: "setVolume".to_string(),
            value: Some(serde_json::json!(72.5)),
        };
        match ParsedCommand::from_payload(payload).expect("command should parse") {
            ParsedCommand::SetVolume(value) => assert_eq!(value, 72.5),
            _ => panic!("unexpected command variant"),
        }
    }

    #[test]
    fn rejects_missing_boolean_value() {
        let payload = EmbeddedPlayerCommandPayload {
            kind: "setFullscreen".to_string(),
            value: None,
        };
        assert!(ParsedCommand::from_payload(payload).is_err());
    }
}
