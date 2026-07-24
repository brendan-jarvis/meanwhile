use crate::config::{cache_path, Config};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::fs;
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Headline {
    pub text: String,
    pub url: String,
    pub domain: String,
    pub kind: String, // "news" | "local"
}

struct FeedInner {
    items: Vec<Headline>,
    generation: u64,
    status: String,
    fetched_at: Option<SystemTime>,
    wake: bool,
}

/// Background Exa fetcher. Never panics into the UI; degrades to poetic-only.
pub struct Newsfeed {
    inner: Arc<(Mutex<FeedInner>, Condvar)>,
    cfg: Arc<Mutex<Config>>,
    api_key: Option<String>,
}

impl Newsfeed {
    pub fn new(cfg: Config, api_key: Option<String>) -> Self {
        let status = if api_key.is_some() {
            "connecting".into()
        } else {
            "no api key — poetic only".into()
        };
        let mut items = Vec::new();
        let mut generation = 0u64;

        // load cache
        if let Ok(text) = fs::read_to_string(cache_path()) {
            if let Ok(cached) = serde_json::from_str::<Value>(&text) {
                let at = cached.get("at").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs_f64();
                if now - at < 86400.0 {
                    let cached_places: Vec<String> = cached
                        .get("places")
                        .and_then(|v| serde_json::from_value(v.clone()).ok())
                        .unwrap_or_default();
                    let same_places = places_key(&cached_places) == places_key(&cfg.places);
                    if let Some(arr) = cached.get("items").and_then(|v| v.as_array()) {
                        for i in arr {
                            if let Ok(h) = serde_json::from_value::<Headline>(i.clone()) {
                                if h.kind != "local" || same_places {
                                    items.push(h);
                                }
                            }
                        }
                        if !items.is_empty() {
                            generation = 1;
                        }
                    }
                }
            }
        }

        let status = if generation > 0 {
            format!("{} cached headlines", items.len())
        } else {
            status
        };

        let inner = Arc::new((
            Mutex::new(FeedInner {
                items,
                generation,
                status,
                fetched_at: None,
                wake: false,
            }),
            Condvar::new(),
        ));

        Self {
            inner,
            cfg: Arc::new(Mutex::new(cfg)),
            api_key,
        }
    }

    pub fn start(self: &Arc<Self>) {
        let this = Arc::clone(self);
        thread::Builder::new()
            .name("newsfeed".into())
            .spawn(move || this.run())
            .expect("spawn newsfeed");
    }

    pub fn wake(&self) {
        let (lock, cvar) = &*self.inner;
        let mut g = lock.lock().unwrap();
        g.wake = true;
        cvar.notify_one();
    }

    pub fn snapshot(&self) -> (Vec<Headline>, u64, String, Option<SystemTime>) {
        let g = self.inner.0.lock().unwrap();
        (
            g.items.clone(),
            g.generation,
            g.status.clone(),
            g.fetched_at,
        )
    }

    pub fn update_cfg(&self, cfg: Config) {
        *self.cfg.lock().unwrap() = cfg;
    }

    pub fn api_key(&self) -> Option<String> {
        self.api_key.clone()
    }

    fn run(&self) {
        loop {
            if self.api_key.is_some() {
                self.fetch();
            }
            let minutes = self
                .cfg
                .lock()
                .unwrap()
                .refresh_minutes
                .max(2.0 / 60.0);
            let wait = Duration::from_secs_f64(minutes * 60.0);
            let (lock, cvar) = &*self.inner;
            let mut g = lock.lock().unwrap();
            let start = Instant::now();
            while !g.wake && start.elapsed() < wait {
                let remaining = wait.saturating_sub(start.elapsed());
                let result = cvar.wait_timeout(g, remaining).unwrap();
                g = result.0;
            }
            g.wake = false;
        }
    }

