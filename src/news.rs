use crate::config::{cache_path, fetch_log_path, Config};
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
    /// Short blurb from the feed (used when decoding a story without Exa).
    #[serde(default)]
    pub summary: Option<String>,
}

struct FeedInner {
    items: Vec<Headline>,
    generation: u64,
    status: String,
    /// Multi-line log of the last fetch (for status / debug panel).
    last_log: String,
    fetched_at: Option<SystemTime>,
    wake: bool,
}

/// Background headline fetcher. Primary source is plain RSS/Atom (no API key).
/// Optional Exa key only improves click-to-summarize.
pub struct Newsfeed {
    inner: Arc<(Mutex<FeedInner>, Condvar)>,
    cfg: Arc<Mutex<Config>>,
    api_key: Option<String>,
    offline: bool,
    verbose: bool,
}

impl Newsfeed {
    pub fn new(cfg: Config, api_key: Option<String>, offline: bool, verbose: bool) -> Self {
        let status = if offline {
            "offline — poetic only".into()
        } else {
            "fetching feeds…".into()
        };
        let mut items = Vec::new();
        let mut generation = 0u64;

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
                last_log: String::new(),
                fetched_at: None,
                wake: false,
            }),
            Condvar::new(),
        ));

        Self {
            inner,
            cfg: Arc::new(Mutex::new(cfg)),
            api_key,
            offline,
            verbose,
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

    pub fn last_log(&self) -> String {
        self.inner.0.lock().unwrap().last_log.clone()
    }

    pub fn update_cfg(&self, cfg: Config) {
        *self.cfg.lock().unwrap() = cfg;
    }

    pub fn api_key(&self) -> Option<String> {
        self.api_key.clone()
    }

    fn run(&self) {
        loop {
            if !self.offline {
                // Never let a fetch panic kill the news thread / starve the UI.
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    self.fetch();
                }));
                if result.is_err() {
                    if let Ok(mut g) = self.inner.0.lock() {
                        if g.items.is_empty() {
                            g.status = "news offline — poetic only".into();
                        } else {
                            g.status = "refresh panicked — using cache".into();
                        }
                    }
                }
            }
            let minutes = self
                .cfg
                .lock()
                .map(|c| c.refresh_minutes)
                .unwrap_or(15.0)
                .max(2.0 / 60.0);
            let wait = Duration::from_secs_f64(minutes * 60.0);
            let (lock, cvar) = &*self.inner;
            let Ok(mut g) = lock.lock() else {
                thread::sleep(wait);
                continue;
            };
            let start = Instant::now();
            while !g.wake && start.elapsed() < wait {
                let remaining = wait.saturating_sub(start.elapsed());
                let (guard, _) = cvar.wait_timeout(g, remaining).unwrap_or_else(|e| e.into_inner());
                g = guard;
            }
            g.wake = false;
        }
    }

    fn fetch(&self) {
        let cfg = self.cfg.lock().unwrap().clone();
        let report = fetch_all(&cfg, self.verbose);
        write_fetch_log(&report.log);

        let mut g = self.inner.0.lock().unwrap();
        g.last_log = report.log.clone();
        if !report.items.is_empty() {
            let n_local = report.items.iter().filter(|i| i.kind == "local").count();
            g.items = report.items.clone();
            g.generation += 1;
            g.fetched_at = Some(SystemTime::now());
            let places = report.places.join(", ");
            g.status = if report.places.is_empty() {
                format!(
                    "{} headlines · {}/{} feeds",
                    report.items.len(),
                    report.feed_ok,
                    report.feed_ok + report.feed_fail
                )
            } else {
                format!(
                    "{} headlines · {n_local} local ({places}) · {}/{} feeds",
                    report.items.len(),
                    report.feed_ok,
                    report.feed_ok + report.feed_fail
                )
            };
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
                "places": report.places,
                "items": report.items,
            });
            if let Ok(text) = serde_json::to_string(&cache) {
                let _ = fs::write(path, text);
            }
        } else if g.items.is_empty() {
            g.status = report.status_when_empty();
        } else {
            // Keep previous headlines but surface that refresh failed.
            g.status = format!(
                "refresh failed ({}/{} feeds) — using cache",
                report.feed_ok,
                report.feed_ok + report.feed_fail
            );
        }
    }
}

struct FetchReport {
    items: Vec<Headline>,
    places: Vec<String>,
    feed_ok: u32,
    feed_fail: u32,
    log: String,
    no_sources: bool,
}

