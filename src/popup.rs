use std::cell::Cell;
use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};
use std::time::{Duration, Instant};

use windows_sys::core::PCWSTR;
use windows_sys::w;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Dwm::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Registry::*;
use windows_sys::Win32::UI::HiDpi::*;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::dib_info;
use crate::splash::icon_pixels;
use crate::settings::{Settings, MARGINS};
use crate::update::{self, Status};

const WM_MOUSELEAVE: u32 = 0x02A3;
const W: f32 = 256.0;
const H: f32 = 413.0;
const P: f32 = 16.0;
const PREVIEW: R = R { x: P, y: 70.0, w: W - 2.0 * P, h: 126.0 };
const SEGMENTS: R = R { x: P, y: 234.0, w: W - 2.0 * P, h: 32.0 };
const DISTANCE: R = R { x: 6.0, y: 280.0, w: W - 12.0, h: 36.0 };
const UPDATE: R = R { x: 6.0, y: 333.0, w: W - 12.0, h: 36.0 };
const EXIT: R = R { x: 6.0, y: 369.0, w: W - 12.0, h: 36.0 };

thread_local! {
    static POPUP: Cell<HWND> = const { Cell::new(null_mut()) };
    static SETTINGS: Cell<Settings> = Cell::new(Settings::load());
    static HOVER: Cell<Option<Hit>> = const { Cell::new(None) };
    static HIDDEN_AT: Cell<Option<Instant>> = const { Cell::new(None) };
}

pub fn settings() -> Settings {
    SETTINGS.get()
}

#[derive(Clone, Copy, PartialEq)]
enum Hit {
    Corner(usize),
    Margin(usize),
    Distance,
    Update,
    Exit,
}

#[derive(Clone, Copy)]
struct R {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

impl R {
    fn cell(self, col: usize, row: usize, cols: usize, rows: usize) -> R {
        let (w, h) = (self.w / cols as f32, self.h / rows as f32);
        R { x: self.x + col as f32 * w, y: self.y + row as f32 * h, w, h }
    }
    fn inset(self, d: f32) -> R {
        R { x: self.x + d, y: self.y + d, w: self.w - 2.0 * d, h: self.h - 2.0 * d }
    }
    fn contains(self, x: f32, y: f32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.w && y < self.y + self.h
    }
    fn px(self, s: f32) -> RECT {
        let r = |v: f32| (v * s).round() as i32;
        RECT { left: r(self.x), top: r(self.y), right: r(self.x + self.w), bottom: r(self.y + self.h) }
    }
}

fn corner_box(i: usize) -> R {
    let (w, h, inset) = (64.0, 26.0, 10.0);
    let x = if i & 1 == 0 { PREVIEW.x + inset } else { PREVIEW.x + PREVIEW.w - inset - w };
    let y = if i & 2 == 0 { PREVIEW.y + inset } else { PREVIEW.y + PREVIEW.h - inset - h };
    R { x, y, w, h }
}

fn segment(i: usize) -> R {
    SEGMENTS.cell(i, 0, MARGINS.len(), 1)
}

fn hits() -> impl Iterator<Item = (Hit, R)> {
    let corners = (0..4).map(|i| (Hit::Corner(i), PREVIEW.cell(i & 1, i >> 1, 2, 2)));
    let margins = (0..MARGINS.len()).map(|i| (Hit::Margin(i), segment(i)));
    corners.chain(margins).chain([(Hit::Distance, DISTANCE), (Hit::Update, UPDATE), (Hit::Exit, EXIT)])
}

struct Theme {
    bg: u32,
    surface: u32,
    hover: u32,
    stroke: u32,
    text: u32,
    subtle: u32,
    accent: u32,
    on_accent: u32,
}

const DARK: Theme = Theme {
    bg: 0x2C2C2C, surface: 0x383838, hover: 0x454545, stroke: 0x4A4A4A,
    text: 0xFFFFFF, subtle: 0xC5C5C5, accent: 0x4CC2FF, on_accent: 0x000000,
};
const LIGHT: Theme = Theme {
    bg: 0xF9F9F9, surface: 0xEDEDED, hover: 0xE0E0E0, stroke: 0xD1D1D1,
    text: 0x1A1A1A, subtle: 0x5D5D5D, accent: 0x005FB8, on_accent: 0xFFFFFF,
};

unsafe fn theme() -> &'static Theme {
    let (mut v, mut n) = (0u32, 4u32);
    let key = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize");
    let ok = RegGetValueW(HKEY_CURRENT_USER, key, w!("SystemUsesLightTheme"), RRF_RT_REG_DWORD, null_mut(), (&mut v as *mut u32).cast(), &mut n);
    if ok == 0 && v == 1 { &LIGHT } else { &DARK }
}

