use crate::config::{save_config, Config};
use crate::news::{fetch_summary, DecodeBody, Headline, Newsfeed};
use crate::poetic::poetic_line;
use crate::rain::{residue_at, Glyphs, Message, Noise};
use crate::stocks::{Quote, StockFeed};
use crate::term::{StyleId, Term};
use crate::theme::{
    build_palette, load_auto_theme, query_terminal_theme, theme_sources_mtime, ThemeColors,
    Palette, GLYPHS_ASCII, GLYPHS_KATA,
};
use crate::update::UpdateChecker;
use rand::seq::SliceRandom;
use rand::Rng;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

static RESIZED: AtomicBool = AtomicBool::new(false);

const WAKE: &[&str] = &[
    "Wake up, Neo...",
    "The Matrix has you.",
    "Follow the white rabbit.",
    "Knock, knock, Neo.",
];

const HELP: &[&str] = &[
    " meanwhile — things happening right now ",
    "",
    "  q        quit              space   pause",
    "  $        pure ticker mode  (full-screen quotes — no rain)",
    "  k        quotes in rain    (stocks among news / poems)",
    "  y        one quote now",
    "  #        edge ticker bar   @  cycle bar edge (top/bottom/left/right)",
    "  click    decode a story    shift-click  open in browser",
    "  enter    pick one to decode",
    "  t        edit topics       g       edit places (local intel)",
    "  f        focus mode        n/o     headline / poetic now",
    "  m        toggle news       p       toggle poetic",
    "  + / -    speed             r       refresh",
    "  s        status bar        d       feed debug log",
    "  ?        help",
    "",
    "  k: quotes decode in the field · #: optional edge bar · $: full-screen",
    "  CLI: meanwhile --ticker   ·  meanwhile --saver   ·  --check-feeds",
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum TickerEdge {
    Top,
    Bottom,
    Left,
    Right,
}

impl TickerEdge {
    fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "top" => Self::Top,
            "left" => Self::Left,
            "right" => Self::Right,
            _ => Self::Bottom,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Top => "top",
            Self::Bottom => "bottom",
            Self::Left => "left",
            Self::Right => "right",
        }
    }

    fn next(self) -> Self {
        match self {
            Self::Bottom => Self::Top,
            Self::Top => Self::Left,
            Self::Left => Self::Right,
            Self::Right => Self::Bottom,
        }
    }
}

/// Reserved chrome for the optional edge marquee.
#[derive(Clone, Copy)]
enum EdgeRegion {
    /// One horizontal row.
    Horizontal { row: isize, x0: isize, x1: isize },
    /// Vertical strip (characters scroll along the height).
    Vertical {
        col: isize,
        y0: isize,
        y1: isize,
        width: isize,
    },
}

struct TickerBand {
    row: isize,
    /// Integer cell offset into this band's own tape (not shared globally).
    offset: isize,
    /// +1 scroll left (content moves left), -1 scroll right.
    dir: isize,
    /// This row's exclusive slice of the universe — no symbol shared with other rows.
    cells: Vec<(char, StyleId)>,
}

struct Editor {
    kind: String, // "topics" | "places"
    input: String,
    pending: Vec<u8>,
}

struct Picker {
    sel: usize,
}

struct WakeState {
    line: usize,
    chars: f64,
    pause_until: f64,
    advance: bool,
}

struct Link {
    text: String,
    url: String,
    domain: String,
    summary: Option<String>,
}

struct Article {
    title: String,
    domain: String,
    body: DecodeBody,
    url: String,
    /// Where the user clicked / the headline sat — summary expands from here.
    origin_row: Option<isize>,
    origin_x0: Option<isize>,
}

pub struct App {
    term: Term,
    cfg: Config,
    feed: Arc<Newsfeed>,
    basic: bool,
    focus: bool,
    theme_colors: Option<ThemeColors>,
    theme_mtime: Option<SystemTime>,
    pal: Palette,
    glyphs: Glyphs,
    h: usize,
    w: usize,
    streams: Vec<Noise>,
    messages: Vec<Message>,
    news_q: Vec<Headline>,
    local_q: Vec<Headline>,
    gen: u64,
    recent: Vec<String>,
    started_at: SystemTime,
    next_msg: f64,
    paused: bool,
    show_status: bool,
    show_help: bool,
    show_debug: bool,
    editor: Option<Editor>,
    picker: Option<Picker>,
    reader_pending: Arc<Mutex<Option<(u64, Article)>>>,
    reader_req: u64,
    block_seq: u64,
    wake: Option<WakeState>,
    shown_links: Vec<Link>,
    panel_rect: Option<(isize, isize, isize, isize)>,
    news_on: bool,
    poetic_on: bool,
    /// Decode stock quotes into the rain among news/poetic (session + config).
    quotes_in_rain_on: bool,
    /// Shuffled quote queue for interspersed rain messages.
    quote_q: Vec<Quote>,
    quote_gen: u64,
    toast: (String, f64),
    /// Soft update whisper: (message, until_monotonic, url).
    update_hint: (String, f64, String),
    spans: HashMap<isize, Vec<(isize, isize)>>,
    /// Pure stock marquee — no matrix field at all.
    ticker_mode: bool,
    /// Optional edge marquee bar (top/bottom/left/right).
    rain_ticker_on: bool,
    rain_ticker_edge: TickerEdge,
    stock_feed: Option<Arc<StockFeed>>,
    ticker_bands: Vec<TickerBand>,
    /// Generation of quote snapshot last painted into band tapes.
    ticker_gen: u64,
    /// Discrete speed index into [`TICKER_FRAMES_PER_STEP`] (locked to 24 fps).
    ticker_speed_idx: usize,
    /// Frames since last global one-cell step (all rows advance together).
    ticker_frame_accum: u32,
    /// Sub-cell scroll accumulator for rain-mode strip (dt-based at rain fps).
    rain_ticker_scroll_accum: f64,
    /// Screensaver: any real key/click exits (set from CLI `--saver`).
    pub saver: bool,
    /// Incomplete OSC colour reply split across `read`s — finish eating next time.
    osc_tail: bool,
    updates: Arc<UpdateChecker>,
}

/// Fixed ticker redraw rate — speed steps are integer frames-per-cell at this fps.
const TICKER_FPS: f64 = 24.0;

/// Frames between cell steps at [`TICKER_FPS`], from slow → fast.
/// cells/sec = 24 / frames:  1.5, 2, 3, 4, 6, 8, 12, 24
const TICKER_FRAMES_PER_STEP: &[u32] = &[16, 12, 8, 6, 4, 3, 2, 1];
/// Default: 8 frames → 3 cells/sec.
const TICKER_SPEED_DEFAULT_IDX: usize = 2;

impl App {
    pub fn new(
        term: Term,
        cfg: Config,
        feed: Arc<Newsfeed>,
        updates: Arc<UpdateChecker>,
    ) -> Self {
        let basic = {
            let term_env = std::env::var("TERM").unwrap_or_default();
            let colorterm = std::env::var("COLORTERM").unwrap_or_default();
            !term_env.contains("256") && colorterm.is_empty()
        };
        let focus = cfg.focus;
        // File-based first (WezTerm / Starship / Omarchy); live OSC after enter.
        let theme_colors = if cfg.theme == "auto" {
            load_auto_theme()
        } else {
            None
        };
        let theme_mtime_val = theme_sources_mtime();
        let pal = build_palette(basic, focus, theme_colors.as_ref());
        let glyphs = Glyphs::new(if cfg.ascii_only {
            GLYPHS_ASCII
        } else {
            GLYPHS_KATA
        });
        let h = term.h;
        let w = term.w;
        let ticker_mode = cfg.mode.eq_ignore_ascii_case("ticker");
        let rain_ticker_on = cfg.rain_ticker;
        let rain_ticker_edge = TickerEdge::parse(&cfg.rain_ticker_edge);
        let quotes_in_rain_on = cfg.quotes_in_rain;
        let mut term = term;
        term.set_styles(pal.sgr.clone());
        Self {
            term,
            cfg,
            feed,
            basic,
            focus,
            theme_colors,
            theme_mtime: theme_mtime_val,
            pal,
            glyphs,
            h,
            w,
            streams: Vec::new(),
            messages: Vec::new(),
            news_q: Vec::new(),
            local_q: Vec::new(),
            gen: 0,
            recent: Vec::new(),
            started_at: SystemTime::now(),
            next_msg: monotonic() + 1.0,
            paused: false,
            show_status: false,
            show_help: false,
            show_debug: false,
            editor: None,
            picker: None,
            reader_pending: Arc::new(Mutex::new(None)),
            reader_req: 0,
            block_seq: 0,
            wake: None,
            shown_links: Vec::new(),
            panel_rect: None,
            news_on: true,
            poetic_on: true,
            quotes_in_rain_on,
            quote_q: Vec::new(),
            quote_gen: 0,
            toast: (String::new(), 0.0),
            update_hint: (String::new(), 0.0, String::new()),
            spans: HashMap::new(),
            ticker_mode,
            rain_ticker_on,
            rain_ticker_edge,
            stock_feed: None,
            ticker_bands: Vec::new(),
            ticker_gen: 0,
            ticker_speed_idx: TICKER_SPEED_DEFAULT_IDX,
            ticker_frame_accum: 0,
            rain_ticker_scroll_accum: 0.0,
            saver: false,
            osc_tail: false,
            updates,
        }
    }

    fn ticker_frames_per_step(&self) -> u32 {
        TICKER_FRAMES_PER_STEP[self.ticker_speed_idx.min(TICKER_FRAMES_PER_STEP.len() - 1)]
    }

    fn ticker_cells_per_sec(&self) -> f64 {
        TICKER_FPS / self.ticker_frames_per_step() as f64
    }

    fn ensure_stock_feed(&mut self) {
        if self.stock_feed.is_some() {
            return;
        }
        let feed = Arc::new(StockFeed::new(self.cfg.tickers.clone()));
        feed.start();
        self.stock_feed = Some(feed);
    }

    fn enter_ticker_mode(&mut self, t: f64) {
        self.ticker_mode = true;
        // Session-only — do not persist mode, or plain `meanwhile` stays stuck on tickers.
        self.streams.clear();
        self.messages.clear();
        self.wake = None;
        self.picker = None;
        self.ticker_bands.clear();
        self.ticker_gen = 0;
        self.ticker_frame_accum = 0;
        self.ensure_stock_feed();
        if let Some(ref f) = self.stock_feed {
            f.set_symbols(self.cfg.tickers.clone());
            f.wake();
        }
        self.term.clear_screen();
        self.blank_field();
        self.flash("ticker mode — pure quotes · $ for rain", t);
    }

