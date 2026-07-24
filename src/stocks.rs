//! Classic stock-ticker quotes (no rain). Yahoo spark batches — no API key.
//! Default universe: all current S&P 500 constituents (`src/data/sp500.txt`).

use serde_json::Value;
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct Quote {
    pub symbol: String,
    pub price: f64,
    pub change: f64,
    pub change_pct: f64,
}

impl Quote {
    pub fn direction(&self) -> i8 {
        if self.change > 0.005 {
            1
        } else if self.change < -0.005 {
            -1
        } else {
            0
        }
    }
}

struct Inner {
    quotes: Vec<Quote>,
    status: String,
    wake: bool,
    generation: u64,
}

pub struct StockFeed {
    inner: Arc<(Mutex<Inner>, Condvar)>,
    symbols: Arc<Mutex<Vec<String>>>,
}

impl StockFeed {
    pub fn new(symbols: Vec<String>) -> Self {
        let symbols = expand_symbols(symbols);
        Self {
            inner: Arc::new((
                Mutex::new(Inner {
                    quotes: Vec::new(),
                    status: format!("fetching {} symbols…", symbols.len()),
                    wake: false,
                    generation: 0,
                }),
                Condvar::new(),
            )),
            symbols: Arc::new(Mutex::new(symbols)),
        }
    }

    pub fn start(self: &Arc<Self>) {
        let this = Arc::clone(self);
        thread::Builder::new()
            .name("stockfeed".into())
            .spawn(move || this.run())
            .expect("spawn stockfeed");
    }

    pub fn wake(&self) {
        let (lock, cvar) = &*self.inner;
        let mut g = lock.lock().unwrap();
        g.wake = true;
        cvar.notify_one();
    }

    pub fn set_symbols(&self, symbols: Vec<String>) {
        let symbols = expand_symbols(symbols);
        *self.symbols.lock().unwrap() = symbols;
        self.wake();
    }

    pub fn snapshot(&self) -> (Vec<Quote>, String, u64) {
        let g = self.inner.0.lock().unwrap();
        (g.quotes.clone(), g.status.clone(), g.generation)
    }

