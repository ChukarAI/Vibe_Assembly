use std::ffi::c_void;
use std::mem::{size_of, zeroed};
use std::ptr;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use crate::overlay::model::{
    CoordTransform, KillSwitch, MonitorBounds, OverlayCommand, OverlayState, ScreenPoint,
    MOVE_DURATION, PRECLICK_DURATION,
};
use crate::overlay::raster::{render_scene, BgraBuffer};

const WS_POPUP: u32 = 0x8000_0000;
const WS_EX_LAYERED: u32 = 0x0008_0000;
const WS_EX_TRANSPARENT: u32 = 0x0000_0020;
const WS_EX_TOPMOST: u32 = 0x0000_0008;
const WS_EX_NOACTIVATE: u32 = 0x0800_0000;
const WS_EX_TOOLWINDOW: u32 = 0x0000_0080;
const SW_HIDE: i32 = 0;
const SW_SHOWNOACTIVATE: i32 = 4;
const SWP_NOACTIVATE: u32 = 0x0010;
const ULW_ALPHA: u32 = 2;
const AC_SRC_OVER: u8 = 0;
const AC_SRC_ALPHA: u8 = 1;
const BI_RGB: u32 = 0;
const DIB_RGB_COLORS: u32 = 0;
const VK_ESCAPE: i32 = 0x1B;
const SPI_GETCLIENTAREAANIMATION: u32 = 0x1042;
const MONITOR_DEFAULTTOPRIMARY: u32 = 2;
const MONITORINFOF_PRIMARY: u32 = 1;
const IDC_ARROW: usize = 32512;
const ERROR_ACCESS_DENIED: u32 = 5;
const HWND_TOPMOST: isize = -1;
const FRAME_INTERVAL: Duration = Duration::from_nanos(1_000_000_000 / 60);
const DEMO_DURATION: Duration = Duration::from_millis(2500);

type Hwnd = isize;
type Hdc = isize;
type Hbitmap = isize;
type Hinstance = isize;
type Hmonitor = isize;
type Lparam = isize;
type Lresult = isize;
type Wndproc = Option<unsafe extern "system" fn(Hwnd, u32, usize, Lparam) -> Lresult>;