    fn enter_rain_mode(&mut self, t: f64) {
        self.ticker_mode = false;
        self.ticker_bands.clear();
        self.ticker_frame_accum = 0;
        self.ticker_gen = 0;
        self.rain_ticker_scroll_accum = 0.0;
        if self.needs_stock_feed() {
            self.ensure_stock_feed();
            if let Some(ref f) = self.stock_feed {
                f.set_symbols(self.cfg.tickers.clone());
                f.wake();
            }
        }
        self.term.clear_screen();
        let mut note = String::from("rain mode");
        if self.quotes_in_rain_on {
            note.push_str(" · quotes in field (k)");
        }
        if self.rain_ticker_on {
            note.push_str(&format!(" · bar {}", self.rain_ticker_edge.as_str()));
        }
        self.flash(&note, t);
    }

    fn needs_stock_feed(&self) -> bool {
        self.ticker_mode || self.rain_ticker_on || self.quotes_in_rain_on
    }

    /// Bottom status row (when `s` status is shown).
    fn status_row(&self) -> isize {
        (self.h as isize - 1).max(0)
    }

    fn rain_ticker_active(&self) -> bool {
        self.rain_ticker_on && !self.ticker_mode
    }

    fn edge_region(&self) -> Option<EdgeRegion> {
        if !self.rain_ticker_active() {
            return None;
        }
        let w = self.w.saturating_sub(1).max(1) as isize;
        let status_h = if self.show_status { 1 } else { 0 };
        let y1 = (self.h as isize - 1 - status_h).max(0);
        match self.rain_ticker_edge {
            TickerEdge::Bottom => {
                let row = y1;
                Some(EdgeRegion::Horizontal {
                    row,
                    x0: 0,
                    x1: w - 1,
                })
            }
            TickerEdge::Top => Some(EdgeRegion::Horizontal {
                row: 0,
                x0: 0,
                x1: w - 1,
            }),
            TickerEdge::Left => Some(EdgeRegion::Vertical {
                col: 0,
                y0: 0,
                y1,
                width: 1,
            }),
            TickerEdge::Right => Some(EdgeRegion::Vertical {
                col: (w - 1).max(0),
                y0: 0,
                y1,
                width: 1,
            }),
        }
    }

    fn edge_guards_cell(&self, row: isize, x: isize) -> bool {
        match self.edge_region() {
            Some(EdgeRegion::Horizontal { row: r, x0, x1 }) => {
                row == r && x >= x0 && x <= x1
            }
            Some(EdgeRegion::Vertical {
                col,
                y0,
                y1,
                width,
            }) => row >= y0 && row <= y1 && x >= col && x < col + width,
            None => false,
        }
    }

    /// Column bounds for field messages (avoid overlapping a left/right edge bar).
    fn message_col_bounds(&self) -> (isize, isize) {
        let w = self.w.saturating_sub(1).max(1) as isize;
        match self.edge_region() {
            Some(EdgeRegion::Vertical {
                col,
                width,
                ..
            }) if self.rain_ticker_edge == TickerEdge::Left => (col + width + 1, w - 1),
            Some(EdgeRegion::Vertical { col, .. })
                if self.rain_ticker_edge == TickerEdge::Right =>
            {
                (1, (col - 1).max(1))
            }
            _ => (1, w.saturating_sub(1).max(1)),
        }
    }

    /// Single scrolling tape of all quotes for the optional edge bar.
    fn rebuild_rain_ticker_strip(&mut self) {
        let Some(ref feed) = self.stock_feed else {
            return;
        };
        let Some(region) = self.edge_region() else {
            return;
        };
        let (mut quotes, _status, gen) = feed.snapshot();
        quotes.sort_by(|a, b| {
            a.symbol
                .to_ascii_uppercase()
                .cmp(&b.symbol.to_ascii_uppercase())
        });

        let (anchor_row, span_len) = match region {
            EdgeRegion::Horizontal { row, x0, x1 } => (row, (x1 - x0 + 1).max(1) as usize),
            EdgeRegion::Vertical { y0, y1, .. } => (y0, (y1 - y0 + 1).max(1) as usize),
        };

        let structure_ok = self.ticker_bands.len() == 1
            && self.ticker_bands[0].row == anchor_row
            && gen == self.ticker_gen
            && !self.ticker_bands[0].cells.is_empty()
            && self.ticker_bands[0].cells.len() >= span_len;
        if structure_ok {
            // Still refresh row if status toggled moved bottom edge.
            if let EdgeRegion::Horizontal { row, .. } = region {
                self.ticker_bands[0].row = row;
            }
            return;
        }
        self.ticker_gen = gen;

        let mut cells: Vec<(char, StyleId)> = Vec::new();
        if quotes.is_empty() {
            for ch in "  fetching quotes…  ".chars() {
                cells.push((ch, self.pal.amber));
            }
        } else {
            for q in &quotes {
                cells.extend(self.quote_to_cells(q));
            }
        }
        while cells.len() < span_len {
            cells.push((' ', self.pal.blank));
        }
        for _ in 0..(span_len / 4).max(4) {
            cells.push((' ', self.pal.blank));
        }

        let len = cells.len().max(1) as isize;
        let prev_offset = self
            .ticker_bands
            .first()
            .map(|b| b.offset.rem_euclid(len))
            .unwrap_or(0);
        self.ticker_bands = vec![TickerBand {
            row: anchor_row,
            offset: prev_offset,
            dir: 1,
            cells,
        }];
    }

    fn tick_rain_ticker(&mut self, dt: f64) {
        if !self.rain_ticker_active() {
            return;
        }
        self.ensure_stock_feed();
        self.rebuild_rain_ticker_strip();
        // Rain runs ~8 fps; advance by wall-clock so strip speed matches pure ticker.
        self.rain_ticker_scroll_accum += dt * self.ticker_cells_per_sec();
        while self.rain_ticker_scroll_accum >= 1.0 {
            self.rain_ticker_scroll_accum -= 1.0;
            for b in &mut self.ticker_bands {
                let len = b.cells.len().max(1) as isize;
                b.offset = (b.offset + b.dir).rem_euclid(len);
            }
        }
    }

    fn draw_rain_ticker_strip(&mut self) {
        if !self.rain_ticker_active() {
            return;
        }
        self.rebuild_rain_ticker_strip();
        let Some(region) = self.edge_region() else {
            return;
        };
        let Some(band) = self.ticker_bands.first() else {
            return;
        };
        let n = band.cells.len() as isize;
        if n == 0 {
            return;
        }
        let blank = self.pal.blank;
        match region {
            EdgeRegion::Horizontal { row, x0, x1 } => {
                let width = (x1 - x0 + 1).max(1) as usize;
                let spaces = " ".repeat(width);
                self.term.span_cells(row, x0, blank, &spaces);
                for i in 0..width {
                    let idx = (band.offset + i as isize).rem_euclid(n) as usize;
                    let (ch, style) = band.cells[idx];
                    self.term.cell(row, x0 + i as isize, style, ch);
                }
            }
            EdgeRegion::Vertical {
                col,
                y0,
                y1,
                width,
            } => {
                for row in y0..=y1 {
                    for dx in 0..width {
                        let i = (row - y0) + dx * (y1 - y0 + 1);
                        let idx = (band.offset + i).rem_euclid(n) as usize;
                        let (ch, style) = band.cells[idx];
                        self.term.cell(row, col + dx, style, ch);
                    }
                }
            }
        }
    }

    fn clear_edge_region(&mut self) {
        let Some(region) = self.edge_region() else {
            return;
        };
        let blank = self.pal.blank;
        match region {
            EdgeRegion::Horizontal { row, x0, x1 } => {
                let width = (x1 - x0 + 1).max(1) as usize;
                self.term
                    .span_cells(row, x0, blank, &" ".repeat(width));
            }
            EdgeRegion::Vertical {
                col,
                y0,
                y1,
                width,
            } => {
                for row in y0..=y1 {
                    for dx in 0..width {
                        self.term.cell(row, col + dx, blank, ' ');
                    }
                }
            }
        }
    }

    fn format_quote_message(q: &Quote) -> (String, String) {
        let arrow = if q.direction() > 0 {
            "▲"
        } else if q.direction() < 0 {
            "▼"
        } else {
            "═"
        };
        let text = format!(
            "{}  {:.2}  {}{:.2} ({:+.2}%)",
            q.symbol,
            q.price,
            arrow,
            q.change.abs(),
            q.change_pct
        );
        let kind = match q.direction() {
            1 => "ticker_up".into(),
            -1 => "ticker_down".into(),
            _ => "ticker".into(),
        };
        (text, kind)
    }

    fn next_quote(&mut self) -> Option<Quote> {
        self.ensure_stock_feed();
        let Some(ref feed) = self.stock_feed else {
            return None;
        };
        let (quotes, _status, gen) = feed.snapshot();
        if quotes.is_empty() {
            return None;
        }
        if gen != self.quote_gen || self.quote_q.is_empty() {
            self.quote_gen = gen;
            self.quote_q = quotes;
            self.quote_q.shuffle(&mut rand::thread_rng());
        }
        self.quote_q.pop()
    }

    /// Wipe the whole field to blank — used so ticker never sits on matrix residue.
    fn blank_field(&mut self) {
        let blank = self.pal.blank;
        let w = self.w.saturating_sub(1).max(1);
        let spaces = " ".repeat(w);
        for y in 0..self.h {
            self.term.span_cells(y as isize, 0, blank, &spaces);
        }
    }

    fn quote_to_cells(&self, q: &crate::stocks::Quote) -> Vec<(char, StyleId)> {
        let mut cells = Vec::new();
        let dir = q.direction();
        let chg = match dir {
            1 => self.pal.up,
            -1 => self.pal.down,
            _ => self.pal.amber,
        };
        for ch in format!("  {} ", q.symbol).chars() {
            cells.push((ch, self.pal.amber));
        }
        for ch in format!("{:.2} ", q.price).chars() {
            cells.push((ch, self.pal.reader));
        }
        let arrow = if dir > 0 {
            "▲"
        } else if dir < 0 {
            "▼"
        } else {
            "═"
        };
        let chg_txt = format!("{}{:.2} ({:+.2}%)", arrow, q.change.abs(), q.change_pct);
        for ch in chg_txt.chars() {
            cells.push((ch, chg));
        }
        for ch in "  ···  ".chars() {
            cells.push((ch, self.pal.dim));
        }
        cells
    }

