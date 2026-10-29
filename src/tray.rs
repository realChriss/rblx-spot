use std::cell::Cell;
use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};

use windows_sys::w;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Shell::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use crate::popup;

const WM_TRAY: u32 = WM_APP + 1;

thread_local! {
    static TRAY: Cell<HWND> = const { Cell::new(null_mut()) };
    static TASKBAR_CREATED: Cell<u32> = const { Cell::new(0) };
}

fn nid(hwnd: HWND) -> NOTIFYICONDATAW {
    let mut nid: NOTIFYICONDATAW = unsafe { zeroed() };
    nid.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
    nid.hWnd = hwnd;
    nid.uID = 1;
    nid
}

unsafe fn add(hwnd: HWND) {
    let mut nid = nid(hwnd);
    nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    nid.uCallbackMessage = WM_TRAY;
    let px = GetSystemMetrics(SM_CXSMICON);
    nid.hIcon = LoadImageW(GetModuleHandleW(null()), 1 as _, IMAGE_ICON, px, px, LR_SHARED); // id 1 = icon.ico via build.rs
    for (d, s) in nid.szTip.iter_mut().zip("rblx-spot".encode_utf16()) {
        *d = s;
    }
    Shell_NotifyIconW(NIM_ADD, &nid);
}

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_TRAY if lp as u32 == WM_LBUTTONUP => {
            let id = NOTIFYICONIDENTIFIER { cbSize: size_of::<NOTIFYICONIDENTIFIER>() as u32, hWnd: hwnd, uID: 1, ..zeroed() };
            let mut r: RECT = zeroed();
            if Shell_NotifyIconGetRect(&id, &mut r) == S_OK {
                popup::show(POINT { x: (r.left + r.right) / 2, y: r.top });
            }
        }
        _ if msg == TASKBAR_CREATED.get() => add(hwnd),
        _ => return DefWindowProcW(hwnd, msg, wp, lp),
    }
    0
}

pub unsafe fn create() {
    let taskbar_created = RegisterWindowMessageW(w!("TaskbarCreated"));
    TASKBAR_CREATED.set(taskbar_created);
    let class = WNDCLASSW {
        lpfnWndProc: Some(proc),
        hInstance: GetModuleHandleW(null()),
        lpszClassName: w!("rblx-spot-tray"),
        ..zeroed()
    };
    RegisterClassW(&class);
    let hwnd = CreateWindowExW(WS_EX_TOOLWINDOW, class.lpszClassName, null(), WS_POPUP, 0, 0, 0, 0, null_mut(), null_mut(), class.hInstance, null());
    ChangeWindowMessageFilterEx(hwnd, taskbar_created, MSGFLT_ALLOW, null_mut());
    add(hwnd);
    TRAY.set(hwnd);
}

pub unsafe fn remove() {
    Shell_NotifyIconW(NIM_DELETE, &nid(TRAY.get()));
}
