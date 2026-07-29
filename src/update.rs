//! Soft "new version available" whisper — one cached GitHub Releases check.
//! No telemetry; failures are silent; opt out with `check_updates: false`.

use crate::config::VERSION;
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

const UPDATE_CHECK_SECONDS: f64 = 24.0 * 60.0 * 60.0;
const UPDATE_API_URL: &str =
    "https://api.github.com/repos/brendan-jarvis/meanwhile/releases/latest";
const UPDATE_PAGE_URL: &str = "https://github.com/brendan-jarvis/meanwhile/releases/latest";

fn update_cache_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".cache/meanwhile/update.json")
}

/// Comparable x.y.z from a tag like `v0.5.1` or `0.5.1`.
fn release_version(tag: &str) -> Option<(u32, u32, u32)> {
    let t = tag.trim().strip_prefix('v').unwrap_or(tag.trim());
    let mut parts = t.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

fn now_secs() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

fn read_cache() -> Value {
    match fs::read_to_string(update_cache_path()) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_else(|_| json!({})),
        Err(_) => json!({}),
    }
}

fn write_cache(data: &Value) {
    let path = update_cache_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(text) = serde_json::to_string(data) {
        let _ = fs::write(path, text);
    }
}

#[derive(Clone, Debug)]
pub struct UpdateOffer {
    pub tag: String,
    pub url: String,
}

/// Background one-shot (plus cache) release check.
pub struct UpdateChecker {
    available: Mutex<Option<UpdateOffer>>,
}

impl UpdateChecker {
    /// Spawn a daemon thread when `enabled`. Always returns a handle so the UI
    /// can call [`take_available`] without caring whether a check ran.
    pub fn start(enabled: bool) -> Arc<Self> {
        let this = Arc::new(Self {
            available: Mutex::new(None),
        });
        if enabled {
            let worker = Arc::clone(&this);
            let _ = thread::Builder::new()
                .name("updates".into())
                .spawn(move || worker.run());
        }
        this
    }

    fn run(&self) {
        let mut data = read_cache();
        let fresh = data
            .get("checked_at")
            .and_then(|v| v.as_f64())
            .map(|at| now_secs() - at < UPDATE_CHECK_SECONDS)
            .unwrap_or(false);

        if !fresh {
            let checked_at = now_secs();
            if let Some(tag) = fetch_latest_tag() {
                if release_version(&tag).is_some() {
                    data["tag"] = json!(tag);
                }
            }
            data["checked_at"] = json!(checked_at);
            write_cache(&data);
        }

        self.offer_from_cache(&data);
    }

    fn offer_from_cache(&self, data: &Value) {
        let Some(tag) = data.get("tag").and_then(|v| v.as_str()) else {
            return;
        };
        let Some(latest) = release_version(tag) else {
            return;
        };
        let Some(current) = release_version(VERSION) else {
            return;
        };
        if latest <= current {
            return;
        }
        let notified = data.get("notified").and_then(|v| v.as_str()).unwrap_or("");
        if notified == tag {
            return;
        }
        if let Ok(mut g) = self.available.lock() {
            *g = Some(UpdateOffer {
                tag: tag.to_string(),
                url: UPDATE_PAGE_URL.into(),
            });
        }
    }

    /// Pop a pending offer once (and mark that tag as notified in the cache).
    pub fn take_available(&self) -> Option<UpdateOffer> {
        let offer = self.available.lock().ok().and_then(|mut g| g.take())?;
        let mut data = read_cache();
        data["notified"] = json!(&offer.tag);
        write_cache(&data);
        Some(offer)
    }
}

fn fetch_latest_tag() -> Option<String> {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(3))
        .timeout_read(std::time::Duration::from_secs(4))
        .build();
    let resp = agent
        .get(UPDATE_API_URL)
        .set("Accept", "application/vnd.github+json")
        .set("User-Agent", &format!("meanwhile/{VERSION}"))
        .call()
        .ok()?;
    let v: Value = resp.into_json().ok()?;
    v.get("tag_name")
        .and_then(|t| t.as_str())
        .map(|s| s.to_string())
}