    fn run(&self) {
        loop {
            self.fetch();
            // Full S&P refresh is network-heavy; 10 minutes is the default cadence.
            let wait = Duration::from_secs(10 * 60);
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

    fn fetch(&self) {
        let symbols = self.symbols.lock().unwrap().clone();
        let n = symbols.len();
        let mut quotes = match fetch_yahoo_spark_batch(&symbols) {
            Ok(q) => q,
            Err(e) => {
                let mut g = self.inner.0.lock().unwrap();
                if g.quotes.is_empty() {
                    g.status = format!("quotes unreachable ({e})");
                } else {
                    g.status = format!("refresh failed · using last {} quotes", g.quotes.len());
                }
                return;
            }
        };

        // Stable order: follow configured symbol list.
        let order = symbols;
        quotes.sort_by(|a, b| {
            let ia = order
                .iter()
                .position(|s| s.eq_ignore_ascii_case(&a.symbol))
                .unwrap_or(usize::MAX);
            let ib = order
                .iter()
                .position(|s| s.eq_ignore_ascii_case(&b.symbol))
                .unwrap_or(usize::MAX);
            ia.cmp(&ib)
        });

        let got = quotes.len();
        let mut g = self.inner.0.lock().unwrap();
        if !quotes.is_empty() {
            g.quotes = quotes;
            g.generation += 1;
            g.status = format!("{got}/{n} S&P quotes");
        } else if g.quotes.is_empty() {
            g.status = "no quotes returned".into();
        }
    }
}

/// Resolve config aliases: empty / "SP500" / "SP250" → embedded universes.
pub fn expand_symbols(symbols: Vec<String>) -> Vec<String> {
    if symbols.is_empty() {
        return default_symbols();
    }
    let mut out = Vec::new();
    let mut want_sp500 = false;
    let mut want_sp250 = false;
    for s in symbols {
        let t = s.trim();
        if t.is_empty() {
            continue;
        }
        let u = t.to_ascii_uppercase();
        if u == "SP500" || u == "S&P500" || u == "S&P 500" {
            want_sp500 = true;
        } else if u == "SP250" || u == "S&P250" || u == "S&P 250" {
            want_sp250 = true;
        } else {
            // Yahoo prefers BRK-B over BRK.B
            out.push(t.replace('.', "-").to_ascii_uppercase());
        }
    }
    if want_sp500 {
        return sp500_symbols();
    }
    if want_sp250 {
        return sp250_symbols();
    }
    if out.is_empty() {
        default_symbols()
    } else {
        out.sort();
        out.dedup();
        out
    }
}

/// Full S&P 500 (default ticker universe).
pub fn default_symbols() -> Vec<String> {
    sp500_symbols()
}

pub fn sp500_symbols() -> Vec<String> {
    include_str!("data/sp500.txt")
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|s| s.replace('.', "-").to_ascii_uppercase())
        .collect()
}

/// First 250 symbols of the S&P 500 list (alias `SP250` in config).
/// Not a separate official index — a convenient half-universe for denser tape.
pub fn sp250_symbols() -> Vec<String> {
    sp500_symbols().into_iter().take(250).collect()
}

/// Yahoo spark accepts ~20 symbols per request; we fan out in parallel.
fn fetch_yahoo_spark_batch(symbols: &[String]) -> Result<Vec<Quote>, String> {
    const CHUNK: usize = 20;
    let chunks: Vec<Vec<String>> = symbols
        .chunks(CHUNK)
        .map(|c| c.to_vec())
        .collect();

    let mut handles = Vec::new();
    // Limit concurrency so we don't trip rate limits (~8 in flight).
    let pool = 8usize;
    let mut quotes = Vec::with_capacity(symbols.len());
    let mut errors = 0u32;

    for batch in chunks.chunks(pool) {
        handles.clear();
        for chunk in batch {
            let chunk = chunk.clone();
            handles.push(thread::spawn(move || fetch_spark_chunk(&chunk)));
        }
        for h in handles.drain(..) {
            match h.join() {
                Ok(Ok(mut q)) => quotes.append(&mut q),
                Ok(Err(_)) | Err(_) => errors += 1,
            }
        }
    }

    if quotes.is_empty() {
        return Err(format!("all batches failed ({errors} errors)"));
    }
    Ok(quotes)
}

fn fetch_spark_chunk(symbols: &[String]) -> Result<Vec<Quote>, String> {
    let joined = symbols.join(",");
    let url = format!(
        "https://query1.finance.yahoo.com/v7/finance/spark?symbols={joined}&range=5d&interval=1d"
    );
    let resp = ureq::get(&url)
        .set(
            "User-Agent",
            "Mozilla/5.0 (compatible; meanwhile/0.4; +https://github.com/brendan-jarvis/meanwhile)",
        )
        .set("Accept", "application/json")
        .timeout(Duration::from_secs(25))
        .call()
        .map_err(|e| e.to_string())?;
    let v: Value = resp.into_json().map_err(|e| e.to_string())?;
    let results = v
        .pointer("/spark/result")
        .and_then(|r| r.as_array())
        .cloned()
        .unwrap_or_default();

    let mut out = Vec::new();
    for item in results {
        let sym = item
            .get("symbol")
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .to_string();
        let resp0 = item
            .pointer("/response/0")
            .or_else(|| item.get("response").and_then(|r| r.get(0)));
        let Some(resp0) = resp0 else {
            continue;
        };
        let meta = match resp0.get("meta") {
            Some(m) => m,
            None => continue,
        };
        let price = match meta
            .get("regularMarketPrice")
            .and_then(|x| x.as_f64())
            .or_else(|| meta.get("previousClose").and_then(|x| x.as_f64()))
        {
            Some(p) => p,
            None => continue,
        };
        let prev = meta
            .get("chartPreviousClose")
            .and_then(|x| x.as_f64())
            .or_else(|| meta.get("previousClose").and_then(|x| x.as_f64()))
            .unwrap_or(price);
        let change = price - prev;
        let change_pct = if prev.abs() > 1e-9 {
            change / prev * 100.0
        } else {
            0.0
        };
        let symbol = if sym.is_empty() {
            meta.get("symbol")
                .and_then(|s| s.as_str())
                .unwrap_or("?")
                .to_string()
        } else {
            sym
        };
        out.push(Quote {
            symbol,
            price,
            change,
            change_pct,
        });
    }
    Ok(out)
}
