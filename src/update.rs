use std::ptr::{null, null_mut};
use std::sync::Mutex;

use windows_sys::w;
use windows_sys::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE};
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

const REPO: &str = "realChriss/rblx-spot";

#[derive(Clone, PartialEq)]
pub enum Status {
    Idle,
    Checking,
    Latest,
    Available(String),
    Failed,
}

static STATUS: Mutex<Status> = Mutex::new(Status::Idle);

pub fn status() -> Status {
    STATUS.lock().unwrap().clone()
}

pub fn check(on_done: impl FnOnce() + Send + 'static) {
    let mut status = STATUS.lock().unwrap();
    if *status == Status::Checking {
        return;
    }
    *status = Status::Checking;
    drop(status);
    std::thread::spawn(move || {
        let release = crate::geo::fetch_json(&format!("https://api.github.com/repos/{REPO}/releases/latest"));
        *STATUS.lock().unwrap() = match release["tag_name"].as_str() {
            Some(tag) if newer(tag, env!("CARGO_PKG_VERSION")) => Status::Available(tag.to_owned()),
            Some(_) => Status::Latest,
            None => Status::Failed,
        };
        on_done();
    });
}

pub fn open_releases() {
    let url = w!("https://github.com/realChriss/rblx-spot/releases/latest");
    unsafe {
        CoInitializeEx(null(), (COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) as u32);
        ShellExecuteW(null_mut(), w!("open"), url, null_mut(), null_mut(), SW_SHOWNORMAL);
    }
}

fn newer(tag: &str, current: &str) -> bool {
    let parse = |v: &str| v.trim_start_matches('v').split('.').map(|p| p.parse().unwrap_or(0)).collect::<Vec<u32>>();
    parse(tag) > parse(current)
}

#[cfg(test)]
mod tests {
    use super::newer;

    #[test]
    fn versions() {
        assert!(newer("v0.1.2", "0.1.1"));
        assert!(newer("v0.10.0", "0.9.9"));
        assert!(newer("v1.0.0", "0.99.99"));
        assert!(!newer("v0.1.1", "0.1.1"));
        assert!(!newer("v0.1.0", "0.1.1"));
    }
}
