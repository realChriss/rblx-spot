// Constraint (anti-cheat): never touch the Roblox process (OpenProcess, memory, injection, hooks).
// Inputs are only the log files and window queries (FindWindow/GetClientRect).
#![windows_subsystem = "windows"]

mod geo;
mod log;
mod popup;
mod settings;
mod sound;
mod splash;
mod update;

use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};

use settings::{Settings, MARGINS};
use tray_icon::{Icon, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use windows_sys::w;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::HiDpi::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const BOX_ALPHA: u32 = 178;
const TICK_MS: u32 = 250;

struct App {
    overlay: HWND,
    roblox: HWND,
    log: log::Log,
    shown: Option<(String, [i32; 4], u32, Settings)>,
}

impl App {
    unsafe fn tick(&mut self) {
        // requirement: no file IO unless Roblox is focused
        let w = FindWindowW(null(), w!("Roblox"));
        if w.is_null() || w != GetForegroundWindow() || IsIconic(w) != 0 {
            return self.hide();
        }
        if w != self.roblox || self.log.path.is_none() {
            self.roblox = w; // Roblox writes a new log per launch
            self.log.open_newest();
        }
        if self.log.read_new().is_err() {
            self.log.path = None;
        }
        let Some((ip, port)) = &self.log.server else { return self.hide() };
        let (region, km) = geo::get(ip);
        let settings = popup::settings();
        let mut text = format!("Server    {ip}:{port}\nRegion    {region}");
        // IP geolocation is city-level, so closer than that is noise
        match km.filter(|_| settings.distance) {
            Some(km) if km < 50 => text += "\nDistance  < 50 km",
            Some(km) => text += &format!("\nDistance  ~{km} km"),
            None => {}
        }

        let mut r: RECT = zeroed();
        GetClientRect(w, &mut r);
        MapWindowPoints(w, null_mut(), (&mut r as *mut RECT).cast(), 2);
        let key = (text, [r.left, r.top, r.right, r.bottom], GetDpiForWindow(w), settings);
        if self.shown.as_ref() != Some(&key) {
            render(self.overlay, &key.0, key.1, key.2, key.3);
            self.shown = Some(key);
        }
        // topmost gets lost over fullscreen Roblox otherwise
        let flags = SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW;
        SetWindowPos(self.overlay, HWND_TOPMOST, 0, 0, 0, 0, flags);
    }

    unsafe fn hide(&mut self) {
        if self.shown.take().is_some() {
            ShowWindow(self.overlay, SW_HIDE);
        }
    }
}

unsafe fn render(hwnd: HWND, text: &str, [l, t, r, b]: [i32; 4], dpi: u32, s: Settings) {
    let px = |v: i32| v * dpi as i32 / 96;
    let text: Vec<u16> = text.encode_utf16().collect();
    let dc = CreateCompatibleDC(null_mut());
    let font = CreateFontW(
        -((10 * dpi as i32 + 36) / 72), 0, 0, 0, FW_NORMAL as i32, 0, 0, 0, DEFAULT_CHARSET as u32,
        OUT_DEFAULT_PRECIS as u32, CLIP_DEFAULT_PRECIS as u32, ANTIALIASED_QUALITY as u32, 0, w!("Consolas"),
    );
    let old_font = SelectObject(dc, font);
    let mut rc: RECT = zeroed();
    DrawTextW(dc, text.as_ptr(), text.len() as i32, &mut rc, DT_CALCRECT | DT_NOPREFIX);
    let (pad_x, pad_y) = (px(10), px(6));
    let (w, h) = (rc.right + 2 * pad_x, rc.bottom + 2 * pad_y);

    let mut bits = null_mut();
    let bmp = CreateDIBSection(dc, &dib_info(w, h), DIB_RGB_COLORS, &mut bits, null_mut(), 0);
    let old_bmp = SelectObject(dc, bmp);
    SetTextColor(dc, 0xFFFFFF);
    SetBkMode(dc, TRANSPARENT as i32);
    let mut rc = RECT { left: pad_x, top: pad_y, right: w - pad_x, bottom: h - pad_y };
    DrawTextW(dc, text.as_ptr(), text.len() as i32, &mut rc, DT_NOPREFIX);
    GdiFlush();

    if !bits.is_null() {
        // GDI text leaves alpha at 0
        for p in std::slice::from_raw_parts_mut(bits as *mut u32, (w * h) as usize) {
            let c = *p & 0xFF;
            *p = ((BOX_ALPHA + c * (255 - BOX_ALPHA) / 255) << 24) | (c * 0x010101);
        }

        let m = px(MARGINS[s.margin]);
        let x = if s.corner & 1 == 0 { l + m } else { r - w - m };
        let y = if s.corner & 2 == 0 { t + m } else { b - h - m };
        UpdateLayeredWindow(hwnd, null_mut(), &POINT { x, y }, &SIZE { cx: w, cy: h }, dc, &POINT { x: 0, y: 0 }, 0, &blend(255), ULW_ALPHA);
    }

    SelectObject(dc, old_bmp);
    SelectObject(dc, old_font);
    DeleteObject(bmp);
    DeleteObject(font);
    DeleteDC(dc);
}

fn dib_info(w: i32, h: i32) -> BITMAPINFO {
    let mut bmi: BITMAPINFO = unsafe { zeroed() };
    bmi.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
    bmi.bmiHeader.biWidth = w;
    bmi.bmiHeader.biHeight = -h;
    bmi.bmiHeader.biPlanes = 1;
    bmi.bmiHeader.biBitCount = 32;
    bmi.bmiHeader.biCompression = BI_RGB;
    bmi
}

fn blend(alpha: u8) -> BLENDFUNCTION {
    BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: alpha, AlphaFormat: AC_SRC_ALPHA as u8 }
}

fn main() {
    unsafe {
        CreateMutexW(null(), 0, w!("Local\\rblx-spot"));
        if GetLastError() == ERROR_ALREADY_EXISTS {
            let msg = w!("rblx-spot is already running (see the tray icon).");
            MessageBoxW(null_mut(), msg, w!("rblx-spot"), MB_ICONINFORMATION);
            return;
        }
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        sound::play_startup();
        splash::show();

        popup::create();
        TrayIconEvent::set_event_handler(Some(|e| {
            if let TrayIconEvent::Click { rect, button_state: MouseButtonState::Up, .. } = e {
                let x = rect.position.x as i32 + rect.size.width as i32 / 2;
                popup::show(POINT { x, y: rect.position.y as i32 });
            }
        }));
        let tray_px = GetSystemMetrics(SM_CXSMICON) as u32;
        let _tray = TrayIconBuilder::new()
            .with_tooltip("rblx-spot")
            .with_icon(Icon::from_resource(1, Some((tray_px, tray_px))).unwrap()) // id 1 = icon.ico via build.rs
            .build()
            .unwrap();

        let ex = WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TOPMOST;
        let overlay = CreateWindowExW(ex, w!("STATIC"), null(), WS_POPUP, 0, 0, 0, 0, null_mut(), null_mut(), null_mut(), null());

        SetTimer(null_mut(), 0, TICK_MS, None);
        let mut app = App { overlay, roblox: null_mut(), log: log::Log::default(), shown: None };
        let mut msg = zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            if msg.message == WM_TIMER && msg.hwnd.is_null() {
                app.tick();
            } else {
                DispatchMessageW(&msg);
            }
        }
    }
}