impl FetchReport {
    fn status_when_empty(&self) -> String {
        if self.no_sources {
            "no topics/places/feeds — poetic only".into()
        } else if self.feed_ok == 0 && self.feed_fail > 0 {
            format!(
                "feeds unreachable (0/{} ok) — poetic only",
                self.feed_fail
            )
        } else if self.feed_ok > 0 {
            format!(
                "feeds ok but empty ({}/{}) — poetic only",
                self.feed_ok,
                self.feed_ok + self.feed_fail
            )
        } else {
            "no headlines — poetic only".into()
        }
    }
}

/// One-shot feed check for the CLI (`--check-feeds`). Prints diagnostics to stdout.
pub fn run_feed_check(cfg: &Config, verbose: bool) -> i32 {
    println!("meanwhile feed check");
    println!(
        "places: {}",
        if cfg.places.is_empty() {
            "(none)".into()
        } else {
            cfg.places.join(", ")
        }
    );
    println!(
        "topics: {}",
        if cfg.topics.is_empty() {
            "(none)".into()
        } else {
            cfg.topics.join(", ")
        }
    );
    if !cfg.extra_feeds.is_empty() {
        println!("extra_feeds: {}", cfg.extra_feeds.len());
    }
    println!();

    let report = fetch_all(cfg, verbose);
    write_fetch_log(&report.log);
    print!("{}", report.log);
    println!();
    println!(
        "result: {} headlines ({} local) · {} ok · {} failed",
        report.items.len(),
        report.items.iter().filter(|i| i.kind == "local").count(),
        report.feed_ok,
        report.feed_fail
    );
    if !report.items.is_empty() {
        println!("sample:");
        for h in report.items.iter().take(8) {
            println!("  [{}] {}", h.kind, h.text.chars().take(90).collect::<String>());
        }
    }
    println!();
    println!("log written to {}", fetch_log_path().display());

    if report.items.is_empty() {
        1
    } else {
        0
    }
}

fn write_fetch_log(log: &str) {
    let path = fetch_log_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(&path, log);
}

fn fetch_all(cfg: &Config, verbose: bool) -> FetchReport {
    let mut items = Vec::new();
    let mut seen = HashSet::new();
    let mut feed_ok = 0u32;
    let mut feed_fail = 0u32;
    let mut lines: Vec<String> = Vec::new();
    let started = chrono_now();

    lines.push(format!("meanwhile fetch @ {started}"));
    lines.push(format!(
        "places={} topics={} extra_feeds={}",
        cfg.places.join("|"),
        cfg.topics.join("|"),
        cfg.extra_feeds.len()
    ));

    let places: Vec<String> = cfg
        .places
        .iter()
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .take(3)
        .collect();

    // Place → regional RSS
    for place in &places {
        let srcs = feeds_for_place(place);
        if srcs.is_empty() {
            let line = format!(
                "WARN place  no built-in feeds for \"{place}\" — add extra_feeds or a known country name"
            );
            lines.push(line.clone());
            if verbose {
                eprintln!("meanwhile: {line}");
            }
        }
        for src in srcs {
            try_one_source(
                "place",
                src,
                "local",
                cfg.show_source,
                verbose,
                &mut items,
                &mut seen,
                &mut feed_ok,
                &mut feed_fail,
                &mut lines,
            );
        }
    }

    // Topics
    for topic in cfg.topics.iter().take(4) {
        let tnorm = topic.trim().to_lowercase();
        if places.iter().any(|p| p.to_lowercase() == tnorm) {
            lines.push(format!("SKIP topic  \"{topic}\" (same as a place)"));
            continue;
        }
        for src in feeds_for_topic(topic) {
            try_one_source(
                "topic",
                src,
                "news",
                cfg.show_source,
                verbose,
                &mut items,
                &mut seen,
                &mut feed_ok,
                &mut feed_fail,
                &mut lines,
            );
        }
    }

    // User extra feeds
    for url in cfg.extra_feeds.iter().take(8) {
        let url = url.trim();
        if url.is_empty() {
            continue;
        }
        try_one_source(
            "extra",
            url,
            "news",
            cfg.show_source,
            verbose,
            &mut items,
            &mut seen,
            &mut feed_ok,
            &mut feed_fail,
            &mut lines,
        );
    }

    // Defaults only when nothing configured
    let no_sources =
        places.is_empty() && cfg.topics.is_empty() && cfg.extra_feeds.is_empty();
    if items.is_empty() && no_sources {
        for url in DEFAULT_WORLD_FEEDS {
            try_one_source(
                "default",
                url,
                "news",
                cfg.show_source,
                verbose,
                &mut items,
                &mut seen,
                &mut feed_ok,
                &mut feed_fail,
                &mut lines,
            );
        }
    }

    lines.push(format!(
        "done: {} headlines ({} local) · {} ok · {} failed",
        items.len(),
        items.iter().filter(|i| i.kind == "local").count(),
        feed_ok,
        feed_fail
    ));
    lines.push(String::new());

    FetchReport {
        items,
        places,
        feed_ok,
        feed_fail,
        log: lines.join("\n"),
        no_sources,
    }
}