    /// Build band rows and assign each quote to exactly one band so a symbol
    /// never appears twice on screen (multiple rows used to share one looping tape).
    fn rebuild_ticker_layout(&mut self) {
        let Some(ref feed) = self.stock_feed else {
            return;
        };
        let (mut quotes, _status, gen) = feed.snapshot();
        // Alphabetical A→Z so layout is top→bottom, left→right in order.
        quotes.sort_by(|a, b| {
            a.symbol
                .to_ascii_uppercase()
                .cmp(&b.symbol.to_ascii_uppercase())
        });

        let usable = self.h.saturating_sub(if self.show_status { 1 } else { 0 }).max(1);
        let draw_w = self.w.saturating_sub(1).max(1);

        // How many rows? Fill the screen when we have enough names; keep enough
        // quotes per row that several tickers are visible at once (~35–45 cols each).
        let min_quotes_per_band = 6usize;
        let n_bands = if quotes.is_empty() {
            1
        } else {
            let by_density = (quotes.len() / min_quotes_per_band).max(1);
            by_density.min(usable)
        };
        // Spread bands evenly across the full height (no empty half-screen gutters).
        let rows: Vec<isize> = if n_bands == 1 {
            vec![(usable / 2) as isize]
        } else {
            (0..n_bands)
                .map(|i| (i * (usable - 1) / (n_bands - 1)) as isize)
                .collect()
        };

        let structure_ok = self.ticker_bands.len() == n_bands
            && self
                .ticker_bands
                .iter()
                .zip(rows.iter())
                .all(|(b, r)| b.row == *r);
        if structure_ok
            && gen == self.ticker_gen
            && self.ticker_bands.iter().all(|b| !b.cells.is_empty())
        {
            return;
        }
        self.ticker_gen = gen;

        // Contiguous A→Z partitions: top band = earliest names, within each
        // band tape order is left→right alphabetical.
        let mut buckets: Vec<Vec<&crate::stocks::Quote>> = vec![Vec::new(); n_bands];
        if !quotes.is_empty() {
            let chunk = (quotes.len() + n_bands - 1) / n_bands;
            for (i, q) in quotes.iter().enumerate() {
                let b = (i / chunk).min(n_bands - 1);
                buckets[b].push(q);
            }
        }

        let mut rng = rand::thread_rng();
        let mut bands = Vec::with_capacity(n_bands);
        for (i, row) in rows.into_iter().enumerate() {
            let dir = if i % 2 == 0 { 1 } else { -1 };
            let mut cells: Vec<(char, StyleId)> = Vec::new();
            if buckets[i].is_empty() {
                if i == 0 {
                    for ch in "  fetching quotes…  ".chars() {
                        cells.push((ch, self.pal.amber));
                    }
                }
            } else {
                for q in &buckets[i] {
                    cells.extend(self.quote_to_cells(q));
                }
            }
            // Pad with spaces only — never repeat quotes.
            while cells.len() < draw_w {
                cells.push((' ', self.pal.blank));
            }
            for _ in 0..(draw_w / 4).max(4) {
                cells.push((' ', self.pal.blank));
            }

            let len = cells.len().max(1) as isize;
            bands.push(TickerBand {
                row,
                offset: if structure_ok && i < self.ticker_bands.len() {
                    self.ticker_bands[i].offset.rem_euclid(len)
                } else {
                    rng.gen_range(0..len)
                },
                dir,
                cells,
            });
        }
        self.ticker_bands = bands;
    }

    fn tick_ticker(&mut self, _dt: f64) {
        self.ensure_stock_feed();
        self.rebuild_ticker_layout();
        // Frame-locked step: at TICKER_FPS, advance every N frames so +/- speed
        // lands on even integer cells/sec (24/N), not fractional 1.25× periods.
        let n = self.ticker_frames_per_step().max(1);
        self.ticker_frame_accum = self.ticker_frame_accum.saturating_add(1);
        if self.ticker_frame_accum < n {
            return;
        }
        self.ticker_frame_accum = 0;
        for b in &mut self.ticker_bands {
            let len = b.cells.len().max(1) as isize;
            b.offset = (b.offset + b.dir).rem_euclid(len);
        }
    }

    fn draw_ticker(&mut self, t: f64) {
        // Only paint tape rows (and status). Leave the rest blank from enter —
        // no full-screen wipe every frame (that caused flicker/jerk).
        let any_cells = self.ticker_bands.iter().any(|b| !b.cells.is_empty());
        if !any_cells {
            self.blank_field();
            let msg = "fetching quotes…";
            self.term
                .span_cells(self.h as isize / 2, 2, self.pal.amber, msg);
        } else {
            let draw_w = self.w.saturating_sub(1);
            let blank = self.pal.blank;
            for band in &self.ticker_bands {
                let n = band.cells.len() as isize;
                if n == 0 {
                    continue;
                }
                let spaces = " ".repeat(draw_w);
                self.term.span_cells(band.row, 0, blank, &spaces);
                for x in 0..draw_w {
                    let idx = (band.offset + x as isize).rem_euclid(n) as usize;
                    let (ch, style) = band.cells[idx];
                    self.term.cell(band.row, x as isize, style, ch);
                }
            }
        }

        if self.show_status {
            let status = self
                .stock_feed
                .as_ref()
                .map(|f| f.snapshot().1)
                .unwrap_or_else(|| "ticker".into());
            let n_sym = self
                .stock_feed
                .as_ref()
                .map(|f| f.snapshot().0.len())
                .unwrap_or(0);
            let line = format!(
                " meanwhile · ticker · {status} · {n_sym} on tape · $ rain · q quit "
            );
            let max = self.w.saturating_sub(1);
            let mut display: String = line.chars().take(max).collect();
            while display.chars().count() < max {
                display.push(' ');
            }
            self.term
                .span_cells(self.h as isize - 1, 0, self.pal.dim, &display);
        }

        self.paint_footer_hints(t);

        if self.panel_open() {
            self.draw_panel();
        }
        let _ = self.term.present();
    }

    fn check_theme(&mut self, t: f64) {
        if self.cfg.theme != "auto" {
            return;
        }
        let mtime = theme_sources_mtime();
        if mtime == self.theme_mtime {
            return;
        }
        // Config files changed — reload from disk (no OSC mid-loop: replies
        // would race with keys/mouse). Live OSC is only used at startup.
        let new = match load_auto_theme() {
            Some(c) => c,
            None => {
                self.theme_mtime = mtime;
                return;
            }
        };
        self.theme_mtime = mtime;
        if Some(&new) == self.theme_colors.as_ref() {
            return;
        }
        let name = new.name.clone();
        self.theme_colors = Some(new);
        self.apply_palette();
        self.term.clear_screen();
        self.flash(&format!("theme: {name}"), t);
    }

    /// After the TTY is raw, ask the terminal for its real palette (WezTerm etc.).
    fn adopt_terminal_theme(&mut self, t: f64) {
        if self.cfg.theme != "auto" {
            return;
        }
        let Some(new) = query_terminal_theme() else {
            return;
        };
        if self
            .theme_colors
            .as_ref()
            .is_some_and(|c| theme_rgb_eq(c, &new))
        {
            // Keep the nicer file-based name if colors already match.
            return;
        }
        let name = new.name.clone();
        let had = self.theme_colors.is_some();
        self.theme_colors = Some(new);
        self.apply_palette();
        // Only full clear when replacing an existing palette mid-session feel;
        // at startup the screen is already blank from enter().
        if had {
            self.term.clear_screen();
        }
        self.flash(&format!("theme: {name}"), t);
    }

    fn apply_palette(&mut self) {
        self.pal = build_palette(self.basic, self.focus, self.theme_colors.as_ref());
        self.term.set_styles(self.pal.sgr.clone());
    }

    fn panel_open(&self) -> bool {
        self.show_help || self.show_debug || self.editor.is_some() || self.picker.is_some()
    }

    fn guard(&self, row: isize, x: isize) -> bool {
        if self.show_status && row == self.status_row() {
            return true;
        }
        if self.edge_guards_cell(row, x) {
            return true;
        }
        // Modal panels (help, debug, editors, picker) own their rectangle —
        // rain, residue, and headlines must never paint there.
        if let Some((y0, x0, y1, x1)) = self.panel_rect {
            if y0 <= row && row <= y1 && x0 <= x && x <= x1 {
                return true;
            }
        }
        // Phase-aware: only settled message ink blocks residue/streams.
        for m in &self.messages {
            if m.row == row && m.guards_stream_cell(x) {
                return true;
            }
        }
        false
    }

    fn next_headline(&mut self) -> Option<Headline> {
        let (items, generation, _, _) = self.feed.snapshot();
        if generation != self.gen {
            self.gen = generation;
            self.local_q = items.iter().filter(|i| i.kind == "local").cloned().collect();
            self.news_q = items.iter().filter(|i| i.kind == "news").cloned().collect();
            self.local_q.shuffle(&mut rand::thread_rng());
            self.news_q.shuffle(&mut rand::thread_rng());
        } else if self.local_q.is_empty() && self.news_q.is_empty() && !items.is_empty() {
            self.local_q = items.iter().filter(|i| i.kind == "local").cloned().collect();
            self.news_q = items.iter().filter(|i| i.kind == "news").cloned().collect();
            self.local_q.shuffle(&mut rand::thread_rng());
            self.news_q.shuffle(&mut rand::thread_rng());
        }
        let mut rng = rand::thread_rng();
        // Prefer local/place intel when configured — countries like NZ otherwise
        // get drowned by global topics.
        let prefer_local = if self.cfg.places.iter().any(|p| !p.trim().is_empty()) {
            0.75
        } else {
            0.55
        };
        if !self.local_q.is_empty() && (self.news_q.is_empty() || rng.gen_bool(prefer_local)) {
            return self.local_q.pop();
        }
        self.news_q.pop()
    }

