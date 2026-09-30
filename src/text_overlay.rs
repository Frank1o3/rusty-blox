//! A transparent Wayland subsurface that paints Roblox's focused text editor.
//!
//! Roblox stops painting the contents of a focused Android TextBox and expects
//! the platform IME's EditText to paint them. Winit owns the toplevel surface,
//! so this client creates a sibling subsurface on Winit's existing Wayland
//! connection and gives it an empty input region. Keyboard and pointer input
//! continue to reach the game window.

use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::fs::File;
use std::os::fd::{AsRawFd, FromRawFd};
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};

use gtk4::cairo::{Context, FontSlant, FontWeight, Format, ImageSurface, Operator};
use libloading::Library;
use roblox_runtime::jni::game_activity::RawTextBoxInfo;

const MAX_BUFFERS: usize = 3;
const WL_MARSHAL_FLAG_DESTROY: u32 = 1;
const WL_SHM_FORMAT_ARGB8888: c_int = 0;

type Proxy = c_void;
type Interface = c_void;
type Queue = c_void;
type MarshalFlags = unsafe extern "C" fn(
    *mut Proxy,
    u32,
    *const Interface,
    u32,
    u32,
    ...,
) -> *mut Proxy;
type AddListener = unsafe extern "C" fn(*mut Proxy, *const *const c_void, *mut c_void) -> c_int;
type SetQueue = unsafe extern "C" fn(*mut Proxy, *mut Queue);
type ProxyVersion = unsafe extern "C" fn(*mut Proxy) -> u32;
type ProxyDestroy = unsafe extern "C" fn(*mut Proxy);

#[repr(C)]
struct RegistryListener {
    global: unsafe extern "C" fn(*mut c_void, *mut Proxy, u32, *const c_char, u32),
    global_remove: unsafe extern "C" fn(*mut c_void, *mut Proxy, u32),
}

#[repr(C)]
struct BufferListener {
    release: unsafe extern "C" fn(*mut c_void, *mut Proxy),
}

struct Interfaces {
    compositor: *const Interface,
    subcompositor: *const Interface,
    shm: *const Interface,
    surface: *const Interface,
    subsurface: *const Interface,
    region: *const Interface,
    shm_pool: *const Interface,
    buffer: *const Interface,
}

#[derive(Default)]
struct Bindings {
    compositor: *mut Proxy,
    compositor_version: u32,
    subcompositor: *mut Proxy,
    shm: *mut Proxy,
}

struct BindContext {
    marshal: MarshalFlags,
    interfaces: Interfaces,
    bindings: Bindings,
}

unsafe extern "C" fn registry_global(
    data: *mut c_void,
    registry: *mut Proxy,
    name: u32,
    interface_name: *const c_char,
    version: u32,
) {
    if data.is_null() || interface_name.is_null() {
        return;
    }
    // SAFETY: the registry listener's data points to the live BindContext in
    // `WaylandTextOverlay::new`, which remains on the stack through roundtrip.
    let context = unsafe { &mut *data.cast::<BindContext>() };
    // SAFETY: Wayland registry interface names are NUL-terminated strings.
    let interface_name = unsafe { CStr::from_ptr(interface_name) }.to_bytes();
    let (interface, interface_c, bind_version, kind) = match interface_name {
        b"wl_compositor" => (
            context.interfaces.compositor,
            c"wl_compositor",
            version.min(4),
            0,
        ),
        b"wl_subcompositor" => (
            context.interfaces.subcompositor,
            c"wl_subcompositor",
            version.min(1),
            1,
        ),
        b"wl_shm" => (context.interfaces.shm, c"wl_shm", version.min(1), 2),
        _ => return,
    };
    if bind_version == 0 {
        return;
    }
    // SAFETY: this is wl_registry.bind. The selected interface, version and
    // NUL-terminated name match the global just reported by the compositor.
    let proxy = unsafe {
        (context.marshal)(
            registry,
            0,
            interface,
            1,
            0,
            name,
            interface_c.as_ptr(),
            bind_version,
            ptr::null_mut::<*mut Proxy>(),
        )
    };
    if proxy.is_null() {
        return;
    }
    match kind {
        0 => {
            context.bindings.compositor = proxy;
            context.bindings.compositor_version = bind_version;
        }
        1 => context.bindings.subcompositor = proxy,
        _ => context.bindings.shm = proxy,
    }
}