    fn search(
        &self,
        api_key: &str,
        query: &str,
        num: u32,
        category: Option<&str>,
        search_type: &str,
        hours_back: f64,
    ) -> Result<Vec<Value>, String> {
        let since = SystemTime::now() - Duration::from_secs_f64(hours_back * 3600.0);
        let since_secs = since
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        // format as ISO 8601 roughly
        let start_published = format_iso8601(since_secs);

        let mut body = json!({
            "query": query,
            "type": search_type,
            "numResults": num,
            "startPublishedDate": start_published,
        });
        if let Some(cat) = category {
            body["category"] = json!(cat);
        }

        let resp = ureq::post("https://api.exa.ai/search")
            .set("x-api-key", api_key)
            .set("content-type", "application/json")
            .timeout(Duration::from_secs(20))
            .send_json(body)
            .map_err(|e| e.to_string())?;

        let v: Value = resp.into_json().map_err(|e| e.to_string())?;
        Ok(v.get("results")
            .and_then(|r| r.as_array())
            .cloned()
            .unwrap_or_default())
    }

    fn collect(
        &self,
        results: &[Value],
        kind: &str,
        seen: &mut HashSet<String>,
        show_source: bool,
    ) -> Vec<Headline> {
        let mut out = Vec::new();
        for r in results {
            let mut title = clean_title(r.get("title").and_then(|v| v.as_str()).unwrap_or(""));
            let url = r
                .get("url")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let mut domain = String::new();
            if let Some((_, rest)) = url.split_once("://") {
                let host = rest.split('/').next().unwrap_or("");
                domain = host
                    .strip_prefix("www.")
                    .unwrap_or(host)
                    .to_string();
            }
            let root = if !domain.is_empty() {
                domain
                    .split('.')
                    .next()
                    .unwrap_or("")
                    .replace('-', "")
                    .to_lowercase()
            } else {
                String::new()
            };
            if root.len() >= 4 {
                for sep in [" | ", " — ", " – ", " - "] {
                    if let Some((base, suffix)) = title.rsplit_once(sep) {
                        let norm = suffix.replace(' ', "").replace('-', "").to_lowercase();
                        if !base.is_empty()
                            && !norm.is_empty()
                            && (root.contains(&norm) || norm.contains(&root))
                        {
                            title = base.to_string();
                            break;
                        }
                    }
                }
            }
            if title.is_empty() {
                continue;
            }
            let key = title.to_lowercase();
            if seen.contains(&key) {
                continue;
            }
            seen.insert(key);
            let text = if !domain.is_empty() && show_source {
                format!("{title}  ·  {domain}")
            } else {
                title
            };
            out.push(Headline {
                text,
                url,
                domain,
                kind: kind.to_string(),
            });
        }
        out
    }

    fn fetch(&self) {
        let api_key = match &self.api_key {
            Some(k) => k.clone(),
            None => return,
        };
        let cfg = self.cfg.lock().unwrap().clone();
        let mut items = Vec::new();
        let mut seen = HashSet::new();
        let places: Vec<String> = cfg
            .places
            .iter()
            .map(|p| p.trim().to_string())
            .filter(|p| !p.is_empty())
            .take(3)
            .collect();

        for place in &places {
            let query = format!("{place} — local news, events and what is happening there now");
            if let Ok(results) =
                self.search(&api_key, &query, 10, None, "fast", cfg.hours_back)
            {
                items.extend(self.collect(&results, "local", &mut seen, cfg.show_source));
            }
        }

        for topic in cfg.topics.iter().take(4) {
            let query = format!("{topic} — the most significant news right now");
            let results = match self.search(
                &api_key,
                &query,
                25,
                Some("news"),
                "fast",
                cfg.hours_back,
            ) {
                Ok(r) => r,
                Err(_) => {
                    match self.search(
                        &api_key,
                        &query,
                        25,
                        Some("news"),
                        "auto",
                        cfg.hours_back,
                    ) {
                        Ok(r) => r,
                        Err(_) => continue,
                    }
                }
            };
            items.extend(self.collect(&results, "news", &mut seen, cfg.show_source));
        }

        let mut g = self.inner.0.lock().unwrap();
        if !items.is_empty() {
            let n_local = items.iter().filter(|i| i.kind == "local").count();
            g.items = items.clone();
            g.generation += 1;
            g.fetched_at = Some(SystemTime::now());
            g.status = if places.is_empty() {
                format!("{} headlines", items.len())
            } else {
                format!(
                    "{} headlines · {n_local} local ({})",
                    items.len(),
                    places.join(", ")
                )
            };
            // cache
            let path = cache_path();
            if let Some(parent) = path.parent() {
                let _ = fs::create_dir_all(parent);
            }
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs_f64();
            let cache = json!({
                "at": now,
                "places": places,
                "items": items,
            });
            if let Ok(text) = serde_json::to_string(&cache) {
                let _ = fs::write(path, text);
            }
        } else if g.items.is_empty() {
            g.status = "news offline — poetic only".into();
        }
    }
}

