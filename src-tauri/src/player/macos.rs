use crate::player::EmbeddedPlayerBounds;
use libmpv_sys::{
    mpv_error_string, mpv_handle, mpv_opengl_fbo, mpv_opengl_init_params, mpv_render_context,
    mpv_render_context_create, mpv_render_context_render, mpv_render_context_report_swap,
    mpv_render_context_set_update_callback, mpv_render_context_update, mpv_render_param,
    mpv_render_param_type_MPV_RENDER_PARAM_API_TYPE,
    mpv_render_param_type_MPV_RENDER_PARAM_BLOCK_FOR_TARGET_TIME,
    mpv_render_param_type_MPV_RENDER_PARAM_FLIP_Y, mpv_render_param_type_MPV_RENDER_PARAM_INVALID,
    mpv_render_param_type_MPV_RENDER_PARAM_OPENGL_FBO,
    mpv_render_param_type_MPV_RENDER_PARAM_OPENGL_INIT_PARAMS,
    mpv_render_update_flag_MPV_RENDER_UPDATE_FRAME, MPV_RENDER_API_TYPE_OPENGL,
};
use objc2::runtime::AnyObject;
use objc2::{class, msg_send};
use objc2_foundation::{NSPoint, NSRect, NSSize};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::ffi::CString;
use std::ffi::{c_char, c_void, CStr};
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::OnceLock;
use tauri::{AppHandle, Emitter, Manager};

const PLAYER_EVENT_ERROR: &str = "embedded-player-error";
const RTLD_LAZY: i32 = 0x1;

unsafe extern "C" {
    fn dlopen(path: *const c_char, mode: i32) -> *mut c_void;
    fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
}

static OPENGL_HANDLE: OnceLock<usize> = OnceLock::new();

pub struct MacHost {
    webview: usize,
    gl_view: usize,
    gl_context: usize,
    render_state: Option<Box<RenderCallbackState>>,
}

struct RenderCallbackState {
    app: AppHandle,
    gl_context: usize,
    render_context: AtomicUsize,
    pending: AtomicBool,
    visible: AtomicBool,
    width: AtomicI32,
    height: AtomicI32,
}

impl MacHost {
    pub fn new(app: &AppHandle) -> Result<Self, String> {
        let window = app
            .get_webview_window("main")
            .ok_or_else(|| "未找到主窗口".to_string())?;
        let raw = window
            .window_handle()
            .map_err(|err| err.to_string())?
            .as_raw();
        let RawWindowHandle::AppKit(handle) = raw else {
            return Err("当前窗口不是 AppKit 句柄".to_string());
        };
        let webview = handle.ns_view.as_ptr() as usize;

        let (tx, rx) = mpsc::channel::<Result<(usize, usize), String>>();
        window
            .run_on_main_thread(move || unsafe {
                let webview = webview as *mut AnyObject;
                let superview: *mut AnyObject = msg_send![webview, superview];
                if superview.is_null() {
                    let _ = tx.send(Err("未找到主 WebView 的父视图".to_string()));
                    return;
                }

                let frame: NSRect = msg_send![webview, frame];
                let pixel_format_class = class!(NSOpenGLPixelFormat);
                let pixel_format_alloc: *mut AnyObject = msg_send![pixel_format_class, alloc];
                let mut attrs = [
                    99u32,     // NSOpenGLPFAOpenGLProfile
                    0x3200u32, // NSOpenGLProfileVersion3_2Core
                    5u32,      // NSOpenGLPFADoubleBuffer
                    8u32,      // NSOpenGLPFAColorSize
                    24u32, 11u32, // NSOpenGLPFAAlphaSize
                    8u32, 12u32, // NSOpenGLPFADepthSize
                    16u32, 73u32, // NSOpenGLPFAAccelerated
                    0u32,
                ];
                let pixel_format: *mut AnyObject = msg_send![
                    pixel_format_alloc,
                    initWithAttributes: attrs.as_mut_ptr()
                ];
                if pixel_format.is_null() {
                    let _ = tx.send(Err("创建 NSOpenGLPixelFormat 失败".to_string()));
                    return;
                }

                let view_class = class!(NSOpenGLView);
                let view_alloc: *mut AnyObject = msg_send![view_class, alloc];
                let gl_view: *mut AnyObject = msg_send![
                    view_alloc,
                    initWithFrame: frame,
                    pixelFormat: pixel_format
                ];
                if gl_view.is_null() {
                    let _ = tx.send(Err("创建 NSOpenGLView 失败".to_string()));
                    return;
                }

                let _: () = msg_send![gl_view, setWantsBestResolutionOpenGLSurface: true];
                let gl_context: *mut AnyObject = msg_send![gl_view, openGLContext];
                if gl_context.is_null() {
                    let _ = tx.send(Err("获取 NSOpenGLContext 失败".to_string()));
                    return;
                }

                let one = 1i32;
                let _: () = msg_send![gl_context, setValues: &one, forParameter: 222isize];
                let _: () = msg_send![gl_context, setView: gl_view];
                let _: () = msg_send![gl_view, setHidden: true];
                let below = -1isize;
                let _: () = msg_send![superview, addSubview: gl_view, positioned: below, relativeTo: webview];
                let _ = tx.send(Ok((gl_view as usize, gl_context as usize)));
            })
            .map_err(|err| format!("创建播放器 OpenGL 宿主失败：{err}"))?;

        let (gl_view, gl_context) = rx
            .recv()
            .map_err(|_| "播放器 OpenGL 宿主创建结果丢失".to_string())??;

        Ok(Self {
            webview,
            gl_view,
            gl_context,
            render_state: None,
        })
    }

