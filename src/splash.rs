use std::mem::zeroed;
use std::ptr::{null, null_mut};
use std::time::{Duration, Instant};

use windows_sys::w;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::HiDpi::GetDpiForSystem;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::overlay::{blend, dib_info};

const SIZE_DIP: f32 = 160.0;
const TOTAL: f32 = 1.3;

pub fn show() {
    std::thread::spawn(|| unsafe { run() });
}

fn anim(t: f32) -> (f32, f32) {
    if t < 0.35 {
        let x = t / 0.35;
        let c = 2.5;
        let back = 1.0 + (c + 1.0) * (x - 1.0).powi(3) + c * (x - 1.0).powi(2);
        (0.4 + 0.6 * back, (x * 2.0).min(1.0))
    } else if t < 0.85 {
        (1.0, 1.0)
    } else {
        let x = ((t - 0.85) / 0.45).min(1.0);
        let e = x * x * (3.0 - 2.0 * x);
        (1.0 + 0.08 * e, 1.0 - e)
    }
}

// resource 1 = icon.ico, embedded by build.rs
pub unsafe fn icon_pixels(n: i32) -> Vec<u32> {
    let icon = LoadImageW(GetModuleHandleW(null()), 1 as _, IMAGE_ICON, n, n, LR_DEFAULTCOLOR);
    let mut info: ICONINFO = zeroed();
    GetIconInfo(icon, &mut info);
    let mut px = vec![0u32; (n * n) as usize];
    let dc = CreateCompatibleDC(null_mut());
    GetDIBits(dc, info.hbmColor, 0, n as u32, px.as_mut_ptr().cast(), &mut dib_info(n, n), DIB_RGB_COLORS);
    DeleteDC(dc);
    DeleteObject(info.hbmColor);
    DeleteObject(info.hbmMask);
    DestroyIcon(icon);
    for p in &mut px {
        let a = *p >> 24;
        let pm = |shift: u32| ((*p >> shift & 0xFF) * a / 255) << shift;
        *p = a << 24 | pm(16) | pm(8) | pm(0);
    }
    px
}

fn draw(src: &[u32], n: usize, dst: &mut [u32], w: usize, size: f32) {
    let k = n as f32 / size;
    let at = |x: f32, y: f32| {
        if x < 0.0 || y < 0.0 || x >= n as f32 || y >= n as f32 { 0 } else { src[y as usize * n + x as usize] }
    };
    for y in 0..w {
        for x in 0..w {
            let u = (x as f32 + 0.5 - w as f32 / 2.0) * k + n as f32 / 2.0 - 0.5;
            let v = (y as f32 + 0.5 - w as f32 / 2.0) * k + n as f32 / 2.0 - 0.5;
            let (x0, y0) = (u.floor(), v.floor());
            let (fx, fy) = (u - x0, v - y0);
            let quad = [at(x0, y0), at(x0 + 1.0, y0), at(x0, y0 + 1.0), at(x0 + 1.0, y0 + 1.0)];
            dst[y * w + x] = [0, 8, 16, 24].iter().fold(0, |out, &s| {
                let c = quad.map(|p| (p >> s & 0xFF) as f32);
                let top = c[0] + (c[1] - c[0]) * fx;
                let bottom = c[2] + (c[3] - c[2]) * fx;
                out | ((top + (bottom - top) * fy).round() as u32) << s
            });
        }
    }
}

unsafe fn run() {
    let base = SIZE_DIP * GetDpiForSystem() as f32 / 96.0;
    let n = base.round() as i32;
    let src = icon_pixels(n);
    let w = (base * 1.25) as i32; // must exceed anim()'s peak scale (~1.1)
    let pos = POINT { x: (GetSystemMetrics(SM_CXSCREEN) - w) / 2, y: (GetSystemMetrics(SM_CYSCREEN) - w) / 2 };

    let ex = WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TOPMOST;
    let hwnd = CreateWindowExW(ex, w!("STATIC"), null(), WS_POPUP, pos.x, pos.y, w, w, null_mut(), null_mut(), null_mut(), null());
    let dc = CreateCompatibleDC(null_mut());
    let mut bits = null_mut();
    let bmp = CreateDIBSection(dc, &dib_info(w, w), DIB_RGB_COLORS, &mut bits, null_mut(), 0);
    if bits.is_null() {
        DestroyWindow(hwnd);
        DeleteDC(dc);
        return;
    }
    let old = SelectObject(dc, bmp);
    let dst = std::slice::from_raw_parts_mut(bits as *mut u32, (w * w) as usize);
    ShowWindow(hwnd, SW_SHOWNOACTIVATE);

    let start = Instant::now();
    loop {
        let t = start.elapsed().as_secs_f32();
        if t >= TOTAL {
            break;
        }
        let (scale, opacity) = anim(t);
        draw(&src, n as usize, dst, w as usize, base * scale);
        GdiFlush();
        let size = SIZE { cx: w, cy: w };
        UpdateLayeredWindow(hwnd, null_mut(), &pos, &size, dc, &POINT { x: 0, y: 0 }, 0, &blend((opacity * 255.0) as u8), ULW_ALPHA);
        let mut msg = zeroed();
        while PeekMessageW(&mut msg, null_mut(), 0, 0, PM_REMOVE) != 0 {
            DispatchMessageW(&msg);
        }
        std::thread::sleep(Duration::from_millis(15));
    }

    DestroyWindow(hwnd);
    SelectObject(dc, old);
    DeleteObject(bmp);
    DeleteDC(dc);
}
