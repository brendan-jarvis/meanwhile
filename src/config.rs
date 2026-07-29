use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn config_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".config/meanwhile/config.json")
}

pub fn cache_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".cache/meanwhile/headlines.json")
}

/// Last feed-fetch diagnostic log (written every refresh).
pub fn fetch_log_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".cache/meanwhile/last-fetch.log")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub topics: Vec<String>,
    pub places: Vec<String>,
    pub refresh_minutes: f64,
    pub hours_back: f64,
    pub poetic_ratio: f64,
    pub message_every_seconds: f64,
    pub density: f64,
    pub speed: f64,
    /// Target frames per second (ambient rain; 8 keeps the PTY quiet).
    #[serde(default = "default_fps")]
    pub fps: f64,
    pub focus: bool,
    /// "auto" inherits the terminal / WezTerm / Starship / Omarchy palette;
    /// "matrix" forces classic green
    pub theme: String,
    /// append the domain after each headline
    pub show_source: bool,
    pub ascii_only: bool,
    pub env_files: Vec<String>,
    /// Extra RSS/Atom feed URLs (always fetched as general news).
    #[serde(default)]
    pub extra_feeds: Vec<String>,
    /// `"rain"` (default) or `"ticker"` — pure stock marquee, no matrix field.
    #[serde(default = "default_mode")]
    pub mode: String,
    /// Yahoo Finance symbols for ticker mode (e.g. AAPL, SPY, BTC-USD, FBU.NZ).
    #[serde(default = "default_tickers")]
    pub tickers: Vec<String>,
    /// Optional edge marquee bar while raining (`top`/`bottom`/`left`/`right`).
    #[serde(default = "default_rain_ticker")]
    pub rain_ticker: bool,
    /// Edge for the optional marquee bar: `top`, `bottom`, `left`, or `right`.
    #[serde(default = "default_rain_ticker_edge")]
    pub rain_ticker_edge: String,
    /// Decode individual quotes into the rain among news and poetic lines.
    #[serde(default = "default_quotes_in_rain")]
    pub quotes_in_rain: bool,
    /// When quotes are on, chance a spawned line is a stock quote (0–1).
    #[serde(default = "default_quotes_ratio")]
    pub quotes_ratio: f64,
    #[serde(default = "default_mouse")]
    pub mouse: bool,
    /// One cached GitHub release check per day (soft toast). Set false to opt out.
    #[serde(default = "default_check_updates")]
    pub check_updates: bool,
}

fn default_mouse() -> bool {
    true
}

fn default_check_updates() -> bool {
    true
}

fn default_fps() -> f64 {
    8.0
}

fn default_mode() -> String {
    "rain".into()
}

fn default_tickers() -> Vec<String> {
    // Alias expanded at runtime to the full S&P 500 list (~500 names).
    // Use ["SP250"] for the first 250 symbols, or list symbols explicitly.
    vec!["SP500".into()]
}

fn default_rain_ticker() -> bool {
    false
}

fn default_rain_ticker_edge() -> String {
    "bottom".into()
}

fn default_quotes_in_rain() -> bool {
    true
}

fn default_quotes_ratio() -> f64 {
    0.28
}

impl Default for Config {
    fn default() -> Self {
        Self {
            topics: vec![
                "world news".into(),
                "technology".into(),
            ],
            places: vec![],
            refresh_minutes: 15.0,
            hours_back: 48.0,
            poetic_ratio: 0.35,
            message_every_seconds: 1.8,
            density: 0.45,
            speed: 1.0,
            fps: 8.0,
            focus: false,
            theme: "auto".into(),
            show_source: false,
            ascii_only: false,
            // Look for EXA_API_KEY here (plus the process environment).
            env_files: vec![
                "~/.config/meanwhile/.env".into(),
                "~/.env".into(),
            ],
            extra_feeds: vec![],
            mode: "rain".into(),
            tickers: default_tickers(),
            rain_ticker: false,
            rain_ticker_edge: default_rain_ticker_edge(),
            quotes_in_rain: true,
            quotes_ratio: 0.28,
            mouse: true,
            check_updates: true,
        }
    }
}