unsafe extern "C" fn registry_global_remove(_data: *mut c_void, _registry: *mut Proxy, _name: u32) {}

unsafe extern "C" fn buffer_release(data: *mut c_void, _buffer: *mut Proxy) {
    if !data.is_null() {
        // SAFETY: listener data points to a stable AtomicBool owned by its
        // ShmBuffer and lives until after the proxy is destroyed.
        unsafe { &*data.cast::<AtomicBool>() }.store(true, Ordering::Release);
    }
}

struct ShmBuffer {
    proxy: *mut Proxy,
    mapping: *mut c_void,
    mapping_len: usize,
    width: u32,
    height: u32,
    released: Box<AtomicBool>,
}

impl ShmBuffer {
    fn create(
        marshal: MarshalFlags,
        add_listener: AddListener,
        interfaces: &Interfaces,
        shm: *mut Proxy,
        shm_version: u32,
        width: u32,
        height: u32,
    ) -> Result<Self, String> {
        let stride = width
            .checked_mul(4)
            .ok_or_else(|| "text overlay buffer stride overflow".to_owned())?;
        let size = stride
            .checked_mul(height)
            .ok_or_else(|| "text overlay buffer size overflow".to_owned())?;
        let size_i32 = i32::try_from(size)
            .map_err(|_| "text overlay buffer exceeds Wayland's size limit".to_owned())?;
        let width_i32 = i32::try_from(width).map_err(|_| "text overlay width is too large")?;
        let height_i32 = i32::try_from(height).map_err(|_| "text overlay height is too large")?;
        let stride_i32 = i32::try_from(stride).map_err(|_| "text overlay stride is too large")?;

        let name = CString::new(format!("rusty-blox-text-{}", std::process::id()))
            .map_err(|error| error.to_string())?;
        // SAFETY: memfd_create copies the live name and returns a new owned fd.
        let fd = unsafe { libc::memfd_create(name.as_ptr(), libc::MFD_CLOEXEC) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        // SAFETY: `fd` came from memfd_create and ownership is transferred once.
        let file = unsafe { File::from_raw_fd(fd) };
        // SAFETY: ftruncate operates on the owned memfd with the checked size.
        if unsafe { libc::ftruncate(file.as_raw_fd(), size as libc::off_t) } != 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        // SAFETY: this creates a shared read/write mapping over the live memfd.
        let mapping = unsafe {
            libc::mmap(
                ptr::null_mut(),
                size as usize,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                file.as_raw_fd(),
                0,
            )
        };
        if mapping == libc::MAP_FAILED {
            return Err(std::io::Error::last_os_error().to_string());
        }

        // SAFETY: wl_shm.create_pool receives the live fd and checked size.
        let pool = unsafe {
            marshal(
                shm,
                0,
                interfaces.shm_pool,
                shm_version,
                0,
                ptr::null_mut::<*mut Proxy>(),
                file.as_raw_fd(),
                size_i32,
            )
        };
        if pool.is_null() {
            // SAFETY: the mapping was created above and has not been shared.
            unsafe { libc::munmap(mapping, size as usize) };
            return Err("wl_shm_create_pool returned null".to_owned());
        }
        // SAFETY: pool creation copied the fd into Wayland's outgoing message.
        // Flush before `file` closes below so the compositor receives it.
        // Buffer creation itself follows wl_shm_pool.create_buffer's ABI.
        let buffer = unsafe {
            marshal(
                pool,
                0,
                interfaces.buffer,
                1,
                0,
                ptr::null_mut::<*mut Proxy>(),
                0i32,
                width_i32,
                height_i32,
                stride_i32,
                WL_SHM_FORMAT_ARGB8888,
            )
        };
        // wl_shm_pool.destroy is request 1 and the pool no longer needs a
        // client-side proxy once its buffer has been created.
        unsafe {
            marshal(
                pool,
                1,
                ptr::null(),
                1,
                WL_MARSHAL_FLAG_DESTROY,
            );
        }
        if buffer.is_null() {
            // SAFETY: the mapping was created above and no buffer references it.
            unsafe { libc::munmap(mapping, size as usize) };
            return Err("wl_shm_pool_create_buffer returned null".to_owned());
        }

        let released = Box::new(AtomicBool::new(true));
        let listener = BufferListener {
            release: buffer_release,
        };
        // SAFETY: listener layout is one function pointer per wl_buffer event;
        // released remains at a stable address for the ShmBuffer's lifetime.
        let status = unsafe {
            add_listener(
                buffer,
                (&listener as *const BufferListener).cast::<*const c_void>(),
                (&*released as *const AtomicBool).cast_mut().cast(),
            )
        };
        if status != 0 {
            unsafe {
                marshal(
                    buffer,
                    0,
                    ptr::null(),
                    1,
                    WL_MARSHAL_FLAG_DESTROY,
                );
                libc::munmap(mapping, size as usize);
            }
            return Err("could not install the Wayland text buffer listener".to_owned());
        }
        drop(file);
        Ok(Self {
            proxy: buffer,
            mapping,
            mapping_len: size as usize,
            width,
            height,
            released,
        })
    }

    fn is_released(&self) -> bool {
        self.released.load(Ordering::Acquire)
    }
}

impl Drop for ShmBuffer {
    fn drop(&mut self) {
        // SAFETY: the Wayland owner first detaches and roundtrips before these
        // mappings are dropped; each buffer proxy and mapping is destroyed once.
        unsafe { libc::munmap(self.mapping, self.mapping_len) };
    }
}

pub(crate) struct WaylandTextOverlay {
    _library: Library,
    display: *mut Proxy,
    queue: *mut Queue,
    surface: *mut Proxy,
    subsurface: *mut Proxy,
    compositor: *mut Proxy,
    compositor_version: u32,
    subcompositor: *mut Proxy,
    shm: *mut Proxy,
    shm_version: u32,
    interfaces: Interfaces,
    marshal: MarshalFlags,
    add_listener: AddListener,
    set_queue: SetQueue,
    proxy_version: ProxyVersion,
    proxy_destroy: ProxyDestroy,
    buffers: Vec<ShmBuffer>,
    last_frame: Option<FrameKey>,
    hidden: bool,
}

#[derive(Clone, PartialEq)]
struct FrameKey {
    text: String,
    caret: usize,
    info: RawTextBoxInfo,
    scale: u32,
}

impl WaylandTextOverlay {
    pub(crate) fn create(display: *mut c_void, parent: *mut c_void) -> Result<Self, String> {
        let library = unsafe { Library::new("libwayland-client.so.0") }
            .map_err(|error| format!("load libwayland-client for text overlay: {error}"))?;
        let get_registry = unsafe {
            *library
                .get::<unsafe extern "C" fn(*mut Proxy) -> *mut Proxy>(b"wl_display_get_registry\0")
                .map_err(|error| format!("resolve wl_display_get_registry: {error}"))?
        };
        let create_queue = unsafe {
            *library
                .get::<unsafe extern "C" fn(*mut Proxy) -> *mut Queue>(b"wl_display_create_queue\0")
                .map_err(|error| format!("resolve wl_display_create_queue: {error}"))?
        };
        let roundtrip_queue = unsafe {
            *library
                .get::<unsafe extern "C" fn(*mut Proxy, *mut Queue) -> c_int>(b"wl_display_roundtrip_queue\0")
                .map_err(|error| format!("resolve wl_display_roundtrip_queue: {error}"))?
        };
        let dispatch_pending = unsafe {
            *library
                .get::<unsafe extern "C" fn(*mut Proxy, *mut Queue) -> c_int>(b"wl_display_dispatch_queue_pending\0")
                .map_err(|error| format!("resolve wl_display_dispatch_queue_pending: {error}"))?
        };
        let flush = unsafe {
            *library
                .get::<unsafe extern "C" fn(*mut Proxy) -> c_int>(b"wl_display_flush\0")
                .map_err(|error| format!("resolve wl_display_flush: {error}"))?
        };
        let marshal = unsafe {
            *library
                .get::<MarshalFlags>(b"wl_proxy_marshal_flags\0")
                .map_err(|error| format!("resolve wl_proxy_marshal_flags: {error}"))?
        };
        let add_listener = unsafe {
            *library
                .get::<AddListener>(b"wl_proxy_add_listener\0")
                .map_err(|error| format!("resolve wl_proxy_add_listener: {error}"))?
        };
        let set_queue = unsafe {
            *library
                .get::<SetQueue>(b"wl_proxy_set_queue\0")
                .map_err(|error| format!("resolve wl_proxy_set_queue: {error}"))?
        };
        let proxy_version = unsafe {
            *library
                .get::<ProxyVersion>(b"wl_proxy_get_version\0")
                .map_err(|error| format!("resolve wl_proxy_get_version: {error}"))?
        };
        let proxy_destroy = unsafe {
            *library
                .get::<ProxyDestroy>(b"wl_proxy_destroy\0")
                .map_err(|error| format!("resolve wl_proxy_destroy: {error}"))?
        };
        let interfaces = Interfaces {
            compositor: interface(&library, b"wl_compositor_interface\0")?,
            subcompositor: interface(&library, b"wl_subcompositor_interface\0")?,
            shm: interface(&library, b"wl_shm_interface\0")?,
            surface: interface(&library, b"wl_surface_interface\0")?,
            subsurface: interface(&library, b"wl_subsurface_interface\0")?,
            region: interface(&library, b"wl_region_interface\0")?,
            shm_pool: interface(&library, b"wl_shm_pool_interface\0")?,
            buffer: interface(&library, b"wl_buffer_interface\0")?,
        };
        let display = display.cast::<Proxy>();
        let parent = parent.cast::<Proxy>();
        if display.is_null() || parent.is_null() {
            return Err("Winit returned a null Wayland display or surface".to_owned());
        }
        // SAFETY: Winit owns this live wl_display and this wrapper is dropped
        // before Winit's window. The library is kept alive by this object.
        let queue = unsafe { create_queue(display) };
        if queue.is_null() {
            return Err("wl_display_create_queue returned null".to_owned());
        }
        // SAFETY: the display is live and registry is a core display request.
        let registry = unsafe { get_registry(display) };
        if registry.is_null() {
            return Err("wl_display_get_registry returned null".to_owned());
        }
        unsafe { set_queue(registry, queue) };
        let mut context = BindContext {
            marshal,
            interfaces,
            bindings: Bindings::default(),
        };
        let listener = RegistryListener {
            global: registry_global,
            global_remove: registry_global_remove,
        };
        // SAFETY: the registry listener is live for the synchronous roundtrip;
        // its data points at the live stack context above.
        let status = unsafe {
            add_listener(
                registry,
                (&listener as *const RegistryListener).cast::<*const c_void>(),
                (&mut context as *mut BindContext).cast(),
            )
        };
        if status != 0 {
            return Err("could not subscribe to the Wayland registry".to_owned());
        }
        // SAFETY: this private queue receives only our registry events; winit's
        // own events remain queued on its separately assigned queue.
        if unsafe { roundtrip_queue(display, queue) } < 0 {
            return Err("Wayland registry roundtrip failed".to_owned());
        }
        let bindings = context.bindings;
        if bindings.compositor.is_null() || bindings.subcompositor.is_null() || bindings.shm.is_null() {
            return Err("Wayland compositor, subcompositor, or shared memory is unavailable".to_owned());
        }
        // Bind-created proxies inherit the registry's private queue.
        let compositor = bindings.compositor;
        let compositor_version = bindings.compositor_version;
        let subcompositor = bindings.subcompositor;
        let shm = bindings.shm;
        let shm_version = 1;
        let parent_version = unsafe { proxy_version(parent) };

        // Create the transparent child and put it above the engine's toplevel.
        // `get_subsurface` has constructor opcode 1; the resulting child surface
        // keeps using the same Winit-owned Wayland connection.
        let surface = unsafe {
            marshal(
                compositor,
                0,
                context.interfaces.surface,
                compositor_version,
                0,
                ptr::null_mut::<*mut Proxy>(),
            )
        };
        if surface.is_null() {
            return Err("wl_compositor_create_surface returned null".to_owned());
        }
        let subsurface = unsafe {
            marshal(
                subcompositor,
                1,
                context.interfaces.subsurface,
                1,
                0,
                ptr::null_mut::<*mut Proxy>(),
                surface,
                parent,
            )
        };
        if subsurface.is_null() {
            return Err("wl_subcompositor_get_subsurface returned null".to_owned());
        }
        // SAFETY: these are wl_subsurface requests on the newly created child.
        unsafe {
            marshal(subsurface, 5, ptr::null(), 1, 0); // set_desync
            marshal(subsurface, 2, ptr::null(), 1, 0, parent); // place_above
        }
        // Give the overlay an empty input region so clicks reach the Winit
        // toplevel below it, including clicks on the text field itself.
        let region = unsafe {
            marshal(
                compositor,
                1,
                context.interfaces.region,
                compositor_version,
                0,
                ptr::null_mut::<*mut Proxy>(),
            )
        };
        if !region.is_null() {
            unsafe {
                marshal(surface, 5, ptr::null(), parent_version, 0, region);
                marshal(region, 0, ptr::null(), 1, WL_MARSHAL_FLAG_DESTROY);
                marshal(surface, 6, ptr::null(), parent_version, 0);
            }
        }
        if unsafe { flush(display) } < 0 {
            return Err("could not flush Wayland text overlay setup".to_owned());
        }

        Ok(Self {
            _library: library,
            display,
            queue,
            surface,
            subsurface,
            compositor,
            compositor_version,
            subcompositor,
            shm,
            shm_version,
            interfaces: context.interfaces,
            marshal,
            add_listener,
            set_queue,
            proxy_version,
            proxy_destroy,
            buffers: Vec::new(),
            last_frame: None,
            hidden: true,
        })
    }