    fn spawn_message(&mut self, t: f64, force: Option<&str>) {
        if self.messages.len() >= 4.max(self.h / 4) {
            return;
        }
        let mut kind = force.map(|s| s.to_string());
        if kind.is_none() {
            let any = self.news_on || self.poetic_on || self.quotes_in_rain_on;
            if !any {
                return;
            }
            // Quotes first (quotes_ratio), then poetic vs news as before.
            let want_quote = self.quotes_in_rain_on
                && rand::thread_rng().gen::<f64>() < self.cfg.quotes_ratio.clamp(0.0, 1.0);
            if want_quote {
                kind = Some("ticker".into());
            } else if self.news_on
                && (!self.poetic_on || rand::thread_rng().gen::<f64>() > self.cfg.poetic_ratio)
            {
                kind = Some("news".into());
            } else if self.poetic_on {
                kind = Some("poetic".into());
            } else if self.quotes_in_rain_on {
                kind = Some("ticker".into());
            } else if self.news_on {
                kind = Some("news".into());
            } else {
                return;
            }
        }
        let kind_str = kind.as_deref().unwrap_or("poetic");
        let (text, url, kind_out, domain) = if kind_str == "ticker" {
            if let Some(q) = self.next_quote() {
                let (text, k) = Self::format_quote_message(&q);
                (text, None, k, q.symbol)
            } else if force.is_some() {
                self.flash("quotes not ready yet…", t);
                return;
            } else if self.news_on {
                // Fall through to news if quotes empty mid-refresh.
                if let Some(item) = self.next_headline() {
                    let url = if item.url.is_empty() {
                        None
                    } else {
                        Some(item.url.clone())
                    };
                    (item.text, url, item.kind, item.domain)
                } else if self.poetic_on {
                    (self.pick_poetic(), None, "poetic".into(), String::new())
                } else {
                    return;
                }
            } else if self.poetic_on {
                (self.pick_poetic(), None, "poetic".into(), String::new())
            } else {
                return;
            }
        } else if kind_str == "news" {
            if let Some(item) = self.next_headline() {
                let url = if item.url.is_empty() {
                    None
                } else {
                    Some(item.url.clone())
                };
                if let Some(ref u) = url {
                    self.shown_links.retain(|l| l.url != *u);
                    self.shown_links.insert(
                        0,
                        Link {
                            text: item.text.clone(),
                            url: u.clone(),
                            domain: item.domain.clone(),
                            summary: item.summary.clone(),
                        },
                    );
                    self.shown_links.truncate(9);
                }
                (item.text, url, item.kind, item.domain)
            } else {
                if kind_str == "news"
                    && !(self.poetic_on || self.quotes_in_rain_on || force.is_some())
                {
                    return;
                }
                if self.quotes_in_rain_on {
                    if let Some(q) = self.next_quote() {
                        let (text, k) = Self::format_quote_message(&q);
                        (text, None, k, q.symbol)
                    } else if self.poetic_on || force.is_some() {
                        (self.pick_poetic(), None, "poetic".into(), String::new())
                    } else {
                        return;
                    }
                } else {
                    let text = self.pick_poetic();
                    (text, None, "poetic".into(), String::new())
                }
            }
        } else {
            let text = self.pick_poetic();
            (text, None, "poetic".into(), String::new())
        };

        let mut text = text;
        let (x_lo, x_hi) = self.message_col_bounds();
        let max_chars = ((x_hi - x_lo).max(8) as usize).saturating_sub(2);
        if text.chars().count() > max_chars {
            let take = max_chars.saturating_sub(1);
            text = text.chars().take(take).collect::<String>() + "…";
        }
        let taken: std::collections::HashSet<isize> =
            self.messages.iter().map(|m| m.row).collect();
        // Prefer rows outside an open modal / reserved chrome so headlines stay visible.
        let panel = self.panel_rect;
        let status_r = if self.show_status {
            Some(self.status_row())
        } else {
            None
        };
        let edge_rows: Vec<isize> = match self.edge_region() {
            Some(EdgeRegion::Horizontal { row, .. }) => vec![row],
            _ => Vec::new(),
        };
        let free_row = |r: isize| -> bool {
            if status_r == Some(r) || edge_rows.contains(&r) {
                return false;
            }
            match panel {
                Some((y0, _, y1, _)) => r < y0 || r > y1,
                None => true,
            }
        };
        let bottom = (self.h as isize - 2).max(1);
        let candidates: Vec<isize> = (1..bottom)
            .filter(|r| !taken.contains(r) && free_row(*r))
            .collect();
        let candidates = if candidates.is_empty() {
            // Fall back if the panel covers almost every row.
            (1..bottom)
                .filter(|r| !taken.contains(r))
                .collect()
        } else {
            candidates
        };
        if candidates.is_empty() {
            return;
        }
        let spaced: Vec<isize> = candidates
            .iter()
            .copied()
            .filter(|r| !taken.contains(&(r - 1)) && !taken.contains(&(r + 1)))
            .collect();
        let row = *spaced
            .choose(&mut rand::thread_rng())
            .or_else(|| candidates.choose(&mut rand::thread_rng()))
            .unwrap();
        // Constrain x so the line sits between left/right edge bars.
        let avail_w = ((x_hi - x_lo).max(4) + 2) as usize;
        let mut m = Message::new(text, kind_out, url, row, avail_w, t, None, 0.0);
        // Message::new places x0 in 1..avail; shift into the free column window.
        m.x0 = m.x0 + x_lo - 1;
        let n = m.text.chars().count() as isize;
        if m.x0 + n > x_hi {
            m.x0 = (x_hi - n).max(x_lo);
        }
        m.domain = domain;
        // Ensure a write-ray will cross this row (spawn if none approaching).
        self.ensure_ray_for_message(m.row, m.x0, false);
        self.messages.push(m);
    }

    /// Keep a stream on `row` whose head is still left of the message so it will strike.
    fn ensure_ray_for_message(&mut self, row: isize, x0: isize, eraser: bool) {
        let approaching = self.streams.iter().any(|s| {
            s.row == row
                && s.is_eraser() == eraser
                && s.head_col() < x0 + 2
                && !s.dead(self.w)
        });
        if approaching {
            return;
        }
        // Prefer repurposing a dead-ish row; otherwise add a dedicated sweep.
        self.streams
            .push(Noise::sweeping_row(row, self.w, eraser, x0));
    }

    fn apply_stream_rays(&mut self, t: f64) {
        // Snapshot stream heads so we don't fight the borrow checker with messages.
        let rays: Vec<(isize, isize, bool)> = self
            .streams
            .iter()
            .map(|s| (s.row, s.head_col(), s.is_eraser()))
            .collect();
        for m in &mut self.messages {
            for &(row, col, eraser) in &rays {
                if row == m.row {
                    m.hit_by_ray(t, col, eraser);
                }
            }
        }
        // After dwell, request a wipe ray on that row.
        let wipe_targets: Vec<(isize, isize)> = self
            .messages
            .iter()
            .filter(|m| m.wants_wipe_ray())
            .map(|m| (m.row, m.x0))
            .collect();
        for (row, x0) in wipe_targets {
            self.ensure_ray_for_message(row, x0, true);
        }
    }

    fn pick_poetic(&mut self) -> String {
        let elapsed = self
            .started_at
            .elapsed()
            .unwrap_or_default();
        let mut text = String::new();
        for _ in 0..8 {
            text = poetic_line(self.started_at, elapsed);
            let key: String = text.chars().take(25).collect();
            if !self.recent.contains(&key) {
                break;
            }
        }
        let key: String = text.chars().take(25).collect();
        self.recent.push(key);
        if self.recent.len() > 12 {
            let drain = self.recent.len() - 12;
            self.recent.drain(0..drain);
        }
        text
    }

    fn trigger_wake(&mut self, _t: f64) {
        self.close_panel();
        self.streams.clear();
        self.messages.clear();
        self.wake = Some(WakeState {
            line: 0,
            chars: 0.0,
            pause_until: 0.0,
            advance: false,
        });
    }

    fn tick_wake(&mut self, t: f64, dt: f64) {
        let w = match self.wake.as_mut() {
            Some(w) => w,
            None => return,
        };
        if t < w.pause_until {
            if w.advance {
                w.advance = false;
                w.chars = 0.0;
            }
            return;
        }
        if w.advance {
            if w.line >= WAKE.len() - 1 {
                self.wake = None;
                self.term.clear_screen();
                return;
            }
            w.advance = false;
            w.line += 1;
            w.chars = 0.0;
            self.term.clear_screen();
        }
        // re-borrow after possible clear
        let w = self.wake.as_mut().unwrap();
        let line = WAKE[w.line];
        if w.chars < line.len() as f64 {
            w.chars += 11.0 * dt;
            if w.chars >= line.len() as f64 {
                w.pause_until = t + if w.line == WAKE.len() - 1 { 2.8 } else { 1.7 };
                w.advance = true;
            }
        }
    }

    fn tick(&mut self, t: f64, dt: f64) {
        if self.ticker_mode {
            // Keep the marquee moving under modals; draw still paints the panel last.
            if !self.paused {
                self.tick_ticker(dt);
            }
            return;
        }
        if self.wake.is_some() {
            self.tick_wake(t, dt);
            return;
        }
        // Don't start the wake sequence while a panel is up (it would steal the UI).
        if !self.panel_open() && rand::thread_rng().gen::<f64>() < dt / 3600.0 {
            self.trigger_wake(t);
            return;
        }
        // Quote strip keeps moving even while rain is paused (like a real ticker).
        if !self.paused {
            let mult = self.cfg.speed;
            // Density-scaled streams with a hard cap so large splits stay quiet.
            // Keep spawning under modals so the field doesn't thin while help is open.
            let target = 4
                .max((self.h as f64 * self.cfg.density * 1.1) as usize)
                .min(20);
            while self.streams.len() < target && rand::thread_rng().gen::<f64>() < 0.35 {
                let eraser = rand::thread_rng().gen::<f64>() < 0.22;
                self.streams.push(Noise::new(self.h, self.w, eraser));
            }
            for s in &mut self.streams {
                s.update(dt, mult);
            }
            // Rain heads write / wipe messages as they cross each line.
            self.apply_stream_rays(t);
            for m in &mut self.messages {
                m.update(t, dt, mult);
            }
            // Dwell may have just flipped to awaiting_wipe — spawn erasers.
            self.apply_stream_rays(t);
            if t >= self.next_msg {
                self.spawn_message(t, None);
                self.next_msg =
                    t + self.cfg.message_every_seconds * rand::thread_rng().gen_range(0.7..1.4);
            }
        }
        self.tick_rain_ticker(dt);
    }

