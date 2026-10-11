use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;

use crate::geo;

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
pub struct Log {
    pub path: Option<PathBuf>,
    pos: u64,
    pub server: Option<(String, String)>,
}

impl Log {
    pub fn open_newest(&mut self) {
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

    pub fn read_new(&mut self) -> std::io::Result<()> {
        let Some(path) = &self.path else { return Ok(()) };
        let mut f = fs::File::open(path)?;
        f.seek(SeekFrom::Start(self.pos))?;
        let mut chunk = Vec::new();
        f.read_to_end(&mut chunk)?;
        let end = chunk.iter().rposition(|&b| b == b'\n').map_or(0, |i| i + 1);
        self.pos += end as u64;
        if let Some(event) = last_event(&String::from_utf8_lossy(&chunk[..end])) {
            if let Some((ip, _)) = &event {
                geo::lookup(ip);
            }
            self.server = event;
        }
        Ok(())
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
}