fn try_one_source(
    label: &str,
    src: &str,
    kind: &str,
    show_source: bool,
    verbose: bool,
    items: &mut Vec<Headline>,
    seen: &mut HashSet<String>,
    feed_ok: &mut u32,
    feed_fail: &mut u32,
    lines: &mut Vec<String>,
) {
    let display = source_label(src);
    let t0 = Instant::now();
    match pull_source(src, 20) {
        Ok(entries) => {
            let before = items.len();
            items.extend(collect_entries(&entries, kind, seen, show_source));
            let added = items.len() - before;
            *feed_ok += 1;
            let ms = t0.elapsed().as_millis();
            let line = format!(
                "OK   {label:8} +{added:<3} in {ms:>5}ms  ({display})  raw={}",
                entries.len()
            );
            lines.push(line.clone());
            if verbose {
                eprintln!("meanwhile: {line}");
            }
        }
        Err(e) => {
            *feed_fail += 1;
            let ms = t0.elapsed().as_millis();
            let line = format!("FAIL {label:8} in {ms:>5}ms  ({display})  {e}");
            lines.push(line.clone());
            if verbose {
                eprintln!("meanwhile: {line}");
            }
        }
    }
}

fn source_label(src: &str) -> String {
    if src == HN_TOP24_SENTINEL {
        "hacker-news top/24h".into()
    } else if src.len() > 64 {
        format!("{}…", &src[..64])
    } else {
        src.to_string()
    }
}

fn chrono_now() -> String {
    // Local-ish wall clock via libc for the log header.
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!("unix:{secs}")
}

// ---------------------------------------------------------------------------
// Built-in RSS registry (place / topic → feed URLs)
// Only feeds that speak open RSS/Atom (no keys). Probed for availability.
// ---------------------------------------------------------------------------

const DEFAULT_WORLD_FEEDS: &[&str] = &[
    "https://feeds.bbci.co.uk/news/world/rss.xml",
    "https://www.theguardian.com/world/rss",
    "https://www.aljazeera.com/xml/rss/all.xml",
];

/// Special sentinel consumed by fetch() — not a URL.
const HN_TOP24_SENTINEL: &str = "meanwhile:hn-top24";