fn bgr(c: u32) -> COLORREF {
    (c & 0xFF) << 16 | (c & 0xFF00) | (c >> 16 & 0xFF)
}

struct Canvas<'a> {
    px: &'a mut [u32],
    w: i32,
    h: i32,
    s: f32,
}

impl Canvas<'_> {
    fn fill(&mut self, r: R, radius: f32, color: u32) {
        let s = self.s;
        let (x0, y0, x1, y1) = (r.x * s, r.y * s, (r.x + r.w) * s, (r.y + r.h) * s);
        let rad = (radius * s).min((x1 - x0) / 2.0).min((y1 - y0) / 2.0);
        for y in (y0.floor() as i32).max(0)..(y1.ceil() as i32).min(self.h) {
            for x in (x0.floor() as i32).max(0)..(x1.ceil() as i32).min(self.w) {
                let (cx, cy) = (x as f32 + 0.5, y as f32 + 0.5);
                let (dx, dy) = (cx - cx.clamp(x0 + rad, x1 - rad), cy - cy.clamp(y0 + rad, y1 - rad));
                let a = (0.5 + rad - (dx * dx + dy * dy).sqrt()).clamp(0.0, 1.0);
                let p = &mut self.px[(y * self.w + x) as usize];
                *p = [0, 8, 16].iter().fold(0, |out, &sh| {
                    let (d, c) = ((*p >> sh & 0xFF) as f32, (color >> sh & 0xFF) as f32);
                    out | ((d + (c - d) * a).round() as u32) << sh
                });
            }
        }
    }

    fn image(&mut self, x0: f32, y0: f32, size: f32, src: impl FnOnce(i32) -> Vec<u32>) {
        let px = |v: f32| (v * self.s).round() as i32;
        let (x0, y0, n) = (px(x0), px(y0), px(size));
        let src = src(n);
        for y in y0.max(0)..(y0 + n).min(self.h) {
            for x in x0.max(0)..(x0 + n).min(self.w) {
                let s = src[((y - y0) * n + x - x0) as usize];
                let p = &mut self.px[(y * self.w + x) as usize];
                *p = [0, 8, 16].iter().fold(0, |out, &sh| out | ((s >> sh & 0xFF) + (*p >> sh & 0xFF) * (255 - (s >> 24)) / 255) << sh);
            }
        }
    }
}

unsafe fn font(s: f32, size: f32, face: PCWSTR) -> HFONT {
    CreateFontW(
        -(size * s).round() as i32, 0, 0, 0, FW_NORMAL as i32, 0, 0, 0, DEFAULT_CHARSET as u32,
        OUT_DEFAULT_PRECIS as u32, CLIP_DEFAULT_PRECIS as u32, CLEARTYPE_QUALITY as u32, 0, face,
    )
}

unsafe fn text(dc: HDC, s: f32, r: R, str: &str, color: u32, align: u32) {
    if str.is_empty() {
        return;
    }
    let str: Vec<u16> = str.encode_utf16().collect();
    let mut rc = r.px(s);
    SetTextColor(dc, bgr(color));
    DrawTextW(dc, str.as_ptr(), str.len() as i32, &mut rc, align | DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX);
}

