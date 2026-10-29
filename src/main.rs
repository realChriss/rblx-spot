// Constraint (anti-cheat): never touch the Roblox process (OpenProcess, memory, injection, hooks).
// Inputs are only the log files and window queries (FindWindow/GetClientRect).
#![windows_subsystem = "windows"]

mod geo;
mod log;
mod overlay;
mod popup;
mod settings;
mod sound;
mod splash;
mod tray;
mod update;

use std::mem::zeroed;
use std::ptr::{null, null_mut};

use settings::Settings;
use windows_sys::w;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::HiDpi::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

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
            overlay::render(self.overlay, &key.0, key.1, key.2, key.3);
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
        tray::create();

        let overlay = overlay::create();

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
        tray::remove();
    }
}

