// SPDX-License-Identifier: AGPL-3.0-or-later
//! Native Windows overlays: layered windows updated with `UpdateLayeredWindow`.
//!
//! All overlay windows live on one dedicated thread with its own message loop and frame
//! timing, so animations keep playing while the control panel is busy or hidden.

use std::cell::RefCell;
use std::ffi::c_void;
use std::ptr::{null, null_mut};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender, TryRecvError, bounded, unbounded};
use eframe::egui::Pos2;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, SIZE, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION,
    CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, HBITMAP, HDC,
    HGDIOBJ, SelectObject,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use super::{OverlayEvent, Shared, SharedRef, window_size};
use crate::media::{self, Pixels};

const CLASS_NAME: &str = "OpenAnimaOverlay";
const WM_WAKE: u32 = WM_APP + 1;

/// Thread running the overlays, 0 before it starts.
static THREAD_ID: AtomicU32 = AtomicU32::new(0);

pub fn wake() {
    let id = THREAD_ID.load(Ordering::Relaxed);
    if id != 0 {
        unsafe { PostThreadMessageW(id, WM_WAKE, 0, 0) };
    }
}

pub struct Native {
    tx: Sender<Vec<(SharedRef, Pos2)>>,
    thread_id: u32,
}

impl Native {
    pub fn new() -> Self {
        let (tx, rx) = unbounded();
        let (id_tx, id_rx) = bounded(1);
        std::thread::Builder::new()
            .name("overlays".into())
            .spawn(move || {
                // Make sure the thread has a message queue before anyone posts to it.
                let mut msg: MSG = unsafe { std::mem::zeroed() };
                unsafe { PeekMessageW(&mut msg, null_mut(), 0, 0, PM_NOREMOVE) };
                let _ = id_tx.send(unsafe { GetCurrentThreadId() });
                run(rx);
            })
            .expect("spawning overlay thread");
        let thread_id = id_rx.recv().unwrap_or(0);
        THREAD_ID.store(thread_id, Ordering::Relaxed);
        Self { tx, thread_id }
    }

    /// Replaces the set of displayed overlays.
    pub fn sync(&self, overlays: Vec<(SharedRef, Pos2)>) {
        if self.tx.send(overlays).is_ok() {
            unsafe { PostThreadMessageW(self.thread_id, WM_WAKE, 0, 0) };
        }
    }
}

impl Drop for Native {
    fn drop(&mut self) {
        unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, 0, 0) };
    }
}

/// Mouse input collected by the window procedure, handled by the loop.
enum Input {
    Down(HWND),
    Move(HWND),
    Up(HWND),
    DoubleClick(HWND),
    Close(HWND),
}

thread_local! {
    static INPUT: RefCell<Vec<Input>> = const { RefCell::new(Vec::new()) };
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let input = match msg {
        WM_LBUTTONDOWN => Some(Input::Down(hwnd)),
        WM_MOUSEMOVE => Some(Input::Move(hwnd)),
        WM_LBUTTONUP => Some(Input::Up(hwnd)),
        WM_LBUTTONDBLCLK => Some(Input::DoubleClick(hwnd)),
        WM_CLOSE => Some(Input::Close(hwnd)),
        // Clicking an overlay must not steal the focus from the application being used.
        WM_MOUSEACTIVATE => return MA_NOACTIVATE as LRESULT,
        _ => None,
    };
    match input {
        Some(input) => {
            INPUT.with(|q| q.borrow_mut().push(input));
            0
        }
        None => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// A 32-bit top-down DIB selected into a memory DC.
struct Surface {
    dc: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
    bits: *mut u8,
    width: i32,
    height: i32,
}

impl Surface {
    fn new(width: i32, height: i32) -> Option<Self> {
        unsafe {
            let dc = CreateCompatibleDC(null_mut());
            if dc.is_null() {
                return None;
            }
            let mut info: BITMAPINFO = std::mem::zeroed();
            info.bmiHeader = BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height, // top-down
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..std::mem::zeroed()
            };
            let mut bits: *mut c_void = null_mut();
            let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
            if bitmap.is_null() || bits.is_null() {
                DeleteDC(dc);
                return None;
            }
            let previous = SelectObject(dc, bitmap);
            Some(Self {
                dc,
                bitmap,
                previous,
                bits: bits.cast(),
                width,
                height,
            })
        }
    }

    fn pixels_mut(&mut self) -> &mut [u8] {
        let len = (self.width * self.height * 4) as usize;
        unsafe { std::slice::from_raw_parts_mut(self.bits, len) }
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.previous);
            DeleteObject(self.bitmap);
            DeleteDC(self.dc);
        }
    }
}