unsafe fn paint(hwnd: HWND) {
    let s = GetDpiForWindow(hwnd) as f32 / 96.0;
    let mut rc: RECT = zeroed();
    GetClientRect(hwnd, &mut rc);
    let (w, h) = (rc.right, rc.bottom);
    let mut ps: PAINTSTRUCT = zeroed();
    let wdc = BeginPaint(hwnd, &mut ps);
    let dc = CreateCompatibleDC(wdc);
    let mut bits = null_mut();
    let bmp = CreateDIBSection(dc, &dib_info(w, h), DIB_RGB_COLORS, &mut bits, null_mut(), 0);
    let old_bmp = SelectObject(dc, bmp);
    if !bits.is_null() {
        let (t, cfg) = (theme(), SETTINGS.get());
        shapes(Canvas { px: std::slice::from_raw_parts_mut(bits as *mut u32, (w * h) as usize), w, h, s }, t, cfg);
        labels(dc, s, t, cfg);
        BitBlt(wdc, 0, 0, w, h, dc, 0, 0, SRCCOPY);
    }
    SelectObject(dc, old_bmp);
    DeleteObject(bmp);
    DeleteDC(dc);
    EndPaint(hwnd, &ps);
}

fn shapes(mut c: Canvas, t: &Theme, cfg: Settings) {
    let hover = HOVER.get();
    c.px.fill(t.bg);
    c.fill(PREVIEW, 6.0, t.stroke);
    c.fill(PREVIEW.inset(1.0), 5.0, t.surface);
    for i in 0..4 {
        let color = if i == cfg.corner { t.accent } else if hover == Some(Hit::Corner(i)) { t.subtle } else { t.stroke };
        c.fill(corner_box(i), 4.0, color);
    }
    c.fill(SEGMENTS, 6.0, t.surface);
    for i in 0..MARGINS.len() {
        let seg = segment(i).inset(3.0);
        if i == cfg.margin {
            c.fill(seg, 4.0, t.accent);
        } else if hover == Some(Hit::Margin(i)) {
            c.fill(seg, 4.0, t.hover);
        }
    }
    for (hit, row) in [(Hit::Distance, DISTANCE), (Hit::Update, UPDATE), (Hit::Exit, EXIT)] {
        if hover == Some(hit) {
            c.fill(row, 4.0, t.hover);
        }
    }
    let toggle = R { x: DISTANCE.x + DISTANCE.w - 50.0, y: DISTANCE.y + 8.0, w: 40.0, h: 20.0 };
    if cfg.distance {
        c.fill(toggle, 10.0, t.accent);
        c.fill(R { x: toggle.x + 24.0, y: toggle.y + 4.0, w: 12.0, h: 12.0 }, 6.0, t.on_accent);
    } else {
        c.fill(toggle, 10.0, t.subtle);
        c.fill(toggle.inset(1.0), 9.0, if hover == Some(Hit::Distance) { t.hover } else { t.bg });
        c.fill(R { x: toggle.x + 5.0, y: toggle.y + 5.0, w: 10.0, h: 10.0 }, 5.0, t.subtle);
    }
    c.fill(R { x: P, y: 324.0, w: W - 2.0 * P, h: 1.0 }, 0.0, t.stroke);
    c.image(P, 14.0, 20.0, |n| unsafe { icon_pixels(n) });
}