    pub fn embed_id(&self) -> isize {
        0
    }

    pub fn attach_render_context(
        &mut self,
        app: &AppHandle,
        mpv: *mut mpv_handle,
    ) -> Result<(), String> {
        if self.render_state.is_some() {
            return Ok(());
        }

        let window = app
            .get_webview_window("main")
            .ok_or_else(|| "未找到主窗口".to_string())?;
        let gl_context = self.gl_context;
        let state = Box::new(RenderCallbackState {
            app: app.clone(),
            gl_context,
            render_context: AtomicUsize::new(0),
            pending: AtomicBool::new(false),
            visible: AtomicBool::new(false),
            width: AtomicI32::new(1),
            height: AtomicI32::new(1),
        });
        let state_ptr = (&*state) as *const RenderCallbackState as usize;
        let mpv_ptr = mpv as usize;

        let (tx, rx) = mpsc::channel::<Result<(), String>>();
        window
            .run_on_main_thread(move || unsafe {
                let gl_context = gl_context as *mut AnyObject;
                let render_state = &*(state_ptr as *const RenderCallbackState);
                let mpv = mpv_ptr as *mut mpv_handle;

                let _: () = msg_send![gl_context, makeCurrentContext];

                let init_params = mpv_opengl_init_params {
                    get_proc_address: Some(get_proc_address),
                    get_proc_address_ctx: ptr::null_mut(),
                    extra_exts: ptr::null(),
                };
                let mut render_context = ptr::null_mut::<mpv_render_context>();
                let mut init_params_value = init_params;
                let mut params = [
                    mpv_render_param {
                        type_: mpv_render_param_type_MPV_RENDER_PARAM_API_TYPE,
                        data: MPV_RENDER_API_TYPE_OPENGL.as_ptr().cast_mut().cast(),
                    },
                    mpv_render_param {
                        type_: mpv_render_param_type_MPV_RENDER_PARAM_OPENGL_INIT_PARAMS,
                        data: (&mut init_params_value as *mut mpv_opengl_init_params).cast(),
                    },
                    mpv_render_param {
                        type_: mpv_render_param_type_MPV_RENDER_PARAM_INVALID,
                        data: ptr::null_mut(),
                    },
                ];

                let status =
                    mpv_render_context_create(&mut render_context, mpv, params.as_mut_ptr());
                if status < 0 || render_context.is_null() {
                    let _ = tx.send(Err(format!(
                        "创建 mpv render context 失败：{}",
                        mpv_error_message(status)
                    )));
                    return;
                }

                render_state
                    .render_context
                    .store(render_context as usize, Ordering::Release);
                mpv_render_context_set_update_callback(
                    render_context,
                    Some(render_update_callback),
                    state_ptr as *mut c_void,
                );
                let _ = tx.send(Ok(()));
            })
            .map_err(|err| format!("创建 mpv render context 失败：{err}"))?;

        rx.recv()
            .map_err(|_| "mpv render context 创建结果丢失".to_string())??;
        self.render_state = Some(state);
        Ok(())
    }