struct Window {
    hwnd: HWND,
    shared: SharedRef,
    pos: (i32, i32),
    surface: Option<Surface>,
    /// What is currently on screen: frame serial, size and opacity.
    drawn: Option<(u64, i32, i32, u8)>,
    shown: bool,
    click_through: bool,
    topmost: bool,
    next_due: Instant,
    /// Cursor offset from the window origin while dragging.
    drag: Option<(i32, i32)>,
    /// Frames drawn and moves since `stats_since`, for the debug log.
    stats: (u32, u32),
    stats_since: Instant,
}

impl Window {
    fn create(shared: SharedRef, spawn: Pos2) -> Option<Self> {
        let (click_through, topmost) = {
            let s = shared.lock().unwrap();
            (s.window.click_through, s.window.always_on_top)
        };
        let pos = (spawn.x.round() as i32, spawn.y.round() as i32);
        let class = wide(CLASS_NAME);
        let title = wide("Open Anima");
        let hwnd = unsafe {
            CreateWindowExW(
                ex_style(click_through, topmost),
                class.as_ptr(),
                title.as_ptr(),
                WS_POPUP,
                pos.0,
                pos.1,
                1,
                1,
                null_mut(),
                null_mut(),
                GetModuleHandleW(null()),
                null(),
            )
        };
        if hwnd.is_null() {
            log::error!("CreateWindowExW failed");
            return None;
        }
        Some(Self {
            hwnd,
            shared,
            pos,
            surface: None,
            drawn: None,
            shown: false,
            click_through,
            topmost,
            next_due: Instant::now(),
            drag: None,
            stats: (0, 0),
            stats_since: Instant::now(),
        })
    }

