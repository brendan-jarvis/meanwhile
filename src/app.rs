use crate::config::{save_config, Config};
use crate::news::{fetch_summary, Headline, Newsfeed};
use crate::poetic::poetic_line;
use crate::rain::{residue_at, Glyphs, Message, Noise};
use crate::term::Term;
use crate::theme::{
    build_palette, load_omarchy_colors, theme_mtime, OmarchyColors, Palette, GLYPHS_ASCII,
    GLYPHS_KATA,
};
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
    "  click    decode a story    enter   pick one to decode",
    "  t        edit topics       g       edit places (local intel)",
    "  f        focus mode        o       something true now",
    "  m        toggle news       p       toggle poetic",
    "  + / -    speed             r       refresh headlines",
    "  s        status bar        ?       help",
    "",
    "  in editors: type + enter adds · 1-9 removes · esc closes",
    "  a decoded story dissolves on its own · esc or a click hurries it",
    "",
    "  any other key closes help",
];

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
}

struct Article {
    title: String,
    domain: String,
    summary: String,
    url: String,
}

pub struct App {
    term: Term,
    cfg: Config,
    feed: Arc<Newsfeed>,
    basic: bool,
    focus: bool,
    theme_colors: Option<OmarchyColors>,
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
    toast: (String, f64),
    spans: HashMap<isize, Vec<(isize, isize)>>,
}