    pub fn set_bounds(
        &mut self,
        app: &AppHandle,
        bounds: &EmbeddedPlayerBounds,
    ) -> Result<(), String> {
        let window = app
            .get_webview_window("main")
            .ok_or_else(|| "未找到主窗口".to_string())?;
        let webview = self.webview;
        let gl_view = self.gl_view;
        let gl_context = self.gl_context;
        let bounds = bounds.clone();
        let render_state_ptr = self
            .render_state
            .as_ref()
            .map(|state| (&**state) as *const RenderCallbackState as usize);
        let (tx, rx) = mpsc::channel::<Result<bool, String>>();
        window
            .run_on_main_thread(move || unsafe {
                let webview = webview as *mut AnyObject;
                let gl_view = gl_view as *mut AnyObject;
                let gl_context = gl_context as *mut AnyObject;
                let superview: *mut AnyObject = msg_send![gl_view, superview];
                if superview.is_null() {
                    let _ = tx.send(Err("播放器 OpenGL 视图没有父视图".to_string()));
                    return;
                }

                if webview.is_null() {
                    let _ = tx.send(Err("未找到主 WebView".to_string()));
                    return;
                }

                let webview_frame: NSRect = msg_send![webview, frame];
                let scale = if bounds.scale_factor <= 0.0 {
                    1.0
                } else {
                    bounds.scale_factor
                };
                let viewport_height = if bounds.viewport_height > 0.0 {
                    bounds.viewport_height / scale
                } else {
                    webview_frame.size.height
                };
                let width = bounds.width / scale;
                let height = bounds.height / scale;
                let x = webview_frame.origin.x + (bounds.x / scale);
                let y = webview_frame.origin.y + viewport_height - (bounds.y / scale) - height;
                let frame = NSRect::new(NSPoint::new(x, y), NSSize::new(width, height));
                let next_width = bounds.width.round().max(1.0) as i32;
                let next_height = bounds.height.round().max(1.0) as i32;
                let mut size_changed = true;

                if let Some(ptr) = render_state_ptr {
                    let render_state = &*(ptr as *const RenderCallbackState);
                    let previous_width = render_state.width.load(Ordering::Acquire);
                    let previous_height = render_state.height.load(Ordering::Acquire);
                    size_changed = previous_width != next_width || previous_height != next_height;
                    render_state.width.store(next_width, Ordering::Release);
                    render_state.height.store(next_height, Ordering::Release);
                }

                let _: () = msg_send![gl_view, setFrame: frame];
                if size_changed {
                    let _: () = msg_send![gl_view, update];
                    let _: () = msg_send![gl_context, update];
                }

                let _ = tx.send(Ok(size_changed));
            })
            .map_err(|err| format!("更新播放器位置失败：{err}"))?;
        let needs_redraw = rx
            .recv()
            .map_err(|_| "播放器位置更新结果丢失".to_string())??;

        if needs_redraw {
            if let Some(state) = self.render_state.as_ref() {
                schedule_render(state.as_ref());
            }
        }

        Ok(())
    }

    pub fn set_visible(&mut self, app: &AppHandle, visible: bool) -> Result<(), String> {
        let window = app
            .get_webview_window("main")
            .ok_or_else(|| "未找到主窗口".to_string())?;
        let gl_view = self.gl_view;
        let render_state_ptr = self
            .render_state
            .as_ref()
            .map(|state| (&**state) as *const RenderCallbackState as usize);
        let (tx, rx) = mpsc::channel::<Result<(), String>>();
        window
            .run_on_main_thread(move || unsafe {
                let gl_view = gl_view as *mut AnyObject;
                let _: () = msg_send![gl_view, setHidden: !visible];
                if let Some(ptr) = render_state_ptr {
                    let render_state = &*(ptr as *const RenderCallbackState);
                    render_state.visible.store(visible, Ordering::Release);
                }
                let _ = tx.send(Ok(()));
            })
            .map_err(|err| format!("更新播放器显示状态失败：{err}"))?;
        rx.recv()
            .map_err(|_| "播放器显示状态更新结果丢失".to_string())??;

        if visible {
            if let Some(state) = self.render_state.as_ref() {
                schedule_render(state.as_ref());
            }
        }

        Ok(())
    }