    fn render(&mut self, now: Instant) {
        let shared = self.shared.clone();
        let mut s = shared.lock().unwrap();
        let mut moved = false;
        if let Some(p) = s.move_to.take() {
            self.pos = (p.x.round() as i32, p.y.round() as i32);
            moved = true;
        }
        self.apply_styles(&s);

        let frame = s.anim.lock().unwrap().frame(media::clock(), &s.playback);
        self.next_due = now + Duration::from_secs_f64(frame.next);
        let size = window_size([frame.pixels.width, frame.pixels.height], &s.look);
        let (w, h) = (size.x as i32, size.y as i32);
        let alpha = (s.look.opacity.clamp(0.0, 1.0) * 255.0).round() as u8;
        let key = (frame.pixels.serial, w, h, alpha);

        if self.drawn != Some(key) {
            if self.surface.as_ref().is_none_or(|sf| (sf.width, sf.height) != (w, h)) {
                self.surface = Surface::new(w, h);
            }
            let Some(surface) = self.surface.as_mut() else { return };
            fill(surface.pixels_mut(), &frame.pixels, w as u32, h as u32);
            let blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: alpha,
                AlphaFormat: AC_SRC_ALPHA as u8,
            };
            let dst = POINT {
                x: self.pos.0,
                y: self.pos.1,
            };
            let src = POINT { x: 0, y: 0 };
            let sz = SIZE { cx: w, cy: h };
            unsafe {
                UpdateLayeredWindow(
                    self.hwnd,
                    null_mut(),
                    &dst,
                    &sz,
                    surface.dc,
                    &src,
                    0,
                    &blend,
                    ULW_ALPHA,
                );
            }
            self.drawn = Some(key);
            self.stats.0 += 1;
            moved = false;
        }
        if moved {
            self.set_position();
        }
        if !self.shown {
            unsafe { ShowWindow(self.hwnd, SW_SHOWNOACTIVATE) };
            self.shown = true;
        }
        s.reported_pos = Some(Pos2::new(self.pos.0 as f32, self.pos.1 as f32));
    }

    fn apply_styles(&mut self, s: &Shared) {
        if s.window.click_through != self.click_through {
            self.click_through = s.window.click_through;
            unsafe {
                SetWindowLongPtrW(
                    self.hwnd,
                    GWL_EXSTYLE,
                    ex_style(self.click_through, self.topmost) as isize,
                )
            };
        }
        if s.window.always_on_top != self.topmost {
            self.topmost = s.window.always_on_top;
            let after = if self.topmost { HWND_TOPMOST } else { HWND_NOTOPMOST };
            unsafe {
                SetWindowPos(
                    self.hwnd,
                    after,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                )
            };
        }
    }

    /// Moves requested by behaviors are applied right away, not at the next frame.
    fn apply_pending_move(&mut self) {
        let shared = self.shared.clone();
        let mut s = shared.lock().unwrap();
        if let Some(p) = s.move_to.take() {
            self.pos = (p.x.round() as i32, p.y.round() as i32);
            s.reported_pos = Some(p);
            if self.shown {
                self.set_position();
                self.stats.1 += 1;
            }
        }
    }

    fn log_stats(&mut self, now: Instant) {
        let elapsed = now.duration_since(self.stats_since).as_secs_f32();
        if elapsed >= 2.0 {
            log::debug!(
                "overlay {:?}: {:.1} frames/s, {:.1} moves/s",
                self.hwnd,
                self.stats.0 as f32 / elapsed,
                self.stats.1 as f32 / elapsed
            );
            self.stats = (0, 0);
            self.stats_since = now;
        }
    }

    fn set_position(&self) {
        unsafe {
            SetWindowPos(
                self.hwnd,
                null_mut(),
                self.pos.0,
                self.pos.1,
                0,
                0,
                SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            )
        };
    }
}

impl Drop for Window {
    fn drop(&mut self) {
        unsafe { DestroyWindow(self.hwnd) };
    }
}

fn ex_style(click_through: bool, topmost: bool) -> u32 {
    let mut style = WS_EX_LAYERED | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE;
    if click_through {
        style |= WS_EX_TRANSPARENT;
    }
    if topmost {
        style |= WS_EX_TOPMOST;
    }
    style
}

/// Copies (and scales if needed) a premultiplied BGRA frame into the surface.
fn fill(dst: &mut [u8], frame: &Arc<Pixels>, w: u32, h: u32) {
    if (frame.width, frame.height) == (w, h) {
        dst.copy_from_slice(&frame.bgra);
        return;
    }
    // Filtering premultiplied data is exactly what we want; the channel order is irrelevant.
    let src = image::ImageBuffer::<image::Rgba<u8>, &[u8]>::from_raw(
        frame.width,
        frame.height,
        &frame.bgra[..],
    )
    .expect("frame size matches its buffer");
    let filter = if w < frame.width {
        image::imageops::FilterType::Triangle
    } else {
        image::imageops::FilterType::CatmullRom
    };
    let scaled = image::imageops::resize(&src, w, h, filter);
    dst.copy_from_slice(scaled.as_raw());
}

fn cursor() -> (i32, i32) {
    let mut p = POINT { x: 0, y: 0 };
    unsafe { GetCursorPos(&mut p) };
    (p.x, p.y)
}

fn register_class() {
    let class = wide(CLASS_NAME);
    let wc = WNDCLASSEXW {
        cbSize: size_of::<WNDCLASSEXW>() as u32,
        style: CS_DBLCLKS,
        lpfnWndProc: Some(wndproc),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: unsafe { GetModuleHandleW(null()) },
        hIcon: null_mut(),
        hCursor: unsafe { LoadCursorW(null_mut(), IDC_ARROW) },
        hbrBackground: null_mut(),
        lpszMenuName: null(),
        lpszClassName: class.as_ptr(),
        hIconSm: null_mut(),
    };
    unsafe { RegisterClassExW(&wc) };
}

