use std::ptr::NonNull;

use winit::dpi::PhysicalSize;
use winit::raw_window_handle::{
    HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle,
};
use winit::window::Window;

pub(crate) enum SurfaceOwner {
    X11,
    Wayland {
        egl: WaylandEglWindow,
        overlay: Option<crate::text_overlay::WaylandTextOverlay>,
    },
}

pub(crate) struct WaylandEglWindow {
    _library: libloading::Library,
    handle: NonNull<std::ffi::c_void>,
    resize: unsafe extern "C" fn(*mut std::ffi::c_void, i32, i32, i32, i32),
    destroy: unsafe extern "C" fn(*mut std::ffi::c_void),
}

impl WaylandEglWindow {
    fn create(surface: NonNull<std::ffi::c_void>, width: u32, height: u32) -> Result<Self, String> {
        type Create =
            unsafe extern "C" fn(*mut std::ffi::c_void, i32, i32) -> *mut std::ffi::c_void;
        // SAFETY: the library is kept alive by the returned owner and all
        // function pointers are obtained from that library.
        let library = unsafe { libloading::Library::new("libwayland-egl.so.1") }
            .map_err(|error| format!("load libwayland-egl: {error}"))?;
        // SAFETY: names and signatures are the public wayland-egl ABI.
        let create: libloading::Symbol<Create> = unsafe { library.get(b"wl_egl_window_create\0") }
            .map_err(|error| format!("resolve wl_egl_window_create: {error}"))?;
        // SAFETY: as above.
        let resize: libloading::Symbol<
            unsafe extern "C" fn(*mut std::ffi::c_void, i32, i32, i32, i32),
        > = unsafe { library.get(b"wl_egl_window_resize\0") }
            .map_err(|error| format!("resolve wl_egl_window_resize: {error}"))?;
        // SAFETY: as above.
        let destroy: libloading::Symbol<unsafe extern "C" fn(*mut std::ffi::c_void)> =
            unsafe { library.get(b"wl_egl_window_destroy\0") }
                .map_err(|error| format!("resolve wl_egl_window_destroy: {error}"))?;
        // SAFETY: `surface` is owned by the live winit Window; dimensions fit
        // the signed Wayland EGL ABI after the checks below.
        let handle = NonNull::new(unsafe {
            create(
                surface.as_ptr(),
                dimension_i32(width)?,
                dimension_i32(height)?,
            )
        })
        .ok_or_else(|| "wl_egl_window_create returned null".to_owned())?;
        Ok(Self {
            resize: *resize,
            destroy: *destroy,
            handle,
            _library: library,
        })
    }

    fn resize_to(&self, width: u32, height: u32) -> Result<(), String> {
        // SAFETY: this EGL window remains alive until this owner is dropped.
        unsafe {
            (self.resize)(
                self.handle.as_ptr(),
                dimension_i32(width)?,
                dimension_i32(height)?,
                0,
                0,
            )
        };
        Ok(())
    }
}

impl Drop for WaylandEglWindow {
    fn drop(&mut self) {
        // SAFETY: the handle was created by this library and is destroyed once.
        unsafe { (self.destroy)(self.handle.as_ptr()) };
    }
}

impl SurfaceOwner {
    pub(crate) fn install(window: &Window, size: PhysicalSize<u32>) -> Result<Self, String> {
        let display = window
            .display_handle()
            .map_err(|error| format!("get host display handle: {error}"))?
            .as_raw();
        let raw_window = window
            .window_handle()
            .map_err(|error| format!("get host window handle: {error}"))?
            .as_raw();
        match (display, raw_window) {
            (RawDisplayHandle::Xlib(display), RawWindowHandle::Xlib(window_handle)) => {
                let display = display
                    .display
                    .ok_or_else(|| "winit returned a null Xlib display".to_owned())?;
                // SAFETY: winit owns the X connection and window, and ClientApp
                // retains the Window until after the runtime surface is cleared.
                let surface = unsafe {
                    roblox_runtime::graphics::HostSurface::xlib(
                        display.as_ptr(),
                        window_handle.window as u64,
                        size.width,
                        size.height,
                    )
                }
                .map_err(|error| error.to_string())?;
                roblox_runtime::graphics::install_surface(surface);
                Ok(Self::X11)
            }
            (RawDisplayHandle::Wayland(display), RawWindowHandle::Wayland(window_handle)) => {
                let egl = WaylandEglWindow::create(window_handle.surface, size.width, size.height)?;
                let overlay = match crate::text_overlay::WaylandTextOverlay::create(
                    display.display.as_ptr(),
                    window_handle.surface.as_ptr(),
                ) {
                    Ok(overlay) => Some(overlay),
                    Err(error) => {
                        eprintln!("rusty-blox: text overlay unavailable: {error}");
                        None
                    }
                };
                // SAFETY: winit owns the display/surface, while this owner
                // keeps the derived wl_egl_window alive through engine shutdown.
                let surface = unsafe {
                    roblox_runtime::graphics::HostSurface::wayland(
                        display.display.as_ptr(),
                        window_handle.surface.as_ptr(),
                        egl.handle.as_ptr(),
                        size.width,
                        size.height,
                    )
                }
                .map_err(|error| error.to_string())?;
                roblox_runtime::graphics::install_surface(surface);
                Ok(Self::Wayland { egl, overlay })
            }
            (display, window) => Err(format!(
                "unsupported winit handles: display {display:?}, window {window:?}; expected Xlib or Wayland"
            )),
        }
    }

    pub(crate) fn resize(&self, width: u32, height: u32) -> Result<(), String> {
        if let Self::Wayland { egl, .. } = self {
            egl.resize_to(width, height)?;
        }
        Ok(())
    }

    pub(crate) fn update_text_overlay(
        &mut self,
        text: &str,
        caret: usize,
        info: roblox_runtime::jni::game_activity::RawTextBoxInfo,
        scale_factor: f64,
    ) {
        if let Self::Wayland { overlay: Some(overlay), .. } = self {
            if let Err(error) = overlay.update(text, caret, info, scale_factor) {
                eprintln!("rusty-blox: text overlay update failed: {error}");
            }
        }
    }

    pub(crate) fn hide_text_overlay(&mut self) {
        if let Self::Wayland { overlay: Some(overlay), .. } = self {
            if let Err(error) = overlay.hide() {
                eprintln!("rusty-blox: hide text overlay failed: {error}");
            }
        }
    }
}

pub(crate) fn dimension_i32(value: u32) -> Result<i32, String> {
    i32::try_from(value).map_err(|_| "host window dimension exceeds platform limits".into())
}