unsafe fn labels(dc: HDC, s: f32, t: &Theme, cfg: Settings) {
    SetBkMode(dc, TRANSPARENT as i32);
    // weight 600 on "Segoe UI" is a synthesized bold of the regular face
    let fonts = [font(s, 15.0, w!("Segoe UI Semibold")), font(s, 12.0, w!("Segoe UI")), font(s, 14.0, w!("Segoe UI"))];
    let old_font = SelectObject(dc, fonts[0]);
    let line = |y: f32, h: f32| R { x: P, y, w: W - 2.0 * P, h };
    text(dc, s, R { x: P + 28.0, ..line(12.0, 24.0) }, "rblx-spot", t.text, DT_LEFT);
    SelectObject(dc, fonts[1]);
    text(dc, s, line(12.0, 24.0), concat!("v", env!("CARGO_PKG_VERSION")), t.subtle, DT_RIGHT);
    text(dc, s, line(44.0, 20.0), "Position", t.subtle, DT_LEFT);
    text(dc, s, line(208.0, 20.0), "Margin (px)", t.subtle, DT_LEFT);
    SelectObject(dc, fonts[2]);
    for (i, m) in MARGINS.iter().enumerate() {
        let color = if i == cfg.margin { t.on_accent } else { t.text };
        text(dc, s, segment(i), &m.to_string(), color, DT_CENTER);
    }
    text(dc, s, line(DISTANCE.y, DISTANCE.h), "Show distance", t.text, DT_LEFT);
    let (label, status, color) = match update::status() {
        Status::Idle => ("Check for updates", String::new(), t.subtle),
        Status::Checking => ("Check for updates", "Checking…".into(), t.subtle),
        Status::Latest => ("Check for updates", "Up to date".into(), t.subtle),
        Status::Available(tag) => ("Download update", format!("{tag} available"), t.accent),
        Status::Failed => ("Check for updates", "Couldn't check".into(), t.subtle),
    };
    text(dc, s, line(UPDATE.y, UPDATE.h), label, t.text, DT_LEFT);
    text(dc, s, line(EXIT.y, EXIT.h), "Exit", t.text, DT_LEFT);
    SelectObject(dc, fonts[1]);
    text(dc, s, line(UPDATE.y, UPDATE.h), &status, color, DT_RIGHT);
    SelectObject(dc, old_font);
    for f in fonts {
        DeleteObject(f);
    }
}

unsafe fn hit_at(hwnd: HWND, lp: LPARAM) -> Option<Hit> {
    let s = GetDpiForWindow(hwnd) as f32 / 96.0;
    let (x, y) = ((lp & 0xFFFF) as i16 as f32 / s, (lp >> 16 & 0xFFFF) as i16 as f32 / s);
    hits().find(|(_, r)| r.contains(x, y)).map(|(hit, _)| hit)
}

unsafe fn set_hover(hwnd: HWND, hit: Option<Hit>) {
    if HOVER.replace(hit) != hit {
        InvalidateRect(hwnd, null(), 0);
    }
}

unsafe fn hide(hwnd: HWND) {
    ShowWindow(hwnd, SW_HIDE);
    HOVER.set(None);
    HIDDEN_AT.set(Some(Instant::now()));
}

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => paint(hwnd),
        WM_ERASEBKGND => return 1,
        WM_ACTIVATE if wp & 0xFFFF == WA_INACTIVE as usize => hide(hwnd),
        WM_KEYDOWN if wp == VK_ESCAPE as usize => hide(hwnd),
        WM_MOUSEMOVE => {
            let mut tme = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
            TrackMouseEvent(&mut tme);
            set_hover(hwnd, hit_at(hwnd, lp));
        }
        WM_MOUSELEAVE => set_hover(hwnd, None),
        WM_LBUTTONUP => {
            let mut cfg = SETTINGS.get();
            match hit_at(hwnd, lp) {
                Some(Hit::Corner(i)) => cfg.corner = i,
                Some(Hit::Margin(i)) => cfg.margin = i,
                Some(Hit::Distance) => cfg.distance = !cfg.distance,
                Some(Hit::Update) if matches!(update::status(), Status::Available(_)) => update::open_releases(),
                Some(Hit::Update) => {
                    let hwnd = hwnd as isize;
                    update::check(move || _ = InvalidateRect(hwnd as HWND, null(), 0));
                    InvalidateRect(hwnd as HWND, null(), 0);
                }
                Some(Hit::Exit) => PostQuitMessage(0),
                None => {}
            }
            if SETTINGS.replace(cfg) != cfg {
                cfg.save();
                InvalidateRect(hwnd, null(), 0);
            }
        }
        _ => return DefWindowProcW(hwnd, msg, wp, lp),
    }
    0
}