fn run(rx: Receiver<Vec<(SharedRef, Pos2)>>) {
    register_class();
    let mut windows: Vec<Window> = Vec::new();
    let mut msg: MSG = unsafe { std::mem::zeroed() };

    loop {
        while unsafe { PeekMessageW(&mut msg, null_mut(), 0, 0, PM_REMOVE) } != 0 {
            if msg.message == WM_QUIT {
                return;
            }
            unsafe {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }

        // Latest set of overlays.
        let mut latest = None;
        loop {
            match rx.try_recv() {
                Ok(set) => latest = Some(set),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return,
            }
        }
        if let Some(set) = latest {
            reconcile(&mut windows, set);
        }

        handle_input(&mut windows);
        for window in &mut windows {
            window.apply_pending_move();
        }

        let now = Instant::now();
        for window in &mut windows {
            if log::log_enabled!(log::Level::Debug) {
                window.log_stats(now);
            }
            if now >= window.next_due {
                window.render(now);
            }
        }

        let next = windows.iter().map(|w| w.next_due).min();
        let timeout = next
            .map(|t| t.saturating_duration_since(Instant::now()).as_millis() as u32)
            .unwrap_or(1000)
            .clamp(1, 1000);
        unsafe {
            MsgWaitForMultipleObjectsEx(0, null(), timeout, QS_ALLINPUT, MWMO_INPUTAVAILABLE)
        };
    }
}

fn reconcile(windows: &mut Vec<Window>, set: Vec<(SharedRef, Pos2)>) {
    let mut keep = vec![false; windows.len()];
    let mut created = Vec::new();
    for (shared, spawn) in set {
        let key = shared.lock().unwrap().key.clone();
        let existing = windows
            .iter()
            .position(|w| w.shared.lock().unwrap().key == key);
        match existing {
            Some(i) => {
                keep[i] = true;
                if !Arc::ptr_eq(&windows[i].shared, &shared) {
                    windows[i].shared = shared;
                    windows[i].drawn = None;
                    windows[i].next_due = Instant::now();
                }
            }
            None => created.extend(Window::create(shared, spawn)),
        }
    }
    let mut i = 0;
    windows.retain(|_| {
        i += 1;
        keep[i - 1]
    });
    windows.extend(created);
}

fn handle_input(windows: &mut [Window]) {
    let inputs: Vec<Input> = INPUT.with(|q| std::mem::take(&mut *q.borrow_mut()));
    for input in inputs {
        let hwnd = match &input {
            Input::Down(h) | Input::Move(h) | Input::Up(h) | Input::DoubleClick(h) | Input::Close(h) => *h,
        };
        let Some(window) = windows.iter_mut().find(|w| w.hwnd == hwnd) else {
            continue;
        };
        match input {
            Input::Down(_) => {
                if window.shared.lock().unwrap().window.draggable {
                    let (cx, cy) = cursor();
                    window.drag = Some((cx - window.pos.0, cy - window.pos.1));
                    unsafe { SetCapture(hwnd) };
                }
            }
            Input::Move(_) => {
                if let Some((dx, dy)) = window.drag {
                    let (cx, cy) = cursor();
                    window.pos = (cx - dx, cy - dy);
                    window.set_position();
                    window.shared.lock().unwrap().reported_pos =
                        Some(Pos2::new(window.pos.0 as f32, window.pos.1 as f32));
                }
            }
            Input::Up(_) => {
                if window.drag.take().is_some() {
                    unsafe { ReleaseCapture() };
                }
            }
            Input::DoubleClick(_) => {
                let s = window.shared.lock().unwrap();
                s.send(OverlayEvent::Activated(s.key.clone()));
            }
            Input::Close(_) => {
                let s = window.shared.lock().unwrap();
                s.send(OverlayEvent::Closed(s.key.clone()));
            }
        }
    }
}