    pub(crate) fn update(
        &mut self,
        text: &str,
        caret_chars: usize,
        info: RawTextBoxInfo,
        scale_factor: f64,
    ) -> Result<(), String> {
        self.dispatch_pending()?;
        let scale = scale_factor.round().clamp(1.0, 4.0) as u32;
        let key = FrameKey {
            text: text.to_owned(),
            caret: caret_chars.min(text.chars().count()),
            info,
            scale,
        };
        if self.last_frame.as_ref() == Some(&key) && !self.hidden {
            return Ok(());
        }
        let width = info.width.ceil().clamp(1.0, 4096.0) as u32;
        let height = info.height.ceil().clamp(1.0, 512.0) as u32;
        if info.width <= 0.0 || info.height <= 0.0 {
            return Ok(());
        }
        let Some(slot) = self.buffer_for(width, height)? else {
            // The compositor still owns all of our buffers. Keep the frame
            // dirty and retry after a later pump dispatches their release.
            return Ok(());
        };
        self.draw_buffer(slot, text, key.caret, info)?;

        // The text-box geometry is in physical surface pixels; subsurface
        // placement is in logical coordinates and the buffer scale restores
        // the physical-pixel raster dimensions.
        let x = (info.x / scale as f32).round() as c_int;
        let y = (info.y / scale as f32).round() as c_int;
        unsafe {
            (self.marshal)(self.subsurface, 1, ptr::null(), 1, 0, x, y);
            if self.compositor_version >= 3 {
                (self.marshal)(self.surface, 8, ptr::null(), self.compositor_version, 0, scale as c_int);
            }
            (self.marshal)(
                self.surface,
                1,
                ptr::null(),
                self.compositor_version,
                0,
                self.buffers[slot].proxy,
                0i32,
                0i32,
            );
            (self.marshal)(
                self.surface,
                2,
                ptr::null(),
                self.compositor_version,
                0,
                0i32,
                0i32,
                width as c_int,
                height as c_int,
            );
            (self.marshal)(self.surface, 6, ptr::null(), self.compositor_version, 0);
        }
        self.buffers[slot].released.store(false, Ordering::Release);
        self.hidden = false;
        self.last_frame = Some(key);
        self.flush()
    }