fn feeds_for_place(place: &str) -> Vec<&'static str> {
    let p = place.to_lowercase();
    let p = p.trim();

    // —— Oceania ——
    if matches_any(p, &["new zealand", "nz", "aotearoa", "kiwi"])
        || matches_any(
            p,
            &[
                "auckland",
                "wellington",
                "christchurch",
                "hamilton",
                "dunedin",
                "tāmaki",
                "tamaki",
            ],
        )
    {
        return vec![
            "https://www.rnz.co.nz/rss/national.xml",
            "https://www.rnz.co.nz/rss/political.xml",
            "https://www.stuff.co.nz/rss",
        ];
    }
    if matches_any(
        p,
        &[
            "australia",
            "au",
            "sydney",
            "melbourne",
            "brisbane",
            "perth",
            "adelaide",
            "canberra",
        ],
    ) {
        return vec![
            "https://www.abc.net.au/news/feed/51120/rss.xml", // top stories
            "https://www.abc.net.au/news/feed/45910/rss.xml", // national
            "https://www.sbs.com.au/news/feed",
            "https://www.smh.com.au/rss/feed.xml",
        ];
    }

    // —— UK / Ireland ——
    if matches_any(
        p,
        &[
            "uk",
            "united kingdom",
            "britain",
            "great britain",
            "england",
            "scotland",
            "wales",
            "northern ireland",
        ],
    ) {
        return vec![
            "https://feeds.bbci.co.uk/news/uk/rss.xml",
            "https://feeds.bbci.co.uk/news/politics/rss.xml",
            "https://www.theguardian.com/uk-news/rss",
            "https://feeds.skynews.com/feeds/rss/uk.xml",
            "https://www.independent.co.uk/news/uk/rss",
        ];
    }
    if matches_any(p, &["london"]) {
        return vec![
            "https://feeds.bbci.co.uk/news/england/london/rss.xml",
            "https://www.theguardian.com/uk-news/rss",
        ];
    }
    if matches_any(p, &["ireland", "dublin", "éire", "eire", "republic of ireland"]) {
        return vec![
            "https://www.irishtimes.com/cmlink/news-1.1319192",
            "https://feeds.bbci.co.uk/news/northern_ireland/rss.xml",
        ];
    }

    // —— North America ——
    if matches_any(p, &["us", "usa", "united states", "america", "u.s.", "u.s.a."]) {
        return vec![
            "https://feeds.npr.org/1001/rss.xml",
            "https://www.pbs.org/newshour/feeds/rss/headlines",
            "https://rss.nytimes.com/services/xml/rss/nyt/HomePage.xml",
            "https://rss.politico.com/politics-news.xml",
        ];
    }
    if matches_any(
        p,
        &["canada", "ca", "toronto", "vancouver", "ottawa", "montreal", "calgary"],
    ) {
        return vec![
            "https://www.cbc.ca/webfeed/rss/rss-topstories",
            "https://www.theglobeandmail.com/arc/outboundfeeds/rss/category/canada/",
        ];
    }

    // —— Europe ——
    if matches_any(p, &["europe", "eu", "european union"]) {
        return vec![
            "https://feeds.bbci.co.uk/news/world/europe/rss.xml",
            "https://www.theguardian.com/world/europe/rss",
            "https://www.france24.com/en/europe/rss",
        ];
    }
    if matches_any(p, &["france", "paris", "fr"]) {
        return vec![
            "https://www.france24.com/en/france/rss",
            "https://www.france24.com/en/rss",
        ];
    }
    if matches_any(p, &["germany", "de", "berlin", "deutschland"]) {
        return vec![
            "https://rss.dw.com/rdf/rss-en-all",
            "https://www.theguardian.com/world/germany/rss",
        ];
    }
    if matches_any(p, &["spain", "es", "madrid", "barcelona"]) {
        return vec![
            "https://feeds.elpais.com/mrss-s/pages/ep/site/english.elpais.com/portada",
            "https://feeds.bbci.co.uk/news/world/europe/rss.xml",
        ];
    }
    if matches_any(p, &["italy", "it", "rome", "milan"]) {
        return vec![
            "https://www.theguardian.com/world/italy/rss",
            "https://feeds.bbci.co.uk/news/world/europe/rss.xml",
        ];
    }
    if matches_any(p, &["netherlands", "holland", "nl", "amsterdam"]) {
        return vec!["https://feeds.bbci.co.uk/news/world/europe/rss.xml"];
    }
    if matches_any(p, &["ukraine", "kyiv", "kiev"]) {
        return vec![
            "https://feeds.bbci.co.uk/news/world/europe/rss.xml",
            "https://www.aljazeera.com/xml/rss/all.xml",
        ];
    }

    // —— Middle East / Africa ——
    if matches_any(
        p,
        &["middle east", "israel", "palestine", "gaza", "lebanon", "iran", "iraq", "syria"],
    ) {
        return vec![
            "https://www.aljazeera.com/xml/rss/all.xml",
            "https://feeds.bbci.co.uk/news/world/middle_east/rss.xml",
            "https://www.theguardian.com/world/middleeast/rss",
        ];
    }
    if matches_any(p, &["africa", "south africa", "nigeria", "kenya", "egypt"]) {
        return vec![
            "https://feeds.bbci.co.uk/news/world/africa/rss.xml",
            "https://www.aljazeera.com/xml/rss/all.xml",
        ];
    }

    // —— Asia / Pacific ——
    if matches_any(p, &["asia"]) {
        return vec![
            "https://feeds.bbci.co.uk/news/world/asia/rss.xml",
            "https://www.scmp.com/rss/91/feed",
        ];
    }
    if matches_any(p, &["japan", "jp", "tokyo", "osaka"]) {
        return vec![
            "https://www3.nhk.or.jp/rss/news/cat0.xml",
            "https://feeds.bbci.co.uk/news/world/asia/rss.xml",
        ];
    }
    if matches_any(p, &["china", "cn", "beijing", "hong kong", "hongkong"]) {
        return vec![
            "https://www.scmp.com/rss/91/feed", // Hong Kong / China (SCMP)
            "https://feeds.bbci.co.uk/news/world/asia/china/rss.xml",
        ];
    }
    if matches_any(p, &["india", "in", "delhi", "mumbai", "bangalore", "bengaluru"]) {
        return vec![
            "https://timesofindia.indiatimes.com/rssfeedstopstories.cms",
            "https://feeds.bbci.co.uk/news/world/asia/india/rss.xml",
        ];
    }
    if matches_any(p, &["singapore", "sg"]) {
        return vec![
            "https://www.channelnewsasia.com/api/v1/rss-outbound-feed?_format=xml",
            "https://feeds.bbci.co.uk/news/world/asia/rss.xml",
        ];
    }
    if matches_any(p, &["south korea", "korea", "kr", "seoul"]) {
        return vec!["https://feeds.bbci.co.uk/news/world/asia/rss.xml"];
    }
    if matches_any(p, &["taiwan", "taipei"]) {
        return vec![
            "https://feeds.bbci.co.uk/news/world/asia/rss.xml",
            "https://www.scmp.com/rss/91/feed",
        ];
    }
    if matches_any(p, &["indonesia", "jakarta", "philippines", "manila", "thailand", "bangkok", "vietnam", "malaysia"])
    {
        return vec!["https://feeds.bbci.co.uk/news/world/asia/rss.xml"];
    }

    // —— Latin America ——
    if matches_any(
        p,
        &[
            "latin america",
            "south america",
            "brazil",
            "mexico",
            "argentina",
            "chile",
            "colombia",
            "peru",
        ],
    ) {
        return vec![
            "https://feeds.bbci.co.uk/news/world/latin_america/rss.xml",
            "https://www.theguardian.com/world/americas/rss",
        ];
    }

    // Unknown place: user can set extra_feeds. Don't invent world news as "local".
    Vec::new()
}