pub fn load_config() -> Config {
    let path = config_path();
    let mut cfg = Config::default();
    let text = match fs::read_to_string(&path) {
        Ok(t) => t,
        Err(_) => {
            save_config(&cfg);
            return cfg;
        }
    };
    let mut user: Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(_) => return cfg,
    };

    // migrate prior auto-written denser/faster defaults → quieter ambient
    if let Some(obj) = user.as_object_mut() {
        if obj.get("density").and_then(|v| v.as_f64()) == Some(0.75) {
            obj.remove("density");
        }
        if matches!(
            obj.get("fps").and_then(|v| v.as_f64()),
            Some(12.0) | Some(20.0) | Some(30.0)
        ) {
            obj.remove("fps");
        }
        if obj.get("message_every_seconds").and_then(|v| v.as_f64()) == Some(3.0) {
            obj.remove("message_every_seconds");
        }
        // Old packaged defaults pointed at another developer's machine.
        if let Some(files) = obj.get("env_files").and_then(|v| v.as_array()) {
            let only_legacy = files.iter().all(|v| {
                matches!(
                    v.as_str(),
                    Some("~/dev/tom-os/.env" | "~/dev/exa-newsdesk/.env")
                )
            });
            if only_legacy && !files.is_empty() {
                obj.insert(
                    "env_files".into(),
                    Value::Array(vec![
                        Value::String("~/.config/meanwhile/.env".into()),
                        Value::String("~/.env".into()),
                    ]),
                );
            }
        }
        // UK-centric default topic set → neutral (only if still the old triple)
        if let Some(topics) = obj.get("topics").and_then(|v| v.as_array()) {
            let names: Vec<&str> = topics.iter().filter_map(|v| v.as_str()).collect();
            if names == ["world news", "artificial intelligence", "uk"] {
                obj.insert(
                    "topics".into(),
                    Value::Array(vec![
                        Value::String("world news".into()),
                        Value::String("technology".into()),
                    ]),
                );
            }
        }
        // Old short mega-cap ticker list → full S&P 500 universe
        if let Some(tickers) = obj.get("tickers").and_then(|v| v.as_array()) {
            let names: Vec<&str> = tickers.iter().filter_map(|v| v.as_str()).collect();
            let old = [
                "SPY", "QQQ", "DIA", "IWM", "AAPL", "MSFT", "GOOGL", "AMZN", "NVDA", "META",
                "TSLA", "BRK-B", "JPM", "V", "XOM", "JNJ", "WMT", "BTC-USD", "ETH-USD",
            ];
            if names.len() == old.len() && names.iter().all(|n| old.contains(n)) {
                obj.insert(
                    "tickers".into(),
                    Value::Array(vec![Value::String("SP500".into())]),
                );
            }
        }
    }
    if let Some(env_file) = user.get("env_file").and_then(|v| v.as_str()).map(str::to_string) {
        if let Some(obj) = user.as_object_mut() {
            obj.remove("env_file");
            if !obj.contains_key("env_files") {
                obj.insert("env_files".into(), Value::Array(vec![Value::String(env_file)]));
            }
        }
    }
    if let Some(place) = user.get("place").and_then(|v| v.as_str()).map(|s| s.trim().to_string()) {
        if let Some(obj) = user.as_object_mut() {
            obj.remove("place");
            if !place.is_empty() && !obj.contains_key("places") {
                obj.insert("places".into(), Value::Array(vec![Value::String(place)]));
            }
        }
    }

    if let Ok(user_cfg) = serde_json::from_value::<Config>(user) {
        cfg = user_cfg;
    }
    save_config(&cfg);
    cfg
}

pub fn save_config(cfg: &Config) {
    let path = config_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(text) = serde_json::to_string_pretty(cfg) {
        let _ = fs::write(&path, format!("{text}\n"));
    }
}

pub fn resolve_api_key(cfg: &Config) -> Option<String> {
    if let Ok(key) = std::env::var("EXA_API_KEY") {
        if !key.is_empty() {
            return Some(key);
        }
    }
    // Configured paths, then a few always-checked fallbacks.
    let mut paths: Vec<PathBuf> = cfg.env_files.iter().map(|p| expand_user(p)).collect();
    for extra in [
        "~/.config/meanwhile/.env",
        "~/.env",
        "~/dev/tom-os/.env",
        "~/dev/exa-newsdesk/.env",
    ] {
        let p = expand_user(extra);
        if !paths.contains(&p) {
            paths.push(p);
        }
    }
    for path in paths {
        let text = match fs::read_to_string(&path) {
            Ok(t) => t,
            Err(_) => continue,
        };
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(rest) = line.strip_prefix("EXA_API_KEY=") {
                let key = rest.trim().trim_matches('"').trim_matches('\'').to_string();
                if !key.is_empty() {
                    return Some(key);
                }
            }
        }
    }
    None
}

fn expand_user(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    }
    if path == "~" {
        if let Some(home) = dirs::home_dir() {
            return home;
        }
    }
    PathBuf::from(path)
}

#[allow(dead_code)]
pub fn expand_path(p: impl AsRef<Path>) -> PathBuf {
    expand_user(p.as_ref().to_string_lossy().as_ref())
}
