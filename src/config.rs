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
    /// Target frames per second (ambient rain; 12 feels like classic animation).
    #[serde(default = "default_fps")]
    pub fps: f64,
    pub focus: bool,
    /// "auto" adopts the active Omarchy theme; "matrix" forces green
    pub theme: String,
    /// append the domain after each headline
    pub show_source: bool,
    pub ascii_only: bool,
    pub env_files: Vec<String>,
    #[serde(default = "default_mouse")]
    pub mouse: bool,
}

fn default_mouse() -> bool {
    true
}

fn default_fps() -> f64 {
    12.0
}

impl Default for Config {
    fn default() -> Self {
        Self {
            topics: vec![
                "world news".into(),
                "artificial intelligence".into(),
                "uk".into(),
            ],
            places: vec![],
            refresh_minutes: 15.0,
            hours_back: 36.0,
            poetic_ratio: 0.4,
            message_every_seconds: 1.8,
            density: 0.75,
            speed: 1.0,
            fps: 12.0,
            focus: false,
            theme: "auto".into(),
            show_source: false,
            ascii_only: false,
            env_files: vec![
                "~/dev/tom-os/.env".into(),
                "~/dev/exa-newsdesk/.env".into(),
            ],
            mouse: true,
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

    // migrate v0.1 auto-written defaults to the denser v0.2 feel
    if user.get("density").and_then(|v| v.as_f64()) == Some(0.45) {
        if let Some(obj) = user.as_object_mut() {
            obj.remove("density");
        }
    }
    if user.get("message_every_seconds").and_then(|v| v.as_f64()) == Some(3.0) {
        if let Some(obj) = user.as_object_mut() {
            obj.remove("message_every_seconds");
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
    for env_file in &cfg.env_files {
        let path = expand_user(env_file);
        let text = match fs::read_to_string(&path) {
            Ok(t) => t,
            Err(_) => continue,
        };
        for line in text.lines() {
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
