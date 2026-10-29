use std::path::PathBuf;

use serde_json::{json, Value};

pub const MARGINS: [i32; 5] = [0, 8, 16, 32, 64];
pub const SIZES: [i32; 5] = [8, 10, 12, 14, 18];

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Settings {
    pub corner: usize, // bit 0 = right, bit 1 = bottom
    pub margin: usize,
    pub distance: bool,
    pub size: usize,
}

const DEFAULT: Settings = Settings { corner: 3, margin: 2, distance: true, size: 1 };

fn path() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("APPDATA")?).join("rblx-spot").join("settings.json"))
}

impl Settings {
    pub fn load() -> Self {
        let bytes = path().and_then(|p| std::fs::read(p).ok());
        Self::from_json(&bytes.and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default())
    }

    pub fn save(self) {
        if let Some(p) = path() {
            let _ = std::fs::create_dir_all(p.parent().unwrap());
            let _ = std::fs::write(p, self.to_json().to_string());
        }
    }

    fn from_json(v: &Value) -> Self {
        let margin_px = v["margin"].as_i64();
        let size_pt = v["size"].as_i64();
        Settings {
            corner: v["corner"].as_u64().map(|c| c as usize).filter(|&c| c < 4).unwrap_or(DEFAULT.corner),
            margin: MARGINS.iter().position(|&m| Some(m as i64) == margin_px).unwrap_or(DEFAULT.margin),
            distance: v["distance"].as_bool().unwrap_or(DEFAULT.distance),
            size: SIZES.iter().position(|&p| Some(p as i64) == size_pt).unwrap_or(DEFAULT.size),
        }
    }

    fn to_json(self) -> Value {
        json!({ "corner": self.corner, "margin": MARGINS[self.margin], "distance": self.distance, "size": SIZES[self.size] })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let s = Settings { corner: 0, margin: 4, distance: false, size: 4 };
        assert_eq!(Settings::from_json(&s.to_json()), s);
        assert_eq!(Settings::from_json(&Value::Null), DEFAULT);
        assert_eq!(Settings::from_json(&json!({ "corner": 9, "margin": 7, "distance": "x", "size": 11 })), DEFAULT);
    }
}