    pub(crate) fn hide(&mut self) -> Result<(), String> {
        self.dispatch_pending()?;
        if self.hidden {
            return Ok(());
        }
        unsafe {
            (self.marshal)(
                self.surface,
                1,
                ptr::null(),
                self.compositor_version,
                0,
                ptr::null_mut::<Proxy>(),
                0i32,
                0i32,
            );
            (self.marshal)(self.surface, 6, ptr::null(), self.compositor_version, 0);
        }
        self.hidden = true;
        self.last_frame = None;
        self.flush()
    }

    fn buffer_for(&mut self, width: u32, height: u32) -> Result<Option<usize>, String> {
        if let Some((index, _)) = self
            .buffers
            .iter()
            .enumerate()
            .find(|(_, buffer)| buffer.width == width && buffer.height == height && buffer.is_released())
        {
            return Ok(Some(index));
        }
        if self.buffers.len() >= MAX_BUFFERS {
            return Ok(None);
        }
        let buffer = ShmBuffer::create(
            self.marshal,
            self.add_listener,
            &self.interfaces,
            self.shm,
            self.shm_version,
            width,
            height,
        )?;
        self.buffers.push(buffer);
        Ok(Some(self.buffers.len() - 1))
    }

    fn draw_buffer(
        &mut self,
        index: usize,
        text: &str,
        caret_chars: usize,
        info: RawTextBoxInfo,
    ) -> Result<(), String> {
        let buffer = &mut self.buffers[index];
        let mut image = ImageSurface::create(Format::ARgb32, buffer.width as i32, buffer.height as i32)
            .map_err(|error| format!("create text overlay image: {error}"))?;
        let context = Context::new(&image).map_err(|error| format!("create text overlay painter: {error}"))?;
        context.set_operator(Operator::Clear);
        context.paint().map_err(|error| format!("clear text overlay: {error}"))?;
        context.set_operator(Operator::Over);
        let color = if info.text_color == 0 { 0x00ff_ffff } else { info.text_color as u32 };
        context.set_source_rgb(
            ((color >> 16) & 0xff) as f64 / 255.0,
            ((color >> 8) & 0xff) as f64 / 255.0,
            (color & 0xff) as f64 / 255.0,
        );
        context.select_font_face("Sans", FontSlant::Normal, FontWeight::Normal);
        let font_size = info.font_size.max(10.0).min(buffer.height as f32) as f64;
        context.set_font_size(font_size);

        let display_text = if matches!(info.text_input_type, 5 | 9 | 10) {
            "•".repeat(text.chars().count())
        } else {
            text.to_owned()
        };
        let extents = context
            .text_extents(&display_text)
            .map_err(|error| format!("measure text overlay: {error}"))?;
        let available = buffer.width as f64;
        let alignment = match info.x_alignment {
            1 => (available - extents.x_advance()).max(0.0),
            2 => ((available - extents.x_advance()) / 2.0).max(0.0),
            _ => 2.0,
        };
        let baseline = ((buffer.height as f64 - font_size) / 2.0 + font_size * 0.82)
            .clamp(font_size, buffer.height as f64);
        context.move_to(alignment, baseline);
        context
            .show_text(&display_text)
            .map_err(|error| format!("draw text overlay: {error}"))?;

        let caret_byte = display_text
            .char_indices()
            .nth(caret_chars)
            .map_or(display_text.len(), |(offset, _)| offset);
        let caret_x = alignment
            + context
                .text_extents(&display_text[..caret_byte])
                .map_err(|error| format!("measure text caret: {error}"))?
                .x_advance();
        context.rectangle(caret_x.floor(), 2.0, 1.0, (buffer.height as f64 - 4.0).max(1.0));
        context.fill().map_err(|error| format!("draw text caret: {error}"))?;
        drop(context);
        image.flush();
        let data = image
            .data()
            .map_err(|error| format!("access text overlay pixels: {error}"))?;
        // SAFETY: the mapping is writable for width*height*4 bytes and Cairo's
        // ARGB32 surface has the same row-major native-endian pixel format.
        unsafe {
            ptr::copy_nonoverlapping(data.as_ptr(), buffer.mapping.cast::<u8>(), buffer.mapping_len);
        }
        Ok(())
    }

