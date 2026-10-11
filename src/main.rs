// Constraint (anti-cheat): never touch the Roblox process (OpenProcess, memory, injection, hooks).
// Inputs are only the log files and window queries (FindWindow/GetClientRect).
#![windows_subsystem = "windows"]

mod sound;
mod splash;

use std::collections::HashMap;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::mem::{size_of, zeroed};
use std::path::PathBuf;
use std::ptr::{null, null_mut};
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use tray_icon::menu::{Menu, MenuEvent, MenuItem};
use tray_icon::{Icon, TrayIconBuilder};
use ureq::tls::{TlsConfig, TlsProvider};
use windows_sys::w;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::HiDpi::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const BOX_ALPHA: u32 = 178;
const TICK_MS: u32 = 250;
const UNKNOWN: &str = "region unknown";

type Located = (String, Option<u32>);

static GEO: LazyLock<Mutex<HashMap<String, Located>>> = LazyLock::new(Default::default);
static HOME: Mutex<Option<(f64, f64)>> = Mutex::new(None);

// Roblox logs a teleport's old-server disconnect after the new address; serverId is always last.
fn last_event(chunk: &str) -> Option<Option<(String, String)>> {
    let joined = chunk.rfind("serverId: ");
    let left = chunk.rfind("Time to disconnect replication data");
    if left > joined {
        return Some(None);
    }
    let (ip, rest) = chunk[joined? + 10..].split_once('|')?;
    let port: String = rest.chars().take_while(char::is_ascii_digit).collect();
    let valid = !ip.is_empty() && ip.chars().all(|c| c.is_ascii_digit() || c == '.') && !port.is_empty();
    valid.then(|| Some((ip.to_owned(), port)))
}

#[derive(Default)]
struct Log {
    path: Option<PathBuf>,
    pos: u64,
    server: Option<(String, String)>,
}

impl Log {
    fn open_newest(&mut self) {
        let newest = (|| {
            let dir = PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join(r"Roblox\logs");
            let logs = fs::read_dir(dir).ok()?.flatten().filter(|e| {
                let n = e.file_name().to_string_lossy().into_owned();
                n.contains("_Player_") && n.ends_with("_last.log") && !n.contains("CrashHandler")
            });
            Some(logs.max_by_key(|e| e.metadata().and_then(|m| m.modified()).ok())?.path())
        })();
        *self = Log { path: newest, ..Default::default() };
    }

    fn read_new(&mut self) -> std::io::Result<()> {
        let Some(path) = &self.path else { return Ok(()) };
        let mut f = fs::File::open(path)?;
        f.seek(SeekFrom::Start(self.pos))?;
        let mut chunk = Vec::new();
        f.read_to_end(&mut chunk)?;
        let end = chunk.iter().rposition(|&b| b == b'\n').map_or(0, |i| i + 1);
        self.pos += end as u64;
        if let Some(event) = last_event(&String::from_utf8_lossy(&chunk[..end])) {
            if let Some((ip, _)) = &event {
                lookup(ip);
            }
            self.server = event;
        }
        Ok(())
    }
}

fn lookup(ip: &str) {
    let mut geo = GEO.lock().unwrap();
    if geo.get(ip).is_some_and(|(region, _)| region != UNKNOWN) {
        return;
    }
    geo.insert(ip.to_owned(), ("locating…".into(), None));
    let ip = ip.to_owned();
    std::thread::spawn(move || {
        let located = locate(&ip);
        GEO.lock().unwrap().insert(ip, located);
    });
}

fn locate(ip: &str) -> Located {
    let info = ipinfo(&format!("{ip}/json"));
    let parts: Vec<&str> = ["city", "region", "country"]
        .iter()
        .filter_map(|k| info[k].as_str())
        .filter(|s| !s.is_empty())
        .collect();
    let region = if parts.is_empty() { UNKNOWN.into() } else { parts.join(", ") };
    let km = coords(&info).zip(home()).map(|(a, b)| ((distance_km(a, b) / 10.0).round() * 10.0) as u32);
    (region, km)
}

fn home() -> Option<(f64, f64)> {
    let mut home = HOME.lock().unwrap();
    if home.is_none() {
        *home = coords(&ipinfo("json"));
    }
    *home
}

fn coords(info: &serde_json::Value) -> Option<(f64, f64)> {
    let (lat, lon) = info["loc"].as_str()?.split_once(',')?;
    Some((lat.parse().ok()?, lon.parse().ok()?))
}