    fn draw(&mut self, t: f64) {
        if self.ticker_mode {
            // Pure marquee path — never touches matrix streams/glyphs.
            let rect = if self.panel_open() {
                Some(self.compute_panel_rect())
            } else {
                None
            };
            self.panel_rect = rect;
            self.draw_ticker(t);
            return;
        }
        if let Some(ref w) = self.wake {
            let line = WAKE[w.line];
            let n = (w.chars as usize).min(line.len());
            let shown = format!("{}█", &line[..n]);
            let scramble = self.pal.scramble;
            self.term.span_cells(2, 3, scramble, &shown);
            let _ = self.term.present();
            return;
        }

        self.spans.clear();
        for m in &self.messages {
            self.spans.entry(m.row).or_default().push(m.span_range());
        }

        // mount pending summary (skip under modal — would steal focus visually)
        if !self.panel_open() {
            let pending = self.reader_pending.lock().unwrap().take();
            if let Some((tok, art)) = pending {
                self.mount_summary(t, tok, art);
            }
        }

        let rect = if self.panel_open() {
            Some(self.compute_panel_rect())
        } else {
            None
        };
        if let Some(r) = rect {
            if let Some(old) = self.panel_rect {
                if old != r {
                    self.clear_rect(old);
                }
            }
            self.panel_rect = Some(r);
        } else {
            self.panel_rect = None;
        }

        // Keep the field alive under modals. Streams/residue/headlines never
        // paint inside `panel_rect` (guard), and the panel is drawn last.
        if !self.paused {
            // Very sparse settled-field sparkle (a handful of cells per frame).
            let n = 3.max((self.w * self.h) / 1200).min(12);
            let mut rng = rand::thread_rng();
            let max_x = self.w.saturating_sub(1).max(1);
            for _ in 0..n {
                let y = rng.gen_range(0..self.h) as isize;
                let x = rng.gen_range(0..max_x) as isize;
                if !self.guard(y, x) {
                    let (style, ch) = residue_at(y, x, t, &self.glyphs, &self.pal);
                    self.term.cell(y, x, style, ch);
                }
            }

            // Shared guard snapshot for this frame.
            let status_row = if self.show_status {
                Some(self.status_row())
            } else {
                None
            };
            let edge = self.edge_region();
            let panel = self.panel_rect;
            // Phase-aware: streams may paint the scramble front; settled ink is guarded.
            // Snapshot guard decisions so we don't need to borrow messages while drawing.
            let mut locked: HashMap<isize, Vec<(isize, isize)>> = HashMap::new();
            for m in &self.messages {
                let n = m.text.chars().count() as isize;
                let (lo, hi) = m.span_range();
                for x in lo..=hi {
                    if m.guards_stream_cell(x) {
                        locked.entry(m.row).or_default().push((x, x));
                    }
                }
                let _ = n;
            }

            for s in &mut self.streams {
                let guard_fn = |row: isize, x: isize| -> bool {
                    if status_row == Some(row) {
                        return true;
                    }
                    match edge {
                        Some(EdgeRegion::Horizontal { row: r, x0, x1 }) => {
                            if row == r && x >= x0 && x <= x1 {
                                return true;
                            }
                        }
                        Some(EdgeRegion::Vertical {
                            col,
                            y0,
                            y1,
                            width,
                        }) => {
                            if row >= y0 && row <= y1 && x >= col && x < col + width {
                                return true;
                            }
                        }
                        None => {}
                    }
                    if let Some((y0, x0, y1, x1)) = panel {
                        if y0 <= row && row <= y1 && x0 <= x && x <= x1 {
                            return true;
                        }
                    }
                    if let Some(cols) = locked.get(&row) {
                        for &(a, b) in cols {
                            if x >= a && x <= b {
                                return true;
                            }
                        }
                    }
                    false
                };
                s.draw(
                    &mut self.term,
                    t,
                    &self.pal,
                    &self.glyphs,
                    guard_fn,
                );
            }
            self.streams.retain(|s| !s.dead(self.w));

            for m in &mut self.messages {
                // Skip headlines that intersect the modal so glyphs never show
                // through the panel (partial clip would need per-cell paint).
                if let Some((y0, x0, y1, x1)) = self.panel_rect {
                    let (lo, hi) = m.span_range();
                    if m.row >= y0 && m.row <= y1 && !(hi < x0 || lo > x1) {
                        continue;
                    }
                }
                m.draw(&mut self.term, t, &self.pal, &self.glyphs);
            }
            self.messages.retain(|m| !m.done);
        }

        // Status / toast sit under the modal and must not cover it either.
        if self.show_status && !self.panel_open() {
            let (_, _, status, at) = self.feed.snapshot();
            let when = at
                .map(|ts| {
                    use std::time::UNIX_EPOCH;
                    let secs = ts.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
                    let day_secs = (secs as i64 + local_offset_secs()) as u64 % 86400;
                    let hh = day_secs / 3600;
                    let mm = (day_secs % 3600) / 60;
                    format!(" · refreshed {hh:02}:{mm:02}")
                })
                .unwrap_or_default();
            let places: Vec<&str> = self
                .cfg
                .places
                .iter()
                .map(|p| p.as_str())
                .filter(|p| !p.trim().is_empty())
                .collect();
            let mut line = format!(
                " meanwhile · {status}{when} · topics: {}",
                self.cfg.topics.join(", ")
            );
            if !places.is_empty() {
                line.push_str(&format!(" · ⌖ {}", places.join(", ")));
            }
            line.push_str(" · q quit ? help ");
            let mut display: String = line.chars().take(self.w.saturating_sub(1)).collect();
            while display.chars().count() < self.w.saturating_sub(1) {
                display.push(' ');
            }
            let dim = self.pal.dim;
            self.term
                .span_cells(self.status_row(), 0, dim, &display);
        }

        // Quote strip after rain (and status), before modals.
        if !self.panel_open() {
            self.draw_rain_ticker_strip();
        }

        if !self.panel_open() {
            self.paint_footer_hints(t);
        }

        if self.panel_open() {
            self.draw_panel();
        }
        let _ = self.term.present();
    }

    /// Update whisper (right) and short toast (also right) on the bottom row.
    fn paint_footer_hints(&mut self, t: f64) {
        // Promote a pending release notice once.
        if let Some(offer) = self.updates.take_available() {
            self.update_hint = (
                format!(" update {} available · github releases ", offer.tag),
                t + 12.0,
                offer.url,
            );
        }

        let dim = self.pal.dim;
        // Prefer the status row; else the bottom edge bar; else the last row.
        let row = if self.show_status {
            self.status_row()
        } else if let Some(EdgeRegion::Horizontal { row, .. }) = self.edge_region() {
            if self.rain_ticker_edge == TickerEdge::Bottom {
                row
            } else {
                self.status_row()
            }
        } else {
            self.status_row()
        };

        let (ref hint, hint_until, ref _url) = self.update_hint;
        if !hint.is_empty() && t < hint_until {
            let x = (self.w as isize - hint.chars().count() as isize - 1).max(0);
            self.term.span_cells(row, x, dim, hint);
        }

        let (ref msg, until) = self.toast;
        if !msg.is_empty() && t < until {
            let shown = format!(" {msg} ");
            let x = (self.w as isize - shown.chars().count() as isize - 3).max(0);
            self.term.span_cells(row, x, dim, &shown);
        }
    }

    fn open_summary(&mut self, link: Link, t: f64, origin: Option<(isize, isize)>) {
        self.flash(
            &format!("decoding {}…", if link.domain.is_empty() { "story" } else { &link.domain }),
            t,
        );
        self.reader_req += 1;
        let tok = self.reader_req;
        let api_key = self.feed.api_key().unwrap_or_default();
        let pending = Arc::clone(&self.reader_pending);
        let url = link.url.clone();
        let text = link.text.clone();
        let domain = link.domain.clone();
        let feed_summary = link.summary.clone();
        let (origin_row, origin_x0) = match origin {
            Some((r, x)) => (Some(r), Some(x)),
            None => (None, None),
        };
        thread::Builder::new()
            .name("summary".into())
            .spawn(move || {
                let outcome = fetch_summary(
                    &api_key,
                    &url,
                    &text,
                    &domain,
                    feed_summary.as_deref(),
                );
                *pending.lock().unwrap() = Some((
                    tok,
                    Article {
                        title: outcome.title,
                        domain: outcome.domain,
                        body: outcome.body,
                        url: outcome.url,
                        origin_row,
                        origin_x0,
                    },
                ));
            })
            .ok();
    }

    fn mount_summary(&mut self, t: f64, tok: u64, art: Article) {
        if tok != self.reader_req {
            return;
        }
        // No real blurb → toast only; never paint instructional prose into the rain.
        let Some(summary) = art.body.text().map(str::to_string) else {
            if let Some(msg) = art.body.toast() {
                self.flash(msg, t);
            }
            return;
        };
        let width = 30.max((self.w.saturating_sub(16)).min(72));
        let wrapped = textwrap::wrap(&summary, width);
        let lines: Vec<_> = wrapped.into_iter().take(6).collect();
        let mut block: Vec<(String, String)> = vec![(
            art.title.chars().take(width).collect(),
            "news".into(),
        )];
        for s in &lines {
            block.push((s.to_string(), "summary".into()));
        }
        let k = block.len() as isize;
        if k <= 0 {
            return;
        }

        // Expand out of the clicked headline (or the on-screen message for that URL).
        let origin_row = art
            .origin_row
            .or_else(|| {
                self.messages
                    .iter()
                    .find(|m| m.url.as_ref() == Some(&art.url))
                    .map(|m| m.row)
            })
            .unwrap_or(1);
        let origin_x0 = art
            .origin_x0
            .or_else(|| {
                self.messages
                    .iter()
                    .find(|m| m.url.as_ref() == Some(&art.url))
                    .map(|m| m.x0)
            })
            .unwrap_or(2);

        // Keep title on the origin row; body drops below. If it would run off
        // the bottom (status row / edge), slide the whole block up.
        let top_min = 1isize;
        let bottom_max = (self.h as isize - 2).max(top_min); // leave last row free
        let mut r0 = origin_row.clamp(top_min, bottom_max);
        let overflow = (r0 + k - 1) - bottom_max;
        if overflow > 0 {
            r0 = (r0 - overflow).max(top_min);
        }
        // If the block is taller than the screen, pin to top and clip is already
        // handled by take(6); still clamp.
        if r0 + k - 1 > bottom_max {
            r0 = top_min;
        }

        let max_x = 1.max(self.w as isize - width as isize - 2);
        let x0 = origin_x0.clamp(1, max_x);

        // Clear anything already sitting on the rows we are about to occupy
        // (including the original headline), so the story can grow in place.
        let block_rows: std::collections::HashSet<isize> =
            (r0..r0 + k).collect();
        self.messages
            .retain(|m| !block_rows.contains(&m.row));

        // Heal the field: if we bumped the block up away from the click, or
        // the origin row is no longer the title row, fill vacated rows with
        // residue glyphs so the rain doesn't leave a blank hole.
        let heal_lo = r0.min(origin_row);
        let heal_hi = (r0 + k - 1).max(origin_row);
        for row in heal_lo..=heal_hi {
            if block_rows.contains(&row) {
                continue;
            }
            self.fill_row_residue(row, t);
        }
        // Also heal a couple of rows immediately below a bottom-clamped block
        // (where the body would have overflowed).
        if overflow > 0 {
            for row in (r0 + k)..=(r0 + k - 1 + overflow).min(bottom_max) {
                if !block_rows.contains(&row) {
                    self.fill_row_residue(row, t);
                }
            }
        }

        self.block_seq += 1;
        let group = self.block_seq;
        let dwell = 10.0 + 0.03 * summary.len() as f64;
        for (i, (text, kind)) in block.into_iter().enumerate() {
            let row = r0 + i as isize;
            if row > bottom_max {
                break;
            }
            let url = if kind == "news" {
                Some(art.url.clone())
            } else {
                None
            };
            let mut m = Message::new(
                text,
                kind,
                url,
                row,
                self.w,
                t,
                Some(x0),
                0.28 * i as f64,
            );
            m.domain = art.domain.clone();
            m.group = Some(group);
            m.dwell = dwell;
            self.messages.push(m);
        }
    }