#[repr(C)]
#[derive(Clone, Copy)]
struct Point {
    x: i32,
    y: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Rect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Size {
    cx: i32,
    cy: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct BitmapInfoHeader {
    size: u32,
    width: i32,
    height: i32,
    planes: u16,
    bit_count: u16,
    compression: u32,
    size_image: u32,
    x_pels_per_meter: i32,
    y_pels_per_meter: i32,
    clr_used: u32,
    clr_important: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct BitmapInfo {
    header: BitmapInfoHeader,
    colors: [u32; 3],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct BlendFunction {
    blend_op: u8,
    blend_flags: u8,
    source_constant_alpha: u8,
    alpha_format: u8,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct WindowClassW {
    style: u32,
    wnd_proc: Wndproc,
    cls_extra: i32,
    wnd_extra: i32,
    instance: Hinstance,
    icon: isize,
    cursor: isize,
    background: isize,
    menu_name: *const u16,
    class_name: *const u16,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct MonitorInfo {
    size: u32,
    monitor: Rect,
    work: Rect,
    flags: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Message {
    window: Hwnd,
    message: u32,
    wparam: usize,
    lparam: Lparam,
    time: u32,
    point: Point,
    l_private: u32,
}

#[link(name = "user32")]
#[allow(non_snake_case)]
extern "system" {
    fn SetProcessDpiAwarenessContext(context: *const c_void) -> i32;
    fn GetDpiForWindow(window: Hwnd) -> u32;
    fn MonitorFromPoint(point: Point, flags: u32) -> Hmonitor;
    fn GetMonitorInfoW(monitor: Hmonitor, info: *mut MonitorInfo) -> i32;
    fn GetAsyncKeyState(key: i32) -> i16;
    fn SystemParametersInfoW(action: u32, param: u32, out: *mut c_void, win_ini: u32) -> i32;
    fn RegisterClassW(class: *const WindowClassW) -> u16;
    fn LoadCursorW(instance: Hinstance, cursor: *const u16) -> isize;
    fn CreateWindowExW(
        ex_style: u32,
        class_name: *const u16,
        window_name: *const u16,
        style: u32,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        parent: Hwnd,
        menu: isize,
        instance: Hinstance,
        param: *mut c_void,
    ) -> Hwnd;
    fn DestroyWindow(window: Hwnd) -> i32;
    fn SetWindowPos(
        window: Hwnd,
        insert_after: Hwnd,
        x: i32,
        y: i32,
        cx: i32,
        cy: i32,
        flags: u32,
    ) -> i32;
    fn ShowWindow(window: Hwnd, command: i32) -> i32;
    fn UpdateLayeredWindow(
        window: Hwnd,
        screen_dc: Hdc,
        position: *const Point,
        size: *const Size,
        source_dc: Hdc,
        source_position: *const Point,
        color_key: u32,
        blend: *const BlendFunction,
        flags: u32,
    ) -> i32;
    fn GetDC(window: Hwnd) -> Hdc;
    fn ReleaseDC(window: Hwnd, dc: Hdc) -> i32;
    fn CreateCompatibleDC(dc: Hdc) -> Hdc;
    fn DeleteDC(dc: Hdc) -> i32;
    fn DefWindowProcW(window: Hwnd, message: u32, wparam: usize, lparam: Lparam) -> Lresult;
    fn PeekMessageW(message: *mut Message, window: Hwnd, min: u32, max: u32, remove: u32) -> i32;
    fn TranslateMessage(message: *const Message) -> i32;
    fn DispatchMessageW(message: *const Message) -> Lresult;
}

#[link(name = "gdi32")]
#[allow(non_snake_case)]
extern "system" {
    fn CreateDIBSection(
        dc: Hdc,
        info: *const BitmapInfo,
        usage: u32,
        bits: *mut *mut c_void,
        section: isize,
        offset: u32,
    ) -> Hbitmap;
    fn SelectObject(dc: Hdc, object: isize) -> isize;
    fn DeleteObject(object: isize) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    fn GetModuleHandleW(name: *const u16) -> Hinstance;
    fn GetLastError() -> u32;
}

static WINDOW_CLASS_ID: AtomicU32 = AtomicU32::new(0);

struct OwnedHwnd(Hwnd);

impl Drop for OwnedHwnd {
    fn drop(&mut self) {
        if self.0 != 0 {
            // SAFETY: the handle belongs to this wrapper and is destroyed once.
            unsafe {
                DestroyWindow(self.0);
            }
        }
    }
}

struct OwnedDc(Hdc);

impl Drop for OwnedDc {
    fn drop(&mut self) {
        if self.0 != 0 {
            // SAFETY: CreateCompatibleDC returned this owned memory DC.
            unsafe {
                DeleteDC(self.0);
            }
        }
    }
}

struct WindowDc {
    window: Hwnd,
    dc: Hdc,
}

impl Drop for WindowDc {
    fn drop(&mut self) {
        if self.dc != 0 {
            // SAFETY: GetDC returned this DC for the stored window.
            unsafe {
                ReleaseDC(self.window, self.dc);
            }
        }
    }
}

struct OwnedBitmap(Hbitmap);

impl Drop for OwnedBitmap {
    fn drop(&mut self) {
        if self.0 != 0 {
            // SAFETY: the DIB is no longer selected into any DC.
            unsafe {
                DeleteObject(self.0);
            }
        }
    }
}

struct SurfaceDc {
    _memory: OwnedDc,
    _window: WindowDc,
}

impl SurfaceDc {
    fn new(window: Hwnd) -> Result<Self, String> {
        // SAFETY: GetDC returns a window DC paired with ReleaseDC in WindowDc::drop.
        let window_dc = unsafe { GetDC(window) };
        if window_dc == 0 {
            return Err(last_error("GetDC"));
        }
        let window_dc = WindowDc {
            window,
            dc: window_dc,
        };
        // SAFETY: the window DC is valid and the returned memory DC is owned.
        let memory = unsafe { CreateCompatibleDC(window_dc.dc) };
        if memory == 0 {
            return Err(last_error("CreateCompatibleDC"));
        }
        Ok(Self {
            _memory: OwnedDc(memory),
            _window: window_dc,
        })
    }

    fn memory(&self) -> Hdc {
        self._memory.0
    }
}

struct Dib {
    dc: Hdc,
    _bitmap: OwnedBitmap,
    previous: Hbitmap,
    bits: *mut u8,
    width: usize,
    height: usize,
}

impl Dib {
    fn new(dc: Hdc, width: usize, height: usize) -> Result<Self, String> {
        if width == 0 || height == 0 || width > i32::MAX as usize || height > i32::MAX as usize {
            return Err("invalid layered-window dimensions".to_string());
        }
        let info = BitmapInfo {
            header: BitmapInfoHeader {
                size: size_of::<BitmapInfoHeader>() as u32,
                width: width as i32,
                height: -(height as i32),
                planes: 1,
                bit_count: 32,
                compression: BI_RGB,
                size_image: 0,
                x_pels_per_meter: 0,
                y_pels_per_meter: 0,
                clr_used: 0,
                clr_important: 0,
            },
            colors: [0; 3],
        };
        let mut bits = ptr::null_mut();
        // SAFETY: info describes a top-down 32-bit DIB and bits receives its allocation.
        let bitmap = unsafe { CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, 0, 0) };
        if bitmap == 0 {
            return Err(last_error("CreateDIBSection"));
        }
        let bitmap = OwnedBitmap(bitmap);
        if bits.is_null() {
            return Err("CreateDIBSection returned no pixel storage".to_string());
        }
        // SAFETY: selecting the new DIB into the owned memory DC returns the prior object.
        let previous = unsafe { SelectObject(dc, bitmap.0) };
        if previous == 0 || previous == -1 {
            return Err(last_error("SelectObject"));
        }
        Ok(Self {
            dc,
            _bitmap: bitmap,
            previous,
            bits: bits.cast(),
            width,
            height,
        })
    }

    fn blit(&self, buffer: &BgraBuffer) -> Result<(), String> {
        if buffer.width != self.width || buffer.height != self.height {
            return Err("raster buffer does not match the layered DIB".to_string());
        }
        let len = self
            .width
            .checked_mul(self.height)
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or_else(|| "layered DIB byte length overflow".to_string())?;
        // SAFETY: the DIB owns exactly len bytes and remains selected for this call.
        let output = unsafe { std::slice::from_raw_parts_mut(self.bits, len) };
        buffer.copy_bgra_to(output)
    }
}

impl Drop for Dib {
    fn drop(&mut self) {
        // SAFETY: restore the prior object before OwnedBitmap deletes the selected DIB.
        unsafe {
            SelectObject(self.dc, self.previous);
        }
    }
}

struct LayeredWindow {
    dib: Dib,
    surface: SurfaceDc,
    window: OwnedHwnd,
    width: usize,
    height: usize,
}

impl LayeredWindow {
    fn new(origin: ScreenPoint, width: usize, height: usize) -> Result<Self, String> {
        ensure_dpi_awareness()?;
        let instance = current_instance()?;
        let class_name = overlay_class_name();
        register_class(instance, &class_name)?;
        let class_id = WINDOW_CLASS_ID.load(Ordering::SeqCst);
        if class_id == 0 {
            return Err("overlay window class is unavailable".to_string());
        }
        let x = finite_i32(origin.x, "overlay x")?;
        let y = finite_i32(origin.y, "overlay y")?;
        let cx = finite_i32(width as f64, "overlay width")?;
        let cy = finite_i32(height as f64, "overlay height")?;
        // SAFETY: all pointers are valid UTF-16 constants for the registered class.
        let window = unsafe {
            CreateWindowExW(
                WS_EX_LAYERED
                    | WS_EX_TRANSPARENT
                    | WS_EX_TOPMOST
                    | WS_EX_NOACTIVATE
                    | WS_EX_TOOLWINDOW,
                class_name.as_ptr(),
                ptr::null(),
                WS_POPUP,
                x,
                y,
                cx,
                cy,
                0,
                0,
                instance,
                ptr::null_mut(),
            )
        };
        if window == 0 {
            return Err(last_error("CreateWindowExW"));
        }
        let window = OwnedHwnd(window);
        // SAFETY: the wrapper owns this valid top-level window handle.
        if unsafe { SetWindowPos(window.0, HWND_TOPMOST, x, y, cx, cy, SWP_NOACTIVATE) } == 0 {
            return Err(last_error("SetWindowPos"));
        }
        let surface = SurfaceDc::new(window.0)?;
        let dib = Dib::new(surface.memory(), width, height)?;
        let blank = BgraBuffer::new(width, height)?;
        let result = Self {
            dib,
            surface,
            window,
            width,
            height,
        };
        result.present(&blank, origin)?;
        // SAFETY: the wrapper owns the initialized layered window.
        unsafe {
            ShowWindow(result.window.0, SW_SHOWNOACTIVATE);
        }
        Ok(result)
    }

    fn dpi_factor(&self) -> Result<f64, String> {
        // SAFETY: GetDpiForWindow only reads the owned window handle.
        let dpi = unsafe { GetDpiForWindow(self.window.0) };
        if dpi == 0 {
            return Err(last_error("GetDpiForWindow"));
        }
        Ok(f64::from(dpi) / 96.0)
    }

    fn hide(&self) {
        // SAFETY: hiding does not transfer ownership of the valid window.
        unsafe {
            ShowWindow(self.window.0, SW_HIDE);
        }
    }

    fn present(&self, buffer: &BgraBuffer, origin: ScreenPoint) -> Result<(), String> {
        self.dib.blit(buffer)?;
        let position = Point {
            x: finite_i32(origin.x, "overlay x")?,
            y: finite_i32(origin.y, "overlay y")?,
        };
        let size = Size {
            cx: finite_i32(self.width as f64, "overlay width")?,
            cy: finite_i32(self.height as f64, "overlay height")?,
        };
        let source = Point { x: 0, y: 0 };
        let blend = BlendFunction {
            blend_op: AC_SRC_OVER,
            blend_flags: 0,
            source_constant_alpha: 255,
            alpha_format: AC_SRC_ALPHA,
        };
        // SAFETY: the selected DIB, source DC, position, and size all match this window.
        if unsafe {
            UpdateLayeredWindow(
                self.window.0,
                0,
                &position,
                &size,
                self.surface.memory(),
                &source,
                0,
                &blend,
                ULW_ALPHA,
            )
        } == 0
        {
            return Err(last_error("UpdateLayeredWindow"));
        }
        Ok(())
    }
}

pub fn run_overlay_demo() -> Result<(), String> {
    ensure_dpi_awareness()?;
    let bounds = primary_monitor_bounds()?;
    let width = usize::try_from(finite_i32(bounds.width, "overlay width")?)
        .map_err(|_| "negative overlay width".to_string())?;
    let height = usize::try_from(finite_i32(bounds.height, "overlay height")?)
        .map_err(|_| "negative overlay height".to_string())?;
    let window = LayeredWindow::new(bounds.origin, width, height)?;
    let transform = CoordTransform::new(window.dpi_factor()?, bounds)?;
    let reduced_motion = prefers_reduced_motion();
    let kill_switch = KillSwitch::new();
    let start = transform
        .screenshot_to_screen(ScreenPoint::new(
            bounds.width / transform.scale_factor * 0.30,
            bounds.height / transform.scale_factor * 0.46,
        ))
        .map_err(|error| format!("invalid overlay demo start: {error}"))?;
    let target = transform
        .screenshot_to_screen(ScreenPoint::new(
            bounds.width / transform.scale_factor * 0.62,
            bounds.height / transform.scale_factor * 0.52,
        ))
        .map_err(|error| format!("invalid overlay demo target: {error}"))?;
    let mut state = OverlayState::new(start, transform, reduced_motion, &kill_switch)?;
    if state.action_pill().is_some() {
        return Err("overlay demo action pill must start empty".to_string());
    }
    if state.action_pill_anchor().is_some() {
        return Err("overlay demo action pill anchor must start empty".to_string());
    }
    let mut buffer = BgraBuffer::new(width, height)?;
    let started = Instant::now();

    state
        .command(OverlayCommand::MoveTo(target), Duration::ZERO)
        .map_err(|error| format!("overlay demo command failed: {error}"))?;
    state
        .command(
            OverlayCommand::SetActionPill("Click Save".to_string()),
            Duration::ZERO,
        )
        .map_err(|error| format!("overlay demo command failed: {error}"))?;
    let mut preclick_sent = false;
    let mut click_sent = false;
    let mut typing_sent = false;
    let mut return_sent = false;
    let mut final_frame = None;
    let mut next_frame = started;

    loop {
        pump_messages();
        let now = started.elapsed();
        if kill_switch.is_aborted() {
            state.abort();
            window.hide();
            return Ok(());
        }
        if escape_pressed() {
            state
                .command(OverlayCommand::Abort, now)
                .map_err(|error| format!("overlay demo abort failed: {error}"))?;
            window.hide();
            return Ok(());
        }
        if now >= DEMO_DURATION {
            window.hide();
            return Ok(());
        }
        if now >= MOVE_DURATION && !preclick_sent {
            state
                .command(OverlayCommand::PreClickPulse, now)
                .map_err(|error| format!("overlay demo command failed: {error}"))?;
            preclick_sent = true;
        }
        if now >= MOVE_DURATION + PRECLICK_DURATION && !click_sent {
            state
                .command(OverlayCommand::Click, now)
                .map_err(|error| format!("overlay demo command failed: {error}"))?;
            click_sent = true;
        }
        if now >= MOVE_DURATION + PRECLICK_DURATION + Duration::from_millis(45) && !typing_sent {
            state
                .command(OverlayCommand::ParkForTyping, now)
                .map_err(|error| format!("overlay demo command failed: {error}"))?;
            typing_sent = true;
        }
        if now >= MOVE_DURATION + PRECLICK_DURATION + Duration::from_millis(100) && !return_sent {
            state
                .move_to(start, now)
                .map_err(|error| format!("overlay demo movement failed: {error}"))?;
            return_sent = true;
        }
        state.advance(now);
        if final_frame.is_none()
            && now >= MOVE_DURATION * 2 + PRECLICK_DURATION + Duration::from_millis(100)
        {
            final_frame = Some(now + Duration::from_millis(1000));
        }
        if final_frame.is_some_and(|deadline| now >= deadline) {
            state
                .command(OverlayCommand::Hide, now)
                .map_err(|error| format!("overlay demo command failed: {error}"))?;
            window.hide();
        }
        if state.needs_frame(now) {
            render_scene(&mut buffer, &state.scene(now), bounds.origin)?;
            window.present(&buffer, bounds.origin)?;
            state.mark_frame_presented(now);
            next_frame = Instant::now() + FRAME_INTERVAL;
        }
        let sleep_until = if state.needs_frame(now + FRAME_INTERVAL) {
            next_frame
        } else {
            started + DEMO_DURATION
        };
        let remaining = sleep_until.saturating_duration_since(Instant::now());
        if !remaining.is_zero() {
            std::thread::sleep(remaining.min(Duration::from_millis(8)));
        }
    }
}

unsafe extern "system" fn overlay_wnd_proc(
    window: Hwnd,
    message: u32,
    wparam: usize,
    lparam: Lparam,
) -> Lresult {
    // SAFETY: DefWindowProcW only reads the supplied window handle and message values.
    unsafe { DefWindowProcW(window, message, wparam, lparam) }
}

fn current_instance() -> Result<Hinstance, String> {
    // SAFETY: GetModuleHandleW accepts a null module name and returns no owned handle.
    let instance = unsafe { GetModuleHandleW(ptr::null()) };
    if instance == 0 {
        Err(last_error("GetModuleHandleW"))
    } else {
        Ok(instance)
    }
}

fn register_class(instance: Hinstance, name: &[u16]) -> Result<(), String> {
    let existing = WINDOW_CLASS_ID.load(Ordering::SeqCst);
    if existing != 0 {
        return Ok(());
    }
    // SAFETY: LoadCursorW returns a shared system cursor for a documented identifier.
    let cursor = unsafe { LoadCursorW(0, IDC_ARROW as *const u16) };
    if cursor == 0 {
        return Err(last_error("LoadCursorW"));
    }
    let class = WindowClassW {
        style: 0,
        wnd_proc: Some(overlay_wnd_proc),
        cls_extra: 0,
        wnd_extra: 0,
        instance,
        icon: 0,
        cursor,
        background: 0,
        menu_name: ptr::null(),
        class_name: name.as_ptr(),
    };
    // SAFETY: class and its UTF-16 name remain valid for RegisterClassW's copy.
    let atom = unsafe { RegisterClassW(&class) };
    if atom == 0 {
        return Err(last_error("RegisterClassW"));
    }
    WINDOW_CLASS_ID.store(u32::from(atom), Ordering::SeqCst);
    Ok(())
}

fn overlay_class_name() -> Vec<u16> {
    "VibeAssemblyComputerUseOverlay"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect()
}

fn ensure_dpi_awareness() -> Result<(), String> {
    const DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2: *const c_void = -4isize as *const c_void;
    // SAFETY: this documented context constant is process-wide and has no ownership.
    if unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) } == 0 {
        // SAFETY: GetLastError reports why the process-wide context was rejected.
        let error = unsafe { GetLastError() };
        if error != ERROR_ACCESS_DENIED {
            return Err(format!(
                "SetProcessDpiAwarenessContext failed with Win32 error {error}"
            ));
        }
    }
    Ok(())
}

fn primary_monitor_bounds() -> Result<MonitorBounds, String> {
    // SAFETY: MonitorFromPoint only reads the point and returns a borrowed monitor handle.
    let monitor = unsafe { MonitorFromPoint(Point { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY) };
    if monitor == 0 {
        return Err(last_error("MonitorFromPoint"));
    }
    let mut info = MonitorInfo {
        size: size_of::<MonitorInfo>() as u32,
        monitor: Rect {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        },
        work: Rect {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        },
        flags: 0,
    };
    // SAFETY: info is writable, correctly sized, and paired with the borrowed monitor.
    if unsafe { GetMonitorInfoW(monitor, &mut info) } == 0 {
        return Err(last_error("GetMonitorInfoW"));
    }
    if info.flags & MONITORINFOF_PRIMARY == 0 {
        return Err("Windows did not return the primary monitor".to_string());
    }
    let width = i64::from(info.monitor.right) - i64::from(info.monitor.left);
    let height = i64::from(info.monitor.bottom) - i64::from(info.monitor.top);
    if width <= 0 || height <= 0 {
        return Err("primary monitor has no usable bounds".to_string());
    }
    MonitorBounds::new(
        ScreenPoint::new(f64::from(info.monitor.left), f64::from(info.monitor.top)),
        width as f64,
        height as f64,
    )
}

fn prefers_reduced_motion() -> bool {
    let mut enabled: i32 = 1;
    // SAFETY: the output points to one initialized i32 for this system-wide query.
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            (&mut enabled as *mut i32).cast(),
            0,
        )
    };
    ok != 0 && enabled == 0
}

fn escape_key_down(state: i16) -> bool {
    const HIGH_BIT: i16 = -32768;
    state & HIGH_BIT != 0
}

fn escape_pressed() -> bool {
    // SAFETY: GetAsyncKeyState is a non-invasive poll with no owned resources.
    escape_key_down(unsafe { GetAsyncKeyState(VK_ESCAPE) })
}

fn pump_messages() {
    let mut message: Message = unsafe { zeroed() };
    loop {
        // SAFETY: message is writable and the null window polls all thread messages.
        let present = unsafe { PeekMessageW(&mut message, 0, 0, 0, 1) };
        if present == 0 {
            break;
        }
        // SAFETY: both functions only read the message returned by PeekMessageW.
        unsafe {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}

fn finite_i32(value: f64, label: &str) -> Result<i32, String> {
    if !value.is_finite() || value < i32::MIN as f64 || value > i32::MAX as f64 {
        return Err(format!("{label} is outside the Win32 range"));
    }
    Ok(value.round() as i32)
}

fn last_error(operation: &str) -> String {
    // SAFETY: GetLastError returns the calling thread's last-error code without ownership.
    let code = unsafe { GetLastError() };
    format!("{operation} failed with Win32 error {code}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layered_window_styles_are_click_through_and_non_activating() {
        assert_eq!(WS_EX_TRANSPARENT, 0x20);
        assert_eq!(WS_EX_TOPMOST, 0x8);
        assert_eq!(WS_EX_NOACTIVATE, 0x0800_0000);
        assert_eq!(WS_EX_TOOLWINDOW, 0x80);
        assert_eq!(WS_EX_LAYERED, 0x0008_0000);
    }

    #[test]
    fn escape_high_bit_is_the_only_abort_condition() {
        assert!(!escape_key_down(0));
        assert!(!escape_key_down(1));
        assert!(escape_key_down(i16::MIN));
        assert!(escape_key_down(i16::MIN | 1));
    }

    #[test]
    fn finite_i32_rejects_non_finite_and_out_of_range_values() {
        assert_eq!(finite_i32(-1920.0, "x").unwrap(), -1920);
        assert!(finite_i32(f64::NAN, "x").is_err());
        assert!(finite_i32(f64::INFINITY, "x").is_err());
    }
}