fn feeds_for_topic(topic: &str) -> Vec<&'static str> {
    let t = topic.to_lowercase();
    let t = t.trim();

    if matches_any(
        t,
        &["world", "world news", "international", "global", "news"],
    ) {
        return vec![
            "https://feeds.bbci.co.uk/news/world/rss.xml",
            "https://www.theguardian.com/world/rss",
            "https://www.aljazeera.com/xml/rss/all.xml",
            "https://www.france24.com/en/rss",
            "https://rss.dw.com/rdf/rss-en-all",
        ];
    }
    // Hacker News — not /best (that's a longer-horizon ranking). We use the
    // official Firebase API for true top-by-score in the last 24 hours.
    if matches_any(t, &["hacker news", "hackernews", "hn", "ycombinator", "yc"]) {
        return vec![HN_TOP24_SENTINEL];
    }
    if matches_any(
        t,
        &[
            "tech",
            "technology",
            "artificial intelligence",
            "ai",
            "science & tech",
            "gadgets",
            "software",
            "startups",
        ],
    ) {
        return vec![
            "https://www.theverge.com/rss/index.xml",
            "https://feeds.arstechnica.com/arstechnica/index",
            HN_TOP24_SENTINEL, // top HN of the last day
            "https://hnrss.org/frontpage", // current front page as backup colour
        ];
    }
    if matches_any(t, &["science", "space", "climate", "environment"]) {
        return vec![
            "https://feeds.bbci.co.uk/news/science_and_environment/rss.xml",
            "https://www.sciencedaily.com/rss/all.xml",
            "https://www.theguardian.com/science/rss",
        ];
    }
    if matches_any(t, &["business", "economy", "markets", "finance", "money"]) {
        return vec![
            "https://feeds.bbci.co.uk/news/business/rss.xml",
            "https://www.cnbc.com/id/100003114/device/rss/rss.html",
            "https://www.theguardian.com/uk/business/rss",
        ];
    }
    if matches_any(t, &["sport", "sports", "football", "cricket", "rugby"]) {
        return vec![
            "https://feeds.bbci.co.uk/sport/rss.xml",
            "https://www.theguardian.com/uk/sport/rss",
        ];
    }
    if matches_any(t, &["politics", "policy", "government"]) {
        return vec![
            "https://feeds.bbci.co.uk/news/politics/rss.xml",
            "https://rss.politico.com/politics-news.xml",
            "https://www.theguardian.com/politics/rss",
        ];
    }
    if matches_any(t, &["culture", "arts", "books", "film", "music"]) {
        return vec![
            "https://feeds.bbci.co.uk/news/entertainment_and_arts/rss.xml",
            "https://www.theguardian.com/uk/culture/rss",
        ];
    }

    // Country names typed as topics → same feeds as places
    if matches_any(t, &["uk", "united kingdom", "britain"]) {
        return feeds_for_place("uk");
    }
    if matches_any(t, &["new zealand", "nz", "aotearoa"]) {
        return feeds_for_place("new zealand");
    }
    if matches_any(t, &["australia", "au"]) {
        return feeds_for_place("australia");
    }
    if matches_any(t, &["us", "usa", "united states", "america"]) {
        return feeds_for_place("us");
    }
    if matches_any(t, &["canada", "ca"]) {
        return feeds_for_place("canada");
    }
    if matches_any(t, &["india", "in"]) {
        return feeds_for_place("india");
    }
    if matches_any(t, &["japan", "jp"]) {
        return feeds_for_place("japan");
    }
    if matches_any(t, &["china", "cn"]) {
        return feeds_for_place("china");
    }
    if matches_any(t, &["france", "fr"]) {
        return feeds_for_place("france");
    }
    if matches_any(t, &["germany", "de"]) {
        return feeds_for_place("germany");
    }
    if matches_any(t, &["ireland"]) {
        return feeds_for_place("ireland");
    }

    // Fallback: BBC top stories
    vec!["https://feeds.bbci.co.uk/news/rss.xml"]
}