    /// Paint a full row of settled code so vacated space after a summary
    /// bump doesn't sit blank.
    fn fill_row_residue(&mut self, row: isize, t: f64) {
        if row < 0 || row as usize >= self.h {
            return;
        }
        let max_x = self.w.saturating_sub(1);
        for x in 0..max_x {
            let (style, ch) = residue_at(row, x as isize, t, &self.glyphs, &self.pal);
            self.term.cell(row, x as isize, style, ch);
        }
    }

    fn dismiss_summaries(&mut self, group: Option<u64>) {
        for m in &mut self.messages {
            if m.group.is_some() && (group.is_none() || m.group == group) {
                m.force_erase();
            }
        }
    }

    fn click(&mut self, x: isize, y: isize, t: f64) {
        for m in &self.messages {
            let n = m.text.chars().count() as isize;
            if m.row == y && m.x0 - 1 <= x && x <= m.x0 + n {
                if m.group.is_some() {
                    let g = m.group;
                    self.dismiss_summaries(g);
                    return;
                }
                if let Some(ref url) = m.url {
                    let summary = self
                        .shown_links
                        .iter()
                        .find(|l| l.url == *url)
                        .and_then(|l| l.summary.clone());
                    let origin = Some((m.row, m.x0));
                    let link = Link {
                        text: m.text.clone(),
                        url: url.clone(),
                        domain: m.domain.clone(),
                        summary,
                    };
                    self.open_summary(link, t, origin);
                    return;
                }
            }
        }
    }

    /// Shift-click: open the article in the system browser (OSC 8 is unreliable
    /// under application mouse tracking in WezTerm).
    fn shift_click(&mut self, x: isize, y: isize, t: f64) {
        if let Some(url) = self.url_at(x, y) {
            match open_url(&url) {
                Ok(()) => self.flash("opening in browser…", t),
                Err(e) => self.flash(&format!("open failed: {e}"), t),
            }
        }
    }

    fn url_at(&self, x: isize, y: isize) -> Option<String> {
        for m in &self.messages {
            let n = m.text.chars().count() as isize;
            if m.row == y && m.x0 - 1 <= x && x <= m.x0 + n {
                if let Some(ref url) = m.url {
                    if !url.is_empty() {
                        return Some(url.clone());
                    }
                }
            }
        }
        // Fall back to recently shown headlines near this row? only exact hit.
        None
    }

    fn panel_lines(&self) -> Vec<String> {
        if self.show_help {
            return HELP.iter().map(|s| s.to_string()).collect();
        }
        if self.show_debug {
            let mut lines = vec![
                " ▞ feed debug ".into(),
                String::new(),
            ];
            let log = self.feed.last_log();
            if log.is_empty() {
                // Fall back to on-disk log from last process.
                let path = crate::config::fetch_log_path();
                match std::fs::read_to_string(&path) {
                    Ok(t) if !t.trim().is_empty() => {
                        lines.push(format!("   (from {})", path.display()));
                        lines.push(String::new());
                        for l in t.lines().take(18) {
                            lines.push(format!("  {l}"));
                        }
                    }
                    _ => {
                        lines.push("   no fetch log yet — press r to refresh".into());
                        lines.push(format!("   or: meanwhile --check-feeds"));
                        lines.push(format!("   log path: {}", path.display()));
                    }
                }
            } else {
                for l in log.lines().take(18) {
                    lines.push(format!("  {l}"));
                }
            }
            lines.push(String::new());
            lines.push("   r refreshes · d/esc closes · --check-feeds on CLI".into());
            return lines;
        }
        if let Some(ref picker) = self.picker {
            let mut lines = vec![" ▚ decode a story ".into(), String::new()];
            for (i, l) in self.shown_links.iter().enumerate() {
                let mark = if i == picker.sel { "▸" } else { " " };
                let text: String = l.text.chars().take(70).collect();
                lines.push(format!(" {mark} {}  {text}", i + 1));
            }
            lines.push(String::new());
            lines.push("   ↑/↓ + enter · or just click a headline in the rain".into());
            lines.push("   its story decodes into the stream · esc closes".into());
            return lines;
        }
        if let Some(ref ed) = self.editor {
            let title = if ed.kind == "topics" {
                " ◈ topics — what the news feed follows "
            } else {
                " ⌖ places — local intel "
            };
            let mut lines = vec![title.into(), String::new()];
            let entries = if ed.kind == "topics" {
                &self.cfg.topics
            } else {
                &self.cfg.places
            };
            for (i, v) in entries.iter().take(9).enumerate() {
                lines.push(format!("   {}  {v}", i + 1));
            }
            if entries.is_empty() {
                lines.push("   (none yet — type one below)".into());
            }
            lines.push(String::new());
            lines.push(format!("   ▸ {}█", ed.input));
            lines.push(String::new());
            lines.push("   type + enter adds · 1-9 removes".into());
            lines.push("   enter on empty line closes · esc closes".into());
            return lines;
        }
        Vec::new()
    }

    fn compute_panel_rect(&self) -> (isize, isize, isize, isize) {
        let lines = self.panel_lines();
        let max_len = lines.iter().map(|s| s.chars().count()).max().unwrap_or(0);
        let bw = (self.w.saturating_sub(2)).min(46.max(max_len + 4));
        let bh = lines.len() + 2;
        let y0 = (self.h.saturating_sub(bh) / 2) as isize;
        let x0 = (self.w.saturating_sub(bw) / 2) as isize;
        (
            y0.max(0),
            x0.max(0),
            (y0 + bh as isize - 1).min(self.h as isize - 1),
            (x0 + bw as isize - 1).min(self.w as isize - 2),
        )
    }

    fn draw_panel(&mut self) {
        let (y0, x0, y1, x1) = match self.panel_rect {
            Some(r) => r,
            None => return,
        };
        let bw = (x1 - x0 + 1) as usize;
        let bh = (y1 - y0 + 1) as usize;
        // Opaque backdrop: wipe every cell in the rect every frame so nothing
        // from the rain buffer can show through (blank style + spaces).
        let blank = self.pal.blank;
        let spaces = " ".repeat(bw);
        for i in 0..bh {
            self.term.span_cells(y0 + i as isize, x0, blank, &spaces);
        }
        let accent = if let Some(ref ed) = self.editor {
            if ed.kind == "places" {
                self.pal.local
            } else {
                self.pal.poetic
            }
        } else if self.show_debug {
            self.pal.poetic
        } else {
            self.pal.news
        };
        let dim = self.pal.dim;
        let lines = self.panel_lines();
        for (i, s) in lines.iter().enumerate() {
            let row = y0 + 1 + i as isize;
            if row > y1 {
                break;
            }
            let attr = if i == 0 || s.trim_start().starts_with('▸') {
                accent
            } else {
                dim
            };
            let clipped: String = s.chars().take(bw.saturating_sub(3)).collect();
            // Pad each content line to full width so leftover rain glyphs
            // cannot peek past the end of short help lines.
            let mut line = format!("  {clipped}");
            while line.chars().count() < bw {
                line.push(' ');
            }
            let line: String = line.chars().take(bw).collect();
            self.term.span_cells(row, x0, attr, &line);
        }
    }

    fn clear_rect(&mut self, rect: (isize, isize, isize, isize)) {
        let (y0, x0, y1, x1) = rect;
        let blank = self.pal.blank;
        let width = (x1 - x0 + 1) as usize;
        let spaces = " ".repeat(width);
        for y in y0..=y1 {
            self.term.span_cells(y, x0, blank, &spaces);
        }
    }

    fn close_panel(&mut self) {
        self.editor = None;
        self.show_help = false;
        self.show_debug = false;
        self.picker = None;
        self.panel_rect = None;
        self.term.clear_screen();
    }

    fn flash(&mut self, msg: &str, t: f64) {
        // Toasts win the footer over a lingering update whisper.
        self.update_hint = (String::new(), 0.0, String::new());
        self.toast = (msg.to_string(), t + 2.5);
    }

    fn clear_row(&mut self, row: isize) {
        let blank = self.pal.blank;
        let spaces = " ".repeat(self.w.saturating_sub(1));
        self.term.span_cells(row, 0, blank, &spaces);
    }

    fn handle_bytes(&mut self, data: &[u8], t: f64) -> bool {
        for tok in tokenize(data, &mut self.osc_tail) {
            match tok {
                Token::Mouse { btn, mx, my, press } => {
                    // SGR mouse: button = base (0=left) | 4=shift | 8=meta | 16=ctrl.
                    // With mouse reporting on, WezTerm delivers shift-clicks to us
                    // instead of following OSC 8 — open the URL ourselves.
                    let motion = btn & 32 != 0;
                    let scroll = btn >= 64;
                    let left = (btn & 0b11) == 0;
                    let shift = btn & 4 != 0;
                    if press
                        && left
                        && !motion
                        && !scroll
                        && self.wake.is_none()
                        && self.editor.is_none()
                        && !self.show_help
                        && !self.show_debug
                    {
                        if shift {
                            self.shift_click(mx - 1, my - 1, t);
                        } else {
                            self.click(mx - 1, my - 1, t);
                        }
                    }
                }
                Token::Key(key_b) => {
                    let is_seq = false;
                    let b = key_b;
                    if !self.handle_key(b, is_seq, t) {
                        return false;
                    }
                }
                Token::Seq(key_b) => {
                    let is_seq = true;
                    let b = key_b;
                    if !self.handle_key(b, is_seq, t) {
                        return false;
                    }
                }
            }
        }
        true
    }