    fn dispatch_pending(&mut self) -> Result<(), String> {
        let dispatch = unsafe {
            *self
                ._library
                .get::<unsafe extern "C" fn(*mut Proxy, *mut Queue) -> c_int>(b"wl_display_dispatch_queue_pending\0")
                .map_err(|error| format!("resolve Wayland queue dispatch: {error}"))?
        };
        // SAFETY: this is our private event queue on the live Winit display.
        if unsafe { dispatch(self.display, self.queue) } < 0 {
            return Err("dispatch Wayland text overlay events failed".to_owned());
        }
        Ok(())
    }

    fn flush(&self) -> Result<(), String> {
        let flush = unsafe {
            *self
                ._library
                .get::<unsafe extern "C" fn(*mut Proxy) -> c_int>(b"wl_display_flush\0")
                .map_err(|error| format!("resolve Wayland flush: {error}"))?
        };
        // SAFETY: the display remains owned by Winit and is live here.
        if unsafe { flush(self.display) } < 0 {
            return Err("flush Wayland text overlay failed".to_owned());
        }
        Ok(())
    }
}

impl Drop for WaylandTextOverlay {
    fn drop(&mut self) {
        // Release attached buffers first and wait for the compositor to stop
        // reading their shared-memory mappings before unmapping them.
        unsafe {
            (self.marshal)(
                self.surface,
                1,
                ptr::null(),
                self.compositor_version,
                0,
                ptr::null_mut::<Proxy>(),
                0i32,
                0i32,
            );
            (self.marshal)(self.surface, 6, ptr::null(), self.compositor_version, 0);
        }
        let roundtrip = unsafe {
            self._library
                .get::<unsafe extern "C" fn(*mut Proxy, *mut Queue) -> c_int>(b"wl_display_roundtrip_queue\0")
        };
        if let Ok(roundtrip) = roundtrip {
            // SAFETY: the parent display is still alive; surface detach is sent
            // before this roundtrip and the private queue receives releases.
            unsafe { roundtrip(self.display, self.queue) };
        }
        unsafe {
            for buffer in &self.buffers {
                (self.marshal)(
                    buffer.proxy,
                    0,
                    ptr::null(),
                    1,
                    WL_MARSHAL_FLAG_DESTROY,
                );
            }
            (self.marshal)(self.subsurface, 0, ptr::null(), 1, WL_MARSHAL_FLAG_DESTROY);
            (self.marshal)(
                self.surface,
                0,
                ptr::null(),
                self.compositor_version,
                WL_MARSHAL_FLAG_DESTROY,
            );
            (self.marshal)(self.subcompositor, 0, ptr::null(), 1, WL_MARSHAL_FLAG_DESTROY);
            (self.proxy_destroy)(self.compositor);
            (self.proxy_destroy)(self.shm);
        }
        self.buffers.clear();
        let destroy_queue = unsafe {
            self._library
                .get::<unsafe extern "C" fn(*mut Queue)>(b"wl_event_queue_destroy\0")
        };
        if let Ok(destroy_queue) = destroy_queue {
            // SAFETY: no proxy from this overlay will use the private queue now.
            unsafe { destroy_queue(self.queue) };
        }
    }
}

fn interface(library: &Library, name: &'static [u8]) -> Result<*const Interface, String> {
    // SAFETY: these exported data symbols are wl_interface records in the
    // loaded libwayland-client and the library outlives all proxies using them.
    let symbol = unsafe { library.get::<*const Interface>(name) }
        .map_err(|error| format!("resolve Wayland interface {}: {error}", String::from_utf8_lossy(name)))?;
    Ok(*symbol)
}
