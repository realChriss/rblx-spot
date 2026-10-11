use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use ureq::tls::{TlsConfig, TlsProvider};

const UNKNOWN: &str = "region unknown";

type Located = (String, Option<u32>);

static GEO: LazyLock<Mutex<HashMap<String, Located>>> = LazyLock::new(Default::default);
static HOME: Mutex<Option<(f64, f64)>> = Mutex::new(None);

pub fn get(ip: &str) -> Located {
    GEO.lock().unwrap().get(ip).cloned().unwrap_or_default()
}

pub fn lookup(ip: &str) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distance() {
        let berlin_frankfurt = distance_km((52.52, 13.405), (50.11, 8.682));
        assert!((berlin_frankfurt - 424.0).abs() < 5.0, "{berlin_frankfurt}");
        assert_eq!(distance_km((10.0, 20.0), (10.0, 20.0)), 0.0);
        assert_eq!(coords(&serde_json::json!({"loc": "50.1155,8.6842"})), Some((50.1155, 8.6842)));
        assert_eq!(coords(&serde_json::json!({"bogon": true})), None);
    }
}