    fn handle_key(&mut self, b: u8, is_seq: bool, t: f64) -> bool {
        if self.wake.is_some() {
            if b == b'q' || b == 27 || b == 3 {
                return false;
            }
            return true;
        }

        if self.picker.is_some() {
            let nlinks = self.shown_links.len();
            if is_seq {
                if nlinks > 0 {
                    let sel = self.picker.as_ref().unwrap().sel;
                    if b == 0x41 {
                        self.picker.as_mut().unwrap().sel = (sel + nlinks - 1) % nlinks;
                    } else if b == 0x42 {
                        self.picker.as_mut().unwrap().sel = (sel + 1) % nlinks;
                    }
                }
                return true;
            }
            if (b == 13 || b == 10) && nlinks > 0 {
                let sel = self.picker.as_ref().unwrap().sel;
                let url = self.shown_links[sel].url.clone();
                let origin = self
                    .messages
                    .iter()
                    .find(|m| m.url.as_ref() == Some(&url))
                    .map(|m| (m.row, m.x0));
                let link = Link {
                    text: self.shown_links[sel].text.clone(),
                    url,
                    domain: self.shown_links[sel].domain.clone(),
                    summary: self.shown_links[sel].summary.clone(),
                };
                self.open_summary(link, t, origin);
                self.close_panel();
            } else if (0x31..=0x39).contains(&b) {
                let idx = (b - 0x30) as usize;
                if idx <= nlinks && idx >= 1 {
                    let url = self.shown_links[idx - 1].url.clone();
                    let origin = self
                        .messages
                        .iter()
                        .find(|m| m.url.as_ref() == Some(&url))
                        .map(|m| (m.row, m.x0));
                    let link = Link {
                        text: self.shown_links[idx - 1].text.clone(),
                        url,
                        domain: self.shown_links[idx - 1].domain.clone(),
                        summary: self.shown_links[idx - 1].summary.clone(),
                    };
                    self.open_summary(link, t, origin);
                    self.close_panel();
                } else {
                    self.close_panel();
                }
            } else {
                self.close_panel();
            }
            return true;
        }

        if is_seq {
            return true;
        }

        if self.editor.is_some() {
            let kind = self.editor.as_ref().unwrap().kind.clone();
            if b == 13 || b == 10 {
                let val = self.editor.as_ref().unwrap().input.trim().to_string();
                if val.is_empty() {
                    self.close_panel();
                    return true;
                }
                let lower = val.to_lowercase();
                if lower == "neo" || lower == "wake up" || lower == "follow the white rabbit" {
                    self.close_panel();
                    self.trigger_wake(t);
                    return true;
                }
                let exists = if kind == "topics" {
                    self.cfg.topics.iter().any(|x| x.eq_ignore_ascii_case(&val))
                } else {
                    self.cfg.places.iter().any(|x| x.eq_ignore_ascii_case(&val))
                };
                if !exists {
                    if kind == "topics" {
                        self.cfg.topics.push(val.clone());
                    } else {
                        self.cfg.places.push(val.clone());
                    }
                    let len = if kind == "topics" {
                        self.cfg.topics.len()
                    } else {
                        self.cfg.places.len()
                    };
                    save_config(&self.cfg);
                    self.feed.update_cfg(self.cfg.clone());
                    self.feed.wake();
                    let cap = if kind == "topics" { 4 } else { 3 };
                    let note = if len > cap {
                        format!(" (only the first {cap} are fetched)")
                    } else {
                        String::new()
                    };
                    self.flash(&format!("added {val} — refreshing…{note}"), t);
                }
                if let Some(ed) = self.editor.as_mut() {
                    ed.input.clear();
                    ed.pending.clear();
                }
            } else if b == 27 {
                self.close_panel();
                return true;
            } else if b == 8 || b == 127 {
                if let Some(ed) = self.editor.as_mut() {
                    ed.input.pop();
                    ed.pending.clear();
                }
            } else if (0x31..=0x39).contains(&b)
                && self.editor.as_ref().unwrap().input.is_empty()
            {
                let idx = (b - 0x30) as usize;
                let gone = if kind == "topics" {
                    if idx >= 1 && idx <= self.cfg.topics.len() {
                        Some(self.cfg.topics.remove(idx - 1))
                    } else {
                        None
                    }
                } else if idx >= 1 && idx <= self.cfg.places.len() {
                    Some(self.cfg.places.remove(idx - 1))
                } else {
                    None
                };
                if let Some(gone) = gone {
                    save_config(&self.cfg);
                    self.feed.update_cfg(self.cfg.clone());
                    self.feed.wake();
                    self.flash(&format!("removed {gone} — refreshing…"), t);
                }
            } else if b >= 32 {
                if let Some(ed) = self.editor.as_mut() {
                    ed.pending.push(b);
                    match std::str::from_utf8(&ed.pending) {
                        Ok(s) => {
                            ed.input.push_str(s);
                            ed.pending.clear();
                        }
                        Err(_) => {
                            if ed.pending.len() >= 4 {
                                ed.pending.clear();
                            }
                        }
                    }
                }
            }
            return true;
        }

        if self.show_help || self.show_debug {
            if b == b'q' || b == 3 {
                return false;
            }
            if self.show_debug && b == b'r' {
                self.feed.wake();
                self.flash("refreshing headlines…", t);
                return true;
            }
            self.close_panel();
            return true;
        }

        // Ctrl-C (ISIG off) or Esc with no story open → clean quit so mouse
        // reporting is disabled and the shell doesn't eat leftover CSI clicks.
        if b == 3 {
            return false;
        }
        if b == 27 {
            if self
                .messages
                .iter()
                .any(|m| m.group.is_some() && !m.is_erasing())
            {
                self.dismiss_summaries(None);
                return true;
            }
            return false;
        }
        if b == b'q' {
            return false;
        }
        if b == b' ' {
            self.paused = !self.paused;
            self.flash(
                if self.paused {
                    "paused — space to resume"
                } else {
                    ""
                },
                t,
            );
        } else if b == b'n' {
            if self.ticker_mode {
                self.flash("ticker mode — $ for rain", t);
            } else {
                self.spawn_message(t, Some("news"));
            }
        } else if b == b'o' {
            if self.ticker_mode {
                self.flash("ticker mode — $ for rain", t);
            } else {
                self.spawn_message(t, Some("poetic"));
            }
        } else if b == b'y' {
            if self.ticker_mode {
                self.flash("ticker mode — $ for rain", t);
            } else if !self.quotes_in_rain_on {
                self.flash("quotes in rain off · k to enable", t);
            } else {
                self.spawn_message(t, Some("ticker"));
            }
        } else if b == b'f' {
            self.focus = !self.focus;
            self.cfg.focus = self.focus;
            self.apply_palette();
            save_config(&self.cfg);
            self.flash(
                if self.focus {
                    "focus — text surfaced"
                } else {
                    "embedded — text set back"
                },
                t,
            );
        } else if b == b't' {
            self.editor = Some(Editor {
                kind: "topics".into(),
                input: String::new(),
                pending: Vec::new(),
            });
        } else if b == b'g' {
            self.editor = Some(Editor {
                kind: "places".into(),
                input: String::new(),
                pending: Vec::new(),
            });
        } else if b == 13 || b == 10 || b == b'l' {
            if !self.shown_links.is_empty() {
                self.picker = Some(Picker { sel: 0 });
            } else {
                self.flash("no headlines on screen yet", t);
            }
        } else if b == b'm' {
            self.news_on = !self.news_on;
            self.flash(if self.news_on { "news on" } else { "news off" }, t);
        } else if b == b'p' {
            self.poetic_on = !self.poetic_on;
            self.flash(
                if self.poetic_on {
                    "poetic on"
                } else {
                    "poetic off"
                },
                t,
            );
        } else if b == b'k' {
            if self.ticker_mode {
                self.flash("k is for quotes in rain · $ returns to rain", t);
            } else {
                self.quotes_in_rain_on = !self.quotes_in_rain_on;
                self.cfg.quotes_in_rain = self.quotes_in_rain_on;
                save_config(&self.cfg);
                if self.quotes_in_rain_on {
                    self.ensure_stock_feed();
                    if let Some(ref f) = self.stock_feed {
                        f.set_symbols(self.cfg.tickers.clone());
                        f.wake();
                    }
                    self.flash("quotes in rain on — mixed with news & poems", t);
                } else {
                    self.flash("quotes in rain off", t);
                }
            }
        } else if b == b'+' || b == b'=' {
            if self.ticker_mode {
                if self.ticker_speed_idx + 1 < TICKER_FRAMES_PER_STEP.len() {
                    self.ticker_speed_idx += 1;
                    self.ticker_frame_accum = 0;
                }
                self.flash(
                    &format!(
                        "ticker {:.0} cells/s · {} frames/step @ 24fps",
                        self.ticker_cells_per_sec(),
                        self.ticker_frames_per_step()
                    ),
                    t,
                );
            } else {
                self.cfg.speed = (self.cfg.speed * 1.25).min(4.0);
                self.flash(&format!("speed {:.2}x", self.cfg.speed), t);
            }
        } else if b == b'-' {
            if self.ticker_mode {
                if self.ticker_speed_idx > 0 {
                    self.ticker_speed_idx -= 1;
                    self.ticker_frame_accum = 0;
                }
                self.flash(
                    &format!(
                        "ticker {:.0} cells/s · {} frames/step @ 24fps",
                        self.ticker_cells_per_sec(),
                        self.ticker_frames_per_step()
                    ),
                    t,
                );
            } else {
                self.cfg.speed = (self.cfg.speed / 1.25).max(0.2);
                self.flash(&format!("speed {:.2}x", self.cfg.speed), t);
            }
        } else if b == b'r' {
            if self.ticker_mode {
                if let Some(ref f) = self.stock_feed {
                    f.wake();
                }
                self.flash("refreshing quotes…", t);
            } else {
                self.feed.wake();
                if self.needs_stock_feed() {
                    if let Some(ref f) = self.stock_feed {
                        f.wake();
                    }
                    self.flash("refreshing headlines + quotes…", t);
                } else {
                    self.flash("refreshing headlines…", t);
                }
            }
        } else if b == b's' {
            self.show_status = !self.show_status;
            if !self.show_status {
                self.clear_row(self.status_row());
            }
            // Status toggles reclaim/relinquish a bottom row — rebuild strip position.
            if self.rain_ticker_active() {
                self.ticker_gen = 0;
            }
        } else if b == b'#' {
            if self.ticker_mode {
                self.flash("# is for edge ticker bar · $ returns to rain", t);
            } else {
                if self.rain_ticker_on {
                    self.clear_edge_region();
                }
                self.rain_ticker_on = !self.rain_ticker_on;
                self.cfg.rain_ticker = self.rain_ticker_on;
                save_config(&self.cfg);
                if self.rain_ticker_on {
                    self.ensure_stock_feed();
                    if let Some(ref f) = self.stock_feed {
                        f.set_symbols(self.cfg.tickers.clone());
                        f.wake();
                    }
                    self.ticker_gen = 0;
                    self.flash(
                        &format!("edge bar on · {} · @ cycles", self.rain_ticker_edge.as_str()),
                        t,
                    );
                } else {
                    self.ticker_bands.clear();
                    self.flash("edge bar off", t);
                }
            }
        } else if b == b'@' {
            if self.ticker_mode {
                self.flash("@ cycles edge bar placement in rain mode", t);
            } else {
                if self.rain_ticker_on {
                    self.clear_edge_region();
                }
                self.rain_ticker_edge = self.rain_ticker_edge.next();
                self.cfg.rain_ticker_edge = self.rain_ticker_edge.as_str().into();
                save_config(&self.cfg);
                self.ticker_gen = 0;
                if !self.rain_ticker_on {
                    self.rain_ticker_on = true;
                    self.cfg.rain_ticker = true;
                    save_config(&self.cfg);
                    self.ensure_stock_feed();
                    if let Some(ref f) = self.stock_feed {
                        f.set_symbols(self.cfg.tickers.clone());
                        f.wake();
                    }
                }
                self.flash(
                    &format!("edge bar · {}", self.rain_ticker_edge.as_str()),
                    t,
                );
            }
        } else if b == b'd' {
            if self.ticker_mode {
                // In ticker mode, d still opens a simple status flash.
                let status = self
                    .stock_feed
                    .as_ref()
                    .map(|f| f.snapshot().1)
                    .unwrap_or_else(|| "no feed".into());
                self.flash(&status, t);
            } else {
                self.show_debug = true;
            }
        } else if b == b'$' {
            if self.ticker_mode {
                self.enter_rain_mode(t);
            } else {
                self.enter_ticker_mode(t);
            }
        } else if b == b'?' {
            self.show_help = true;
        }
        true
    }