pub unsafe fn create() {
    let class = WNDCLASSW {
        style: CS_DROPSHADOW,
        lpfnWndProc: Some(proc),
        hInstance: GetModuleHandleW(null()),
        hCursor: LoadCursorW(null_mut(), IDC_ARROW),
        lpszClassName: w!("rblx-spot-popup"),
        ..zeroed()
    };
    RegisterClassW(&class);
    let ex = WS_EX_TOOLWINDOW | WS_EX_TOPMOST;
    let hwnd = CreateWindowExW(ex, class.lpszClassName, w!("rblx-spot"), WS_POPUP, 0, 0, 0, 0, null_mut(), null_mut(), class.hInstance, null());
    DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE as u32, (&DWMWCP_ROUND as *const i32).cast(), 4);
    POPUP.set(hwnd);
}

pub unsafe fn show(at: POINT) {
    // the tray click that should close the popup deactivates it first
    if HIDDEN_AT.get().is_some_and(|t| t.elapsed() < Duration::from_millis(300)) {
        return;
    }
    let hwnd = POPUP.get();
    let mon = MonitorFromPoint(at, MONITOR_DEFAULTTONEAREST);
    let (mut dpi, mut _dpi_y) = (96, 96);
    GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dpi, &mut _dpi_y);
    let mut mi: MONITORINFO = zeroed();
    mi.cbSize = size_of::<MONITORINFO>() as u32;
    GetMonitorInfoW(mon, &mut mi);

    let s = dpi as f32 / 96.0;
    let (w, h, gap) = ((W * s).round() as i32, (H * s).round() as i32, (12.0 * s) as i32);
    let work = mi.rcWork;
    let x = (at.x - w / 2).min(work.right - w - gap).max(work.left + gap);
    let y = (at.y - h - gap).min(work.bottom - h - gap).max(work.top + gap);
    let border = bgr(theme().stroke);
    DwmSetWindowAttribute(hwnd, DWMWA_BORDER_COLOR as u32, (&border as *const u32).cast(), 4);
    SetWindowPos(hwnd, HWND_TOPMOST, x, y, w, h, SWP_SHOWWINDOW);
    SetForegroundWindow(hwnd);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_and_layout() {
        let mut px = vec![0u32; 100];
        let mut c = Canvas { px: &mut px, w: 10, h: 10, s: 1.0 };
        c.fill(R { x: 2.0, y: 2.0, w: 6.0, h: 6.0 }, 3.0, 0xFFFFFF);
        c.fill(R { x: -5.0, y: 9.0, w: 30.0, h: 30.0 }, 4.0, 0xFF0000);
        c.fill(R { x: 5.0, y: 5.0, w: -2.0, h: 0.0 }, 4.0, 0x00FF00);
        assert_eq!(px[5 * 10 + 5], 0xFFFFFF);
        assert_eq!(px[2 * 10 + 2], 0);
        for s in [1.0, 1.1, 1.25, 1.75, 2.25] {
            Canvas { px: &mut px, w: 10, h: 10, s }.image(7.0 / s, -2.0, 4.0, |n| vec![0xFF123456; (n * n) as usize]);
        }
        assert_eq!(px[10 + 8], 0x123456);

        let all: Vec<_> = hits().map(|(_, r)| r).collect();
        for (i, a) in all.iter().enumerate() {
            assert!(a.x >= 0.0 && a.y >= 0.0 && a.x + a.w <= W && a.y + a.h <= H);
            for b in &all[i + 1..] {
                assert!(a.x + a.w <= b.x || b.x + b.w <= a.x || a.y + a.h <= b.y || b.y + b.h <= a.y);
            }
        }
    }
}