fn distance_km((lat1, lon1): (f64, f64), (lat2, lon2): (f64, f64)) -> f64 {
    let (p1, p2) = (lat1.to_radians(), lat2.to_radians());
    let a = ((p2 - p1) / 2.0).sin().powi(2) + p1.cos() * p2.cos() * ((lon2 - lon1).to_radians() / 2.0).sin().powi(2);
    12742.0 * a.sqrt().asin()
}

fn ipinfo(path: &str) -> serde_json::Value {
    ureq::get(format!("https://ipinfo.io/{path}"))
        .config()
        .timeout_global(Some(Duration::from_secs(5)))
        .tls_config(TlsConfig::builder().provider(TlsProvider::NativeTls).build()) // without this ureq panics: rustls feature is off
        .build()
        .call()
        .and_then(|mut r| r.body_mut().read_json())
        .unwrap_or_default()
}

struct App {
    overlay: HWND,
    roblox: HWND,
    log: Log,
    shown: Option<(String, i32, i32, u32)>,
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
        let (region, km) = GEO.lock().unwrap().get(ip).cloned().unwrap_or_default();
        let mut text = format!("Server    {ip}:{port}\nRegion    {region}");
        // IP geolocation is city-level, so closer than that is noise
        match km {
            Some(km) if km < 50 => text += "\nDistance  < 50 km",
            Some(km) => text += &format!("\nDistance  ~{km} km"),
            None => {}
        }

        let mut r: RECT = zeroed();
        GetClientRect(w, &mut r);
        let mut corner = POINT { x: r.right, y: r.bottom };
        ClientToScreen(w, &mut corner);
        let key = (text, corner.x, corner.y, GetDpiForWindow(w));
        if self.shown.as_ref() != Some(&key) {
            render(self.overlay, &key.0, corner, key.3);
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

unsafe fn render(hwnd: HWND, text: &str, corner: POINT, dpi: u32) {
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

    // GDI text leaves alpha at 0
    for p in std::slice::from_raw_parts_mut(bits as *mut u32, (w * h) as usize) {
        let c = *p & 0xFF;
        *p = ((BOX_ALPHA + c * (255 - BOX_ALPHA) / 255) << 24) | (c * 0x010101);
    }

    let pos = POINT { x: corner.x - w - px(16), y: corner.y - h - px(16) };
    UpdateLayeredWindow(hwnd, null_mut(), &pos, &SIZE { cx: w, cy: h }, dc, &POINT { x: 0, y: 0 }, 0, &blend(255), ULW_ALPHA);

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

        let menu = Menu::with_items(&[&MenuItem::new("Exit", true, None)]).unwrap();
        MenuEvent::set_event_handler(Some(|_| PostQuitMessage(0)));
        let tray_px = GetSystemMetrics(SM_CXSMICON) as u32;
        let _tray = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip("rblx-spot")
            .with_icon(Icon::from_resource(1, Some((tray_px, tray_px))).unwrap()) // id 1 = icon.ico via build.rs
            .build()
            .unwrap();

        let ex = WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TOPMOST;
        let overlay = CreateWindowExW(ex, w!("STATIC"), null(), WS_POPUP, 0, 0, 0, 0, null_mut(), null_mut(), null_mut(), null());

        SetTimer(null_mut(), 0, TICK_MS, None);
        let mut app = App { overlay, roblox: null_mut(), log: Log::default(), shown: None };
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_events() {
        let s = |ip: &str, port: &str| Some(Some((ip.to_owned(), port.to_owned())));
        assert_eq!(last_event("nothing here\n"), None);
        assert_eq!(last_event("x serverId: 128.116.1.2|55123\n"), s("128.116.1.2", "55123"));
        assert_eq!(last_event("serverId: 1.2.3.4|1\nTime to disconnect replication data\n"), Some(None));
        assert_eq!(last_event("Time to disconnect replication data\nserverId: 5.6.7.8|9\n"), s("5.6.7.8", "9"));
        assert_eq!(last_event("serverId: garbage\n"), None);
    }

    #[test]
    fn distance() {
        let berlin_frankfurt = distance_km((52.52, 13.405), (50.11, 8.682));
        assert!((berlin_frankfurt - 424.0).abs() < 5.0, "{berlin_frankfurt}");
        assert_eq!(distance_km((10.0, 20.0), (10.0, 20.0)), 0.0);
        assert_eq!(coords(&serde_json::json!({"loc": "50.1155,8.6842"})), Some((50.1155, 8.6842)));
        assert_eq!(coords(&serde_json::json!({"bogon": true})), None);
    }
}
