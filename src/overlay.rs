use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};

use windows_sys::w;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::settings::{Settings, MARGINS, SIZES};

const BOX_ALPHA: u32 = 178;

pub unsafe fn create() -> HWND {
    let ex = WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TOPMOST;
    CreateWindowExW(ex, w!("STATIC"), null(), WS_POPUP, 0, 0, 0, 0, null_mut(), null_mut(), null_mut(), null())
}

pub unsafe fn render(hwnd: HWND, text: &str, [l, t, r, b]: [i32; 4], dpi: u32, s: Settings) {
    let px = |v: i32| v * dpi as i32 / 96;
    let text: Vec<u16> = text.encode_utf16().collect();
    let dc = CreateCompatibleDC(null_mut());
    let font = CreateFontW(
        -((SIZES[s.size] * dpi as i32 + 36) / 72), 0, 0, 0, FW_NORMAL as i32, 0, 0, 0, DEFAULT_CHARSET as u32,
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

pub fn dib_info(w: i32, h: i32) -> BITMAPINFO {
    let mut bmi: BITMAPINFO = unsafe { zeroed() };
    bmi.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
    bmi.bmiHeader.biWidth = w;
    bmi.bmiHeader.biHeight = -h;
    bmi.bmiHeader.biPlanes = 1;
    bmi.bmiHeader.biBitCount = 32;
    bmi.bmiHeader.biCompression = BI_RGB;
    bmi
}

pub fn blend(alpha: u8) -> BLENDFUNCTION {
    BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: alpha, AlphaFormat: AC_SRC_ALPHA as u8 }
}