    pub fn request_redraw(&self) {
        if let Some(state) = self.render_state.as_ref() {
            schedule_render(state.as_ref());
        }
    }
}

unsafe extern "C" fn render_update_callback(cb_ctx: *mut c_void) {
    if cb_ctx.is_null() {
        return;
    }
    let state = &*(cb_ctx as *const RenderCallbackState);
    schedule_render(state);
}

fn schedule_render(state: &RenderCallbackState) {
    if state.pending.swap(true, Ordering::AcqRel) {
        return;
    }

    let state_ptr = state as *const RenderCallbackState as usize;
    if let Some(window) = state.app.get_webview_window("main") {
        let _ = window.run_on_main_thread(move || unsafe {
            let state = &*(state_ptr as *const RenderCallbackState);
            state.pending.store(false, Ordering::Release);
            render_on_main_thread(state);
        });
    } else {
        state.pending.store(false, Ordering::Release);
    }
}

unsafe fn render_on_main_thread(state: &RenderCallbackState) {
    let render_context = state.render_context.load(Ordering::Acquire) as *mut mpv_render_context;
    if render_context.is_null() {
        return;
    }

    let gl_context = state.gl_context as *mut AnyObject;
    let _: () = msg_send![gl_context, makeCurrentContext];
    let update_flags = mpv_render_context_update(render_context);
    if update_flags & (mpv_render_update_flag_MPV_RENDER_UPDATE_FRAME as u64) == 0 {
        return;
    }

    let width = state.width.load(Ordering::Acquire).max(1);
    let height = state.height.load(Ordering::Acquire).max(1);
    let mut fbo = mpv_opengl_fbo {
        fbo: 0,
        w: width,
        h: height,
        internal_format: 0,
    };
    let mut flip = 1i32;
    let mut block_for_target_time = 0i32;
    let mut params = [
        mpv_render_param {
            type_: mpv_render_param_type_MPV_RENDER_PARAM_OPENGL_FBO,
            data: (&mut fbo as *mut mpv_opengl_fbo).cast(),
        },
        mpv_render_param {
            type_: mpv_render_param_type_MPV_RENDER_PARAM_FLIP_Y,
            data: (&mut flip as *mut i32).cast(),
        },
        mpv_render_param {
            type_: mpv_render_param_type_MPV_RENDER_PARAM_BLOCK_FOR_TARGET_TIME,
            data: (&mut block_for_target_time as *mut i32).cast(),
        },
        mpv_render_param {
            type_: mpv_render_param_type_MPV_RENDER_PARAM_INVALID,
            data: ptr::null_mut(),
        },
    ];

    let status = mpv_render_context_render(render_context, params.as_mut_ptr());
    if status < 0 {
        let _ = state.app.emit(
            PLAYER_EVENT_ERROR,
            format!("mpv render 失败：{}", mpv_error_message(status)),
        );
        return;
    }

    let _: () = msg_send![gl_context, flushBuffer];
    mpv_render_context_report_swap(render_context);
}

unsafe extern "C" fn get_proc_address(_ctx: *mut c_void, name: *const c_char) -> *mut c_void {
    if name.is_null() {
        return ptr::null_mut();
    }
    let handle = *OPENGL_HANDLE.get_or_init(|| unsafe {
        let path = CString::new("/System/Library/Frameworks/OpenGL.framework/OpenGL")
            .expect("valid OpenGL framework path");
        dlopen(path.as_ptr(), RTLD_LAZY) as usize
    });
    if handle == 0 {
        return ptr::null_mut();
    }
    dlsym(handle as *mut c_void, name)
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