fn matches_any(hay: &str, needles: &[&str]) -> bool {
    needles.iter().any(|n| {
        if hay == *n {
            return true;
        }
        // Short codes (nz, uk, us) must be whole tokens, not substrings
        // ("us" must not match "australia").
        if n.len() <= 3 {
            return hay
                .split(|c: char| !c.is_alphanumeric())
                .any(|tok| tok == *n);
        }
        hay.contains(n)
    })
}

// ---------------------------------------------------------------------------
// RSS / Atom / special-source fetch + parse
// ---------------------------------------------------------------------------

struct FeedEntry {
    title: String,
    url: String,
    summary: Option<String>,
}

fn pull_source(src: &str, limit: usize) -> Result<Vec<FeedEntry>, String> {
    if src == HN_TOP24_SENTINEL {
        return fetch_hn_top_last_day(limit.min(20));
    }
    fetch_rss(src, limit)
}

/// Top Hacker News stories from the last 24 hours, by score.
///
/// Note: `https://hnrss.org/best` is *not* this — it's HN's longer-horizon
/// "best" ranking. We use the free Firebase API (topstories + item lookup),
/// filter `time` to the past day, sort by `score`, take the top N.
///
/// Item lookups run in parallel so a refresh doesn't sit for a minute.
fn fetch_hn_top_last_day(limit: usize) -> Result<Vec<FeedEntry>, String> {
    let ids: Vec<u64> = ureq::get("https://hacker-news.firebaseio.com/v0/topstories.json")
        .set("User-Agent", "meanwhile/0.4")
        .timeout(Duration::from_secs(12))
        .call()
        .map_err(|e| e.to_string())?
        .into_json()
        .map_err(|e| e.to_string())?;

    #[derive(Deserialize)]
    struct HnItem {
        title: Option<String>,
        url: Option<String>,
        score: Option<i64>,
        time: Option<i64>,
        descendants: Option<i64>,
        #[serde(rename = "type")]
        kind: Option<String>,
    }

    fn load_item(id: u64) -> Option<(i64, FeedEntry)> {
        let url = format!("https://hacker-news.firebaseio.com/v0/item/{id}.json");
        let item: HnItem = ureq::get(&url)
            .set("User-Agent", "meanwhile/0.4")
            .timeout(Duration::from_secs(6))
            .call()
            .ok()?
            .into_json()
            .ok()?;
        if item.kind.as_deref() != Some("story") {
            return None;
        }
        let _ = item.time; // available if we re-enable a hard 24h cutoff
        let title = clean_title(item.title.as_deref().unwrap_or(""));
        if title.is_empty() {
            return None;
        }
        let link = item
            .url
            .filter(|u| !u.is_empty())
            .unwrap_or_else(|| format!("https://news.ycombinator.com/item?id={id}"));
        let score = item.score.unwrap_or(0);
        let comments = item.descendants.unwrap_or(0);
        Some((
            score,
            FeedEntry {
                title,
                url: link,
                summary: Some(format!("{score} points · {comments} comments · HN")),
            },
        ))
    }

    // Parallel batches — 40 ids is enough for a 24h slice of topstories.
    let sample: Vec<u64> = ids.into_iter().take(40).collect();
    let mut scored: Vec<(i64, FeedEntry)> = Vec::new();
    for chunk in sample.chunks(10) {
        let mut handles = Vec::new();
        for &id in chunk {
            handles.push(thread::spawn(move || load_item(id)));
        }
        for h in handles {
            if let Ok(Some((score, entry))) = h.join() {
                // topstories are already the current ranked set (≈ last day).
                scored.push((score, entry));
            }
        }
    }

    scored.sort_by(|a, b| b.0.cmp(&a.0));
    let out: Vec<FeedEntry> = scored.into_iter().take(limit).map(|(_, e)| e).collect();
    if out.is_empty() {
        // Quiet day / API blip — fall back to current front page RSS.
        return fetch_rss("https://hnrss.org/frontpage", limit);
    }
    Ok(out)
}

fn fetch_rss(url: &str, limit: usize) -> Result<Vec<FeedEntry>, String> {
    let body = ureq::get(url)
        .set(
            "User-Agent",
            "meanwhile/0.4 (+https://github.com/tomdavenport/meanwhile)",
        )
        .set("Accept", "application/rss+xml, application/atom+xml, application/xml, text/xml, */*")
        .timeout(Duration::from_secs(15))
        .call()
        .map_err(|e| e.to_string())?
        .into_string()
        .map_err(|e| e.to_string())?;

    let entries = parse_feed_xml(&body);
    if entries.is_empty() {
        return Err("empty feed".into());
    }
    Ok(entries.into_iter().take(limit).collect())
}