fn places_key(places: &[String]) -> Vec<String> {
    let mut v: Vec<String> = places
        .iter()
        .map(|p| p.trim().to_lowercase())
        .filter(|p| !p.is_empty())
        .collect();
    v.sort();
    v
}

pub fn clean_title(raw: &str) -> String {
    let mut t = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    loop {
        let mut changed = false;
        for sep in [" | ", " — ", " – ", " - "] {
            if let Some((base, suffix)) = t.rsplit_once(sep) {
                let words: Vec<_> = suffix.split_whitespace().collect();
                if !base.is_empty()
                    && words.len() <= 4
                    && !suffix.chars().any(|c| c.is_ascii_digit())
                {
                    t = base.to_string();
                    changed = true;
                    break;
                }
            }
        }
        if !changed {
            break;
        }
    }
    t
}

fn format_iso8601(unix_secs: u64) -> String {
    // minimal UTC formatter for Exa startPublishedDate
    let days = (unix_secs / 86400) as i64;
    let day_secs = unix_secs % 86400;
    let h = day_secs / 3600;
    let m = (day_secs % 3600) / 60;
    let s = day_secs % 60;

    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { y + 1 } else { y };

    format!("{year:04}-{month:02}-{d:02}T{h:02}:{m:02}:{s:02}.000Z")
}

/// Fetch a story summary via Exa contents API.
pub fn fetch_summary(
    api_key: &str,
    url: &str,
    fallback_title: &str,
    domain: &str,
) -> (String, String, String, String) {
    // returns (title, domain, summary, url)
    let body = json!({
        "ids": [url],
        "summary": {"query": "What happened, concretely? 2-4 short sentences."},
        "livecrawl": "fallback"
    });
    match ureq::post("https://api.exa.ai/contents")
        .set("x-api-key", api_key)
        .set("content-type", "application/json")
        .timeout(Duration::from_secs(25))
        .send_json(body)
    {
        Ok(resp) => {
            let v: Value = resp.into_json().unwrap_or(json!({}));
            let results = v
                .get("results")
                .and_then(|r| r.as_array())
                .cloned()
                .unwrap_or_default();
            let r = results.first();
            let title = r
                .and_then(|x| x.get("title"))
                .and_then(|t| t.as_str())
                .map(clean_title)
                .filter(|t| !t.is_empty())
                .unwrap_or_else(|| fallback_title.to_string());
            let summary = r
                .and_then(|x| x.get("summary"))
                .and_then(|s| s.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "(no summary could be decoded)".into());
            (title, domain.to_string(), summary, url.to_string())
        }
        Err(_) => (
            fallback_title.to_string(),
            domain.to_string(),
            "(the summary could not be decoded — shift-click the headline to open the story)"
                .into(),
            url.to_string(),
        ),
    }
}