    pub fn run(&mut self) -> std::io::Result<()> {
        // SIGWINCH (SIGINT/TERM handled inside Term::enter for mouse cleanup)
        unsafe {
            libc::signal(libc::SIGWINCH, handle_sigwinch as *const () as usize);
        }
        self.term.enter(self.cfg.mouse)?;
        // Inherit the active WezTerm (or other) palette now that the TTY is raw.
        self.adopt_terminal_theme(0.0);
        if self.ticker_mode {
            self.enter_ticker_mode(0.0);
        } else {
            if self.needs_stock_feed() {
                self.ensure_stock_feed();
                if let Some(ref f) = self.stock_feed {
                    f.set_symbols(self.cfg.tickers.clone());
                    f.wake();
                }
            }
            // Surface feed status early.
            let (_, _, status, _) = self.feed.snapshot();
            if status.contains("poetic only") || status.contains("unreachable") {
                self.flash(&status, 0.0);
            } else if !self.cfg.places.is_empty() {
                self.flash(
                    &format!("places: {} — fetching rss…", self.cfg.places.join(", ")),
                    0.0,
                );
            } else {
                self.flash("fetching rss…", 0.0);
            }
        }
        let mut last = monotonic();
        let mut frame = 0u64;
        // Always leave() on every exit path (including panic unwind via Drop).
        let result = (|| {
            loop {
                let t = monotonic();
                let dt = (t - last).min(0.1);
                last = t;
                frame += 1;
                // ~every 10s at 8 fps — pick up WezTerm/Starship/Omarchy changes
                if frame % 80 == 0 {
                    self.check_theme(t);
                }
                if RESIZED.swap(false, Ordering::SeqCst) {
                    self.term.resize();
                    self.h = self.term.h;
                    self.w = self.term.w;
                    self.messages.retain(|m| {
                        m.row < self.h as isize - 1
                            && m.x0 + (m.text.chars().count() as isize) < self.w as isize - 1
                    });
                    self.streams.retain(|s| s.row < self.h as isize);
                    self.ticker_bands.clear();
                    self.ticker_gen = 0; // rebuild tape width
                    self.panel_rect = None;
                    self.term.clear_screen();
                    if self.ticker_mode {
                        self.blank_field();
                    }
                }
                if !self.paused {
                    self.tick(t, dt);
                }
                self.draw(t);
                // Ambient rain uses config fps (~8); ticker is locked to 24 fps
                // so speed steps are integer frames-per-cell (even cells/sec).
                let fps = if self.ticker_mode {
                    TICKER_FPS
                } else {
                    self.cfg.fps.clamp(4.0, 30.0)
                };
                let frame_budget = 1.0 / fps;
                let target = t + frame_budget;
                let timeout = Duration::from_secs_f64((target - monotonic()).max(0.0));
                let data = self.term.read(timeout);
                if !data.is_empty() {
                    if self.saver {
                        // Any real key/mouse ends the screensaver; OSC noise does not.
                        let toks = tokenize(&data, &mut self.osc_tail);
                        if !toks.is_empty() {
                            break;
                        }
                    } else if !self.handle_bytes(&data, t) {
                        break;
                    }
                }
                // If present() ran long (busy terminal), yield extra so we don't
                // stack frames and flood the PTY further.
                let overshoot = monotonic() - target;
                if overshoot > 0.0 {
                    let extra = Duration::from_secs_f64((overshoot * 0.5).min(0.1));
                    let _ = self.term.read(extra);
                }
            }
            Ok(())
        })();
        self.term.leave();
        result
    }
}

extern "C" fn handle_sigwinch(_: i32) {
    RESIZED.store(true, Ordering::SeqCst);
}

fn monotonic() -> f64 {
    // process-relative monotonic clock via Instant
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    let start = START.get_or_init(Instant::now);
    start.elapsed().as_secs_f64()
}

fn theme_rgb_eq(a: &ThemeColors, b: &ThemeColors) -> bool {
    a.bg == b.bg
        && a.fg == b.fg
        && a.green == b.green
        && a.bgreen == b.bgreen
        && a.yellow == b.yellow
        && a.cyan == b.cyan
        && a.bwhite == b.bwhite
}

/// Open a URL in the user's browser. Handles native Linux, macOS, and WSL.
fn open_url(url: &str) -> Result<(), String> {
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("not an http(s) url".into());
    }
    // Prefer platform tools in order; WSL often has neither xdg-open nor wslview.
    let candidates: &[&[&str]] = &[
        &["xdg-open", url],
        &["wslview", url],
        &["open", url], // macOS
        &["cmd.exe", "/c", "start", "", url], // Windows via WSL
        &["/mnt/c/Windows/System32/cmd.exe", "/c", "start", "", url],
    ];
    let mut last = "no opener found".to_string();
    for args in candidates {
        let (bin, rest) = args.split_first().unwrap();
        match std::process::Command::new(bin)
            .args(rest)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            Ok(_) => return Ok(()),
            Err(e) => last = format!("{bin}: {e}"),
        }
    }
    Err(last)
}

fn local_offset_secs() -> i64 {
    // best-effort local UTC offset via localtime_r (glibc tm_gmtoff)
    unsafe {
        let mut t: libc::time_t = 0;
        libc::time(&mut t);
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&t, &mut tm).is_null() {
            return 0;
        }
        #[cfg(any(target_os = "linux", target_os = "android"))]
        {
            tm.tm_gmtoff
        }
        #[cfg(not(any(target_os = "linux", target_os = "android")))]
        {
            0
        }
    }
}

enum Token {
    Key(u8),
    Seq(u8),
    Mouse {
        btn: i32,
        mx: isize,
        my: isize,
        press: bool,
    },
}

/// Split raw input into key / CSI / mouse tokens.
///
/// OSC replies (`ESC ] … BEL` or `ESC ] … ESC \`) are swallowed so late
/// colour-query answers never become keystrokes (and cannot exit `--saver`).
/// When a reply is split across reads, `osc_tail` carries the unfinished state.
fn tokenize(data: &[u8], osc_tail: &mut bool) -> Vec<Token> {
    let mut toks = Vec::new();
    let mut i = 0;
    let n = data.len();

    if *osc_tail {
        // Finish an OSC reply that split across reads. A bare `\` is the tail
        // of ST here, not a key.
        while i < n
            && data[i] != 0x07
            && data[i] != 0x5C
            && !(data[i] == 27 && i + 1 < n && data[i + 1] == 0x5C)
        {
            i += 1;
        }
        if i < n {
            i += if data[i] == 27 { 2 } else { 1 };
            *osc_tail = false;
        } else {
            // Still waiting for BEL/ST.
            return toks;
        }
    }

    while i < n {
        let b = data[i];
        // OSC reply (e.g. late colour-query answer): swallow to BEL or ST.
        if b == 27 && i + 1 < n && data[i + 1] == 0x5D {
            let mut j = i + 2;
            while j < n
                && data[j] != 0x07
                && !(data[j] == 27 && j + 1 < n && data[j + 1] == 0x5C)
            {
                j += 1;
            }
            if j >= n {
                *osc_tail = true;
                break;
            }
            i = j + if data[j] == 27 { 2 } else { 1 };
            continue;
        }
        if b == 27 && i + 1 < n && (data[i + 1] == 0x5B || data[i + 1] == 0x4F) {
            let mut j = i + 2;
            while j < n && !(0x40..=0x7E).contains(&data[j]) {
                j += 1;
            }
            let final_b = if j < n { data[j] } else { 0 };
            let params = &data[i + 2..j.min(n)];
            if (final_b == 0x4D || final_b == 0x6D) && params.starts_with(b"<") {
                let s = String::from_utf8_lossy(&params[1..]);
                let parts: Vec<_> = s.split(';').collect();
                if parts.len() >= 3 {
                    if let (Ok(btn), Ok(mx), Ok(my)) = (
                        parts[0].parse::<i32>(),
                        parts[1].parse::<isize>(),
                        parts[2].parse::<isize>(),
                    ) {
                        toks.push(Token::Mouse {
                            btn,
                            mx,
                            my,
                            press: final_b == 0x4D,
                        });
                    }
                }
            } else {
                toks.push(Token::Seq(final_b));
            }
            i = if j < n { j + 1 } else { n };
        } else {
            toks.push(Token::Key(b));
            i += 1;
        }
    }
    toks
}