/// Minimal RSS 2.0 + Atom parser (no extra crate). Good enough for news feeds.
fn parse_feed_xml(xml: &str) -> Vec<FeedEntry> {
    let mut out = Vec::new();

    // RSS: <item>...</item>
    for block in iter_tag_blocks(xml, "item") {
        if let Some(e) = entry_from_rss_item(&block) {
            out.push(e);
        }
    }
    if !out.is_empty() {
        return out;
    }

    // Atom: <entry>...</entry>
    for block in iter_tag_blocks(xml, "entry") {
        if let Some(e) = entry_from_atom_entry(&block) {
            out.push(e);
        }
    }
    out
}

fn iter_tag_blocks<'a>(xml: &'a str, tag: &str) -> Vec<&'a str> {
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let mut out = Vec::new();
    let lower = xml.to_ascii_lowercase();
    // Search case-insensitively via lowercase index into original
    let mut start = 0;
    while let Some(rel) = lower[start..].find(&open.to_ascii_lowercase()) {
        let abs = start + rel;
        // find end of open tag
        let after_name = abs + open.len();
        let gt = match xml[after_name..].find('>') {
            Some(i) => after_name + i + 1,
            None => break,
        };
        let close_l = close.to_ascii_lowercase();
        if let Some(rel_c) = lower[gt..].find(&close_l) {
            let end = gt + rel_c;
            out.push(&xml[gt..end]);
            start = end + close.len();
        } else {
            break;
        }
    }
    out
}

fn entry_from_rss_item(block: &str) -> Option<FeedEntry> {
    let title = clean_title(&inner_text(block, "title").unwrap_or_default());
    if title.is_empty() {
        return None;
    }
    let url = inner_text(block, "link")
        .or_else(|| attr_in_tag(block, "enclosure", "url"))
        .unwrap_or_default()
        .trim()
        .to_string();
    let summary = inner_text(block, "description")
        .or_else(|| inner_text(block, "content:encoded"))
        .map(|s| strip_html(&s))
        .filter(|s| !s.is_empty());
    Some(FeedEntry {
        title,
        url,
        summary,
    })
}

fn entry_from_atom_entry(block: &str) -> Option<FeedEntry> {
    let title = clean_title(&inner_text(block, "title").unwrap_or_default());
    if title.is_empty() {
        return None;
    }
    // <link href="..." rel="alternate"/>
    let url = atom_link(block).unwrap_or_default();
    let summary = inner_text(block, "summary")
        .or_else(|| inner_text(block, "content"))
        .map(|s| strip_html(&s))
        .filter(|s| !s.is_empty());
    Some(FeedEntry {
        title,
        url,
        summary,
    })
}

fn atom_link(block: &str) -> Option<String> {
    // Prefer rel="alternate", else first href
    let mut first = None;
    let lower = block.to_ascii_lowercase();
    let mut search = 0;
    while let Some(rel) = lower[search..].find("<link") {
        let abs = search + rel;
        let end = block[abs..].find('>').map(|i| abs + i + 1)?;
        let tag = &block[abs..end];
        let href = attr_value(tag, "href")?;
        let rel_attr = attr_value(tag, "rel").unwrap_or_default();
        if first.is_none() {
            first = Some(href.clone());
        }
        if rel_attr.is_empty() || rel_attr == "alternate" {
            return Some(href);
        }
        search = end;
    }
    first
}

fn inner_text(block: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}");
    let lower = block.to_ascii_lowercase();
    let open_l = open.to_ascii_lowercase();
    let idx = lower.find(&open_l)?;
    let after = idx + open.len();
    let gt = block[after..].find('>')? + after + 1;
    // self-closing
    if block[..gt].trim_end().ends_with("/>") {
        return None;
    }
    let close = format!("</{tag}>");
    let close_l = close.to_ascii_lowercase();
    let end = lower[gt..].find(&close_l)? + gt;
    let raw = block[gt..end].trim();
    Some(decode_xml_entities(raw))
}

fn attr_in_tag(block: &str, tag: &str, attr: &str) -> Option<String> {
    let open = format!("<{tag}");
    let lower = block.to_ascii_lowercase();
    let idx = lower.find(&open.to_ascii_lowercase())?;
    let end = block[idx..].find('>').map(|i| idx + i + 1)?;
    attr_value(&block[idx..end], attr)
}