impl App {
    pub fn new(term: Term, cfg: Config, feed: Arc<Newsfeed>) -> Self {
        let basic = {
            let term_env = std::env::var("TERM").unwrap_or_default();
            let colorterm = std::env::var("COLORTERM").unwrap_or_default();
            !term_env.contains("256") && colorterm.is_empty()
        };
        let focus = cfg.focus;
        let theme_colors = if cfg.theme == "auto" {
            load_omarchy_colors()
        } else {
            None
        };
        let theme_mtime_val = theme_mtime();
        let pal = build_palette(basic, focus, theme_colors.as_ref());
        let glyphs = Glyphs::new(if cfg.ascii_only {
            GLYPHS_ASCII
        } else {
            GLYPHS_KATA
        });
        let h = term.h;
        let w = term.w;
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
            toast: (String::new(), 0.0),
            spans: HashMap::new(),
        }
    }

    fn check_theme(&mut self, t: f64) {
        if self.cfg.theme != "auto" {
            return;
        }
        let mtime = theme_mtime();
        if mtime == self.theme_mtime {
            return;
        }
        let new = match load_omarchy_colors() {
            Some(c) => c,
            None => return,
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

    fn apply_palette(&mut self) {
        self.pal = build_palette(self.basic, self.focus, self.theme_colors.as_ref());
        self.term.set_styles(self.pal.sgr.clone());
    }

    fn guard(&self, row: isize, x: isize) -> bool {
        if row == self.h as isize - 1 && self.show_status {
            return true;
        }
        if let Some((y0, x0, y1, x1)) = self.panel_rect {
            if y0 <= row && row <= y1 && x0 <= x && x <= x1 {
                return true;
            }
        }
        if let Some(spans) = self.spans.get(&row) {
            for &(lo, hi) in spans {
                if lo <= x && x <= hi {
                    return true;
                }
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
        if !self.local_q.is_empty() && (self.news_q.is_empty() || rng.gen_bool(0.55)) {
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
            if !(self.news_on || self.poetic_on) {
                return;
            }
            if self.news_on
                && (!self.poetic_on || rand::thread_rng().gen::<f64>() > self.cfg.poetic_ratio)
            {
                kind = Some("news".into());
            } else {
                kind = Some("poetic".into());
            }
        }
        let kind_str = kind.as_deref().unwrap_or("poetic");
        let (text, url, kind_out, domain) = if kind_str == "news" {
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
                        },
                    );
                    self.shown_links.truncate(9);
                }
                (item.text, url, item.kind, item.domain)
            } else {
                if kind_str == "news" && !(self.poetic_on || force.is_some()) {
                    return;
                }
                let text = self.pick_poetic();
                (text, None, "poetic".into(), String::new())
            }
        } else {
            let text = self.pick_poetic();
            (text, None, "poetic".into(), String::new())
        };

        let mut text = text;
        if text.chars().count() > self.w.saturating_sub(6) {
            let take = self.w.saturating_sub(9);
            text = text.chars().take(take).collect::<String>() + "…";
        }
        let taken: std::collections::HashSet<isize> =
            self.messages.iter().map(|m| m.row).collect();
        let candidates: Vec<isize> = (1..(self.h as isize - 2))
            .filter(|r| !taken.contains(r))
            .collect();
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
        let mut m = Message::new(text, kind_out, url, row, self.w, t, None, 0.0);
        m.domain = domain;
        self.messages.push(m);
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
        if self.wake.is_some() {
            self.tick_wake(t, dt);
            return;
        }
        let quiet = self.picker.is_none() && self.editor.is_none() && !self.show_help;
        if quiet && rand::thread_rng().gen::<f64>() < dt / 3600.0 {
            self.trigger_wake(t);
            return;
        }
        let mult = self.cfg.speed;
        // Fewer concurrent streams on large panes — density still scales, but
        // hard-cap so a full-screen split doesn't spawn 60+ writers.
        let target = 6
            .max((self.h as f64 * self.cfg.density * 1.15) as usize)
            .min(28);
        while self.streams.len() < target && rand::thread_rng().gen::<f64>() < 0.35 {
            let eraser = rand::thread_rng().gen::<f64>() < 0.22;
            self.streams.push(Noise::new(self.h, self.w, eraser));
        }
        for s in &mut self.streams {
            s.update(dt, mult);
        }
        for m in &mut self.messages {
            m.update(t, dt, mult);
        }
        if t >= self.next_msg {
            self.spawn_message(t, None);
            self.next_msg =
                t + self.cfg.message_every_seconds * rand::thread_rng().gen_range(0.7..1.4);
        }
    }

    fn draw(&mut self, t: f64) {
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

        // mount pending summary
        let pending = self.reader_pending.lock().unwrap().take();
        if let Some((tok, art)) = pending {
            self.mount_summary(t, tok, art);
        }

        let rect = if self.show_help || self.editor.is_some() || self.picker.is_some() {
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

            // Shared guard snapshot for this frame (one small clone, not per stream).
            let h = self.h;
            let show_status = self.show_status;
            let panel = self.panel_rect;
            let spans = self.spans.clone();

            for s in &mut self.streams {
                let guard_fn = |row: isize, x: isize| -> bool {
                    if row == h as isize - 1 && show_status {
                        return true;
                    }
                    if let Some((y0, x0, y1, x1)) = panel {
                        if y0 <= row && row <= y1 && x0 <= x && x <= x1 {
                            return true;
                        }
                    }
                    if let Some(ss) = spans.get(&row) {
                        for &(lo, hi) in ss {
                            if lo <= x && x <= hi {
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
                m.draw(&mut self.term, t, &self.pal, &self.glyphs);
            }
            self.messages.retain(|m| !m.done);
        }

        if self.show_status {
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
                .span_cells(self.h as isize - 1, 0, dim, &display);
        }

        let (ref msg, until) = self.toast;
        if !msg.is_empty() && t < until {
            let dim = self.pal.dim;
            let shown = format!(" {msg} ");
            let x = (self.w as isize - shown.chars().count() as isize - 3).max(0);
            self.term
                .span_cells(self.h as isize - 1, x, dim, &shown);
        }

        if self.show_help || self.editor.is_some() || self.picker.is_some() {
            self.draw_panel();
        }
        let _ = self.term.present();
    }

    fn open_summary(&mut self, link: Link, t: f64) {
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
        thread::Builder::new()
            .name("summary".into())
            .spawn(move || {
                let (title, domain, summary, url) =
                    fetch_summary(&api_key, &url, &text, &domain);
                *pending.lock().unwrap() = Some((
                    tok,
                    Article {
                        title,
                        domain,
                        summary,
                        url,
                    },
                ));
            })
            .ok();
    }

    fn mount_summary(&mut self, t: f64, tok: u64, art: Article) {
        if tok != self.reader_req {
            return;
        }
        let width = 30.max((self.w.saturating_sub(16)).min(72));
        let wrapped = textwrap::wrap(&art.summary, width);
        let lines: Vec<_> = wrapped.into_iter().take(6).collect();
        let mut block: Vec<(String, String)> = vec![(
            art.title.chars().take(width).collect(),
            "news".into(),
        )];
        for s in &lines {
            block.push((s.to_string(), "summary".into()));
        }
        let k = block.len();
        let taken: std::collections::HashSet<isize> =
            self.messages.iter().map(|m| m.row).collect();
        let mut r0: Option<isize> = None;
        for _ in 0..30 {
            let max_r = 1.max(self.h as isize - 2 - k as isize);
            let cand = rand::thread_rng().gen_range(1..=max_r.max(1));
            if (0..k).all(|i| !taken.contains(&(cand + i as isize))) {
                r0 = Some(cand);
                break;
            }
        }
        let r0 = r0.unwrap_or_else(|| {
            self.messages
                .retain(|m| !(1 <= m.row && m.row < 1 + k as isize));
            1
        });
        let x0 = 2.max((self.w as isize - width as isize) / 2 + rand::thread_rng().gen_range(-6..=6));
        self.block_seq += 1;
        let group = self.block_seq;
        let dwell = 10.0 + 0.03 * art.summary.len() as f64;
        for (i, (text, kind)) in block.into_iter().enumerate() {
            let url = if kind == "news" {
                Some(art.url.clone())
            } else {
                None
            };
            let mut m = Message::new(
                text,
                kind,
                url,
                r0 + i as isize,
                self.w,
                t,
                Some(x0),
                0.35 * i as f64,
            );
            m.domain = art.domain.clone();
            m.group = Some(group);
            m.dwell = dwell;
            self.messages.push(m);
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
                    let link = Link {
                        text: m.text.clone(),
                        url: url.clone(),
                        domain: m.domain.clone(),
                    };
                    self.open_summary(link, t);
                    return;
                }
            }
        }
    }

    fn panel_lines(&self) -> Vec<String> {
        if self.show_help {
            return HELP.iter().map(|s| s.to_string()).collect();
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
        let blank = self.pal.blank;
        let spaces = " ".repeat(bw);
        for i in 0..=(y1 - y0) {
            self.term.span_cells(y0 + i, x0, blank, &spaces);
        }
        let accent = if let Some(ref ed) = self.editor {
            if ed.kind == "places" {
                self.pal.local
            } else {
                self.pal.poetic
            }
        } else {
            self.pal.news
        };
        let dim = self.pal.dim;
        let lines = self.panel_lines();
        for (i, s) in lines.iter().enumerate() {
            let attr = if i == 0 || s.trim_start().starts_with('▸') {
                accent
            } else {
                dim
            };
            let clipped: String = s.chars().take(bw.saturating_sub(3)).collect();
            self.term
                .span_cells(y0 + 1 + i as isize, x0 + 2, attr, &clipped);
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
        self.picker = None;
        self.panel_rect = None;
        self.term.clear_screen();
    }

    fn flash(&mut self, msg: &str, t: f64) {
        self.toast = (msg.to_string(), t + 2.5);
    }

    fn clear_row(&mut self, row: isize) {
        let blank = self.pal.blank;
        let spaces = " ".repeat(self.w.saturating_sub(1));
        self.term.span_cells(row, 0, blank, &spaces);
    }

    fn handle_bytes(&mut self, data: &[u8], t: f64) -> bool {
        for tok in tokenize(data) {
            match tok {
                Token::Mouse { btn, mx, my, press } => {
                    if press
                        && btn == 0
                        && self.wake.is_none()
                        && self.editor.is_none()
                        && !self.show_help
                    {
                        self.click(mx - 1, my - 1, t);
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
                let link = Link {
                    text: self.shown_links[sel].text.clone(),
                    url: self.shown_links[sel].url.clone(),
                    domain: self.shown_links[sel].domain.clone(),
                };
                self.open_summary(link, t);
                self.close_panel();
            } else if (0x31..=0x39).contains(&b) {
                let idx = (b - 0x30) as usize;
                if idx <= nlinks && idx >= 1 {
                    let link = Link {
                        text: self.shown_links[idx - 1].text.clone(),
                        url: self.shown_links[idx - 1].url.clone(),
                        domain: self.shown_links[idx - 1].domain.clone(),
                    };
                    self.open_summary(link, t);
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

        if self.show_help {
            if b == b'q' || b == 3 {
                return false;
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
            self.spawn_message(t, Some("news"));
        } else if b == b'o' {
            self.spawn_message(t, Some("poetic"));
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
        } else if b == b'+' || b == b'=' {
            self.cfg.speed = (self.cfg.speed * 1.25).min(4.0);
            self.flash(&format!("speed {:.2}x", self.cfg.speed), t);
        } else if b == b'-' {
            self.cfg.speed = (self.cfg.speed / 1.25).max(0.2);
            self.flash(&format!("speed {:.2}x", self.cfg.speed), t);
        } else if b == b'r' {
            self.feed.wake();
            self.flash("refreshing headlines…", t);
        } else if b == b's' {
            self.show_status = !self.show_status;
            if !self.show_status {
                self.clear_row(self.h as isize - 1);
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
        let mut last = monotonic();
        let mut frame = 0u64;
        // Always leave() on every exit path (including panic unwind via Drop).
        let result = (|| {
            loop {
                let t = monotonic();
                let dt = (t - last).min(0.1);
                last = t;
                frame += 1;
                if frame % 90 == 0 {
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
                    self.panel_rect = None;
                    self.term.clear_screen();
                }
                if !self.paused {
                    self.tick(t, dt);
                }
                self.draw(t);
                // Classic animation pace (~12 fps default). Motion is integrated
                // with real dt, so lowering fps only softens the redraw rate.
                let fps = self.cfg.fps.clamp(4.0, 30.0);
                let frame_budget = 1.0 / fps;
                let target = t + frame_budget;
                let timeout = Duration::from_secs_f64((target - monotonic()).max(0.0));
                let data = self.term.read(timeout);
                if !data.is_empty() && !self.handle_bytes(&data, t) {
                    break;
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

fn tokenize(data: &[u8]) -> Vec<Token> {
    let mut toks = Vec::new();
    let mut i = 0;
    let n = data.len();
    while i < n {
        let b = data[i];
        if b == 27 && i + 1 < n && (data[i + 1] == 0x5B || data[i + 1] == 0x4F) {
            let mut j = i + 2;
            while j < n && !(0x40..=0x7E).contains(&data[j]) {
                j += 1;
            }
            let final_b = if j < n { data[j] } else { 0 };
            let params = &data[i + 2..j];
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
            i = j + 1;
        } else {
            toks.push(Token::Key(b));
            i += 1;
        }
    }
    toks
}