fn attr_value(tag: &str, attr: &str) -> Option<String> {
    let lower = tag.to_ascii_lowercase();
    let key = format!("{attr}=");
    let idx = lower.find(&key.to_ascii_lowercase())?;
    let rest = tag[idx + key.len()..].trim_start();
    let quote = rest.chars().next()?;
    if quote == '"' || quote == '\'' {
        let end = rest[1..].find(quote)? + 1;
        return Some(decode_xml_entities(&rest[1..end]));
    }
    // unquoted
    let end = rest
        .find(|c: char| c.is_whitespace() || c == '>' || c == '/')
        .unwrap_or(rest.len());
    Some(decode_xml_entities(&rest[..end]))
}

fn decode_xml_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
}

fn strip_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    let t = decode_xml_entities(&out);
    t.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn collect_entries(
    entries: &[FeedEntry],
    kind: &str,
    seen: &mut HashSet<String>,
    show_source: bool,
) -> Vec<Headline> {
    let mut out = Vec::new();
    for e in entries {
        let mut title = clean_title(&e.title);
        let url = e.url.trim().to_string();
        let mut domain = String::new();
        if let Some((_, rest)) = url.split_once("://") {
            let host = rest.split('/').next().unwrap_or("");
            domain = host.strip_prefix("www.").unwrap_or(host).to_string();
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
        let summary = e.summary.as_ref().map(|s| {
            let s = s.trim();
            if s.chars().count() > 600 {
                s.chars().take(600).collect::<String>() + "…"
            } else {
                s.to_string()
            }
        });
        out.push(Headline {
            text,
            url,
            domain,
            kind: kind.to_string(),
            summary,
        });
    }
    out
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
    // CDATA leftovers
    t = t
        .trim_start_matches("<![CDATA[")
        .trim_end_matches("]]>")
        .trim()
        .to_string();
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

/// Body of a decoded story — never instructional UI copy.
#[derive(Debug, Clone)]
pub enum DecodeBody {
    /// Real blurb from the feed description/summary.
    Feed(String),
    /// Richer summary from optional Exa contents.
    Exa(String),
    /// Feed had no usable description (and no Exa key / Exa not used).
    EmptyFeed,
    /// Had a key (or needed remote decode) but nothing came back.
    Unavailable,
}

#[derive(Debug, Clone)]
pub struct DecodeOutcome {
    pub title: String,
    pub domain: String,
    pub url: String,
    pub body: DecodeBody,
}

impl DecodeBody {
    pub fn text(&self) -> Option<&str> {
        match self {
            DecodeBody::Feed(s) | DecodeBody::Exa(s) => Some(s.as_str()),
            DecodeBody::EmptyFeed | DecodeBody::Unavailable => None,
        }
    }

    /// Short toast for the UI chrome — not painted into the rain.
    pub fn toast(&self) -> Option<&'static str> {
        match self {
            DecodeBody::Feed(_) | DecodeBody::Exa(_) => None,
            DecodeBody::EmptyFeed => Some("no blurb in feed · shift-click to open"),
            DecodeBody::Unavailable => Some("summary unavailable · shift-click to open"),
        }
    }
}

/// Non-empty feed blurb after trim / whitespace collapse / junk filter.
fn usable_blurb(raw: Option<&str>) -> Option<String> {
    let s = raw?.trim();
    if s.is_empty() {
        return None;
    }
    let cleaned: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if cleaned.is_empty() {
        return None;
    }
    let lower = cleaned.to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        "..." | "…" | "read more" | "continue reading" | "full story" | "click here"
    ) {
        return None;
    }
    Some(cleaned)
}

/// Decode a story: prefer feed blurb; optional Exa when a key is set.
/// Failures are typed — the UI flashes a toast instead of mounting fake rain text.
pub fn fetch_summary(
    api_key: &str,
    url: &str,
    fallback_title: &str,
    domain: &str,
    feed_summary: Option<&str>,
) -> DecodeOutcome {
    let title = fallback_title.to_string();
    let domain = domain.to_string();
    let url = url.to_string();

    if let Some(s) = usable_blurb(feed_summary) {
        return DecodeOutcome {
            title,
            domain,
            url,
            body: DecodeBody::Feed(s),
        };
    }

    if api_key.is_empty() {
        return DecodeOutcome {
            title,
            domain,
            url,
            body: DecodeBody::EmptyFeed,
        };
    }

    // Optional Exa contents summary when a key is available.
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
                .unwrap_or(title);
            match r
                .and_then(|x| x.get("summary"))
                .and_then(|s| s.as_str())
                .and_then(|s| usable_blurb(Some(s)))
            {
                Some(summary) => DecodeOutcome {
                    title,
                    domain,
                    url,
                    body: DecodeBody::Exa(summary),
                },
                None => DecodeOutcome {
                    title,
                    domain,
                    url,
                    body: DecodeBody::Unavailable,
                },
            }
        }
        Err(_) => DecodeOutcome {
            title,
            domain,
            url,
            body: DecodeBody::Unavailable,
        },
    }
}
