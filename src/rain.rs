use crate::term::{StyleId, Term};
use crate::theme::{Palette, SCRAMBLE};
use rand::Rng;

/// Cached glyph table — avoid re-collecting chars on every lookup.
pub struct Glyphs {
    chars: Vec<char>,
}

impl Glyphs {
    pub fn new(s: &str) -> Self {
        Self {
            chars: s.chars().collect(),
        }
    }
}

#[inline]
pub fn glyph_at(row: isize, x: isize, salt: i32, glyphs: &Glyphs) -> char {
    let len = glyphs.chars.len() as i64;
    if len == 0 {
        return ' ';
    }
    let idx = ((x as i64)
        .wrapping_mul(73856093)
        ^ (row as i64).wrapping_mul(19349663)
        ^ (salt as i64).wrapping_mul(83492791))
    .rem_euclid(len) as usize;
    glyphs.chars[idx]
}

/// (style, glyph) for the settled code field at a cell; evolves slowly.
pub fn residue_at(
    row: isize,
    x: isize,
    t: f64,
    glyphs: &Glyphs,
    pal: &Palette,
) -> (StyleId, char) {
    let rh = ((x as i64)
        .wrapping_mul(2654435761u32 as i64)
        ^ (row as i64).wrapping_mul(40503)
        ^ ((t / 7.0) as i64).wrapping_mul(69069))
        & 0xFFFFFF;
    if rh % 100 < 22 {
        return (pal.blank, ' ');
    }
    let style = pal.residue[(rh as usize) % pal.residue.len()];
    let ch = glyph_at(row, x, ((rh >> 8) & 0xFF) as i32, glyphs);
    (style, ch)
}

/// A stream sweeping left to right. Writers leave settled code behind;
/// erasers carve a moving window of darkness that heals after them.
pub struct Noise {
    pub row: isize,
    eraser: bool,
    head: f64,
    speed: f64,
    length: i32,
    last_head: i32,
    last_tail: i32,
    /// Only shimmer trail glyphs when this ticks (cuts PTY traffic).
    shimmer_phase: u8,
}

impl Noise {
    pub fn new(h: usize, w: usize, eraser: bool) -> Self {
        let mut rng = rand::thread_rng();
        let head = -rng.gen_range(0.0..12.0);
        let length = rng.gen_range(8..=(10.max((w / 3).min(34)).max(10))) as i32;
        Self {
            row: rng.gen_range(0..h.max(1)) as isize,
            eraser,
            head,
            speed: if eraser {
                rng.gen_range(25.0..60.0)
            } else {
                rng.gen_range(15.0..45.0)
            },
            length,
            last_head: head as i32,
            last_tail: head as i32 - length,
            shimmer_phase: rng.gen_range(0..3),
        }
    }

    pub fn update(&mut self, dt: f64, mult: f64) {
        self.head += self.speed * mult * dt;
    }

    pub fn dead(&self, w: usize) -> bool {
        self.head - self.length as f64 > w as f64
    }

    pub fn draw(
        &mut self,
        term: &mut Term,
        t: f64,
        pal: &Palette,
        glyphs: &Glyphs,
        mut guard: impl FnMut(isize, isize) -> bool,
    ) {
        let hi = self.head as i32;
        let tail = hi - self.length + 1;
        let head_moved = hi != self.last_head;

        if self.eraser {
            for x in (self.last_head + 1)..=hi {
                if !guard(self.row, x as isize) {
                    term.cell(self.row, x as isize, pal.blank, ' ');
                }
            }
            for x in self.last_tail..tail {
                if !guard(self.row, x as isize) {
                    let (style, ch) = residue_at(self.row, x as isize, t, glyphs, pal);
                    term.cell(self.row, x as isize, style, ch);
                }
            }
            self.last_head = hi;
            self.last_tail = self.last_tail.max(tail);
            return;
        }

        // Settle cells that just left the trail.
        for x in self.last_tail..tail {
            if !guard(self.row, x as isize) {
                let (style, ch) = residue_at(self.row, x as isize, t, glyphs, pal);
                term.cell(self.row, x as isize, style, ch);
            }
        }
        self.last_head = hi;
        self.last_tail = self.last_tail.max(tail);

        // When the head advances, paint the whole trail once. Between advances
        // only the bright head cell updates — at ambient fps that still reads
        // as living rain without rewriting every stream body every frame.
        self.shimmer_phase = self.shimmer_phase.wrapping_add(1);
        let redraw_len = if head_moved {
            self.length
        } else if self.shimmer_phase % 4 == 0 {
            1 // head sparkle only
        } else {
            0
        };

        // Quantize time so glyph salt holds steady for a few frames (dirty buffer
        // then sees identical cells and emits nothing).
        let salt_t = (t * 0.6) as i32;
        for d in 0..redraw_len {
            let x = hi - d;
            if guard(self.row, x as isize) {
                continue;
            }
            let salt = salt_t + ((x * 7 + self.row as i32 * 13) % 23);
            let ch = glyph_at(self.row, x as isize, salt, glyphs);
            if d == 0 {
                term.cell(self.row, x as isize, pal.head, ch);
            } else {
                let band = (((d as f64 / self.length as f64) * pal.trail.len() as f64) as usize)
                    .min(pal.trail.len() - 1);
                term.cell(self.row, x as isize, pal.trail[band], ch);
            }
        }
    }
}

/// A headline or poetic line that decodes out of the field, lingers, dissolves.
pub struct Message {
    pub text: String,
    pub kind: String,
    pub url: Option<String>,
    pub row: isize,
    pub domain: String,
    pub group: Option<u64>,
    pub x0: isize,
    speed: f64,
    phase: Phase,
    phase_start: f64,
    head: f64,
    erase: f64,
    pub dwell: f64,
    pub done: bool,
    /// Avoid re-emitting OSC 8 hyperlinks every frame while dwelling.
    link_painted: bool,
    /// Last integer reveal/erase head so we only repaint when it advances.
    last_drawn_head: i32,
}

#[derive(Clone, Copy, PartialEq)]
enum Phase {
    Reveal,
    Dwell,
    Erase,
}

impl Message {
    pub fn new(
        text: String,
        kind: String,
        url: Option<String>,
        row: isize,
        width: usize,
        t: f64,
        x0: Option<isize>,
        delay: f64,
    ) -> Self {
        let mut rng = rand::thread_rng();
        let text_len = text.chars().count();
        let max_x = 1.max(width.saturating_sub(text_len).saturating_sub(2));
        let x0 = x0.unwrap_or_else(|| rng.gen_range(1..=max_x.max(1)) as isize);
        let x0 = 1.max(x0.min(max_x as isize));
        let speed = rng.gen_range(32.0..48.0);
        Self {
            text,
            kind,
            url,
            row,
            domain: String::new(),
            group: None,
            x0,
            speed,
            phase: Phase::Reveal,
            phase_start: t,
            head: -delay * speed,
            erase: 0.0,
            dwell: 4.0 + 0.055 * text_len as f64,
            done: false,
            link_painted: false,
            last_drawn_head: i32::MIN,
        }
    }

    pub fn span_range(&self) -> (isize, isize) {
        let n = self.text.chars().count() as isize;
        (self.x0 - 1, self.x0 + n + 1)
    }

    pub fn update(&mut self, t: f64, dt: f64, mult: f64) {
        let n = self.text.chars().count() as f64;
        match self.phase {
            Phase::Reveal => {
                self.head += self.speed * mult * dt;
                if self.head >= n + SCRAMBLE as f64 {
                    self.phase = Phase::Dwell;
                    self.phase_start = t;
                }
            }
            Phase::Dwell => {
                if t - self.phase_start >= self.dwell {
                    self.phase = Phase::Erase;
                }
            }
            Phase::Erase => {
                self.erase += self.speed * 1.6 * mult * dt;
                if self.erase >= n + SCRAMBLE as f64 {
                    self.done = true;
                }
            }
        }
    }

    pub fn force_erase(&mut self) {
        self.phase = Phase::Erase;
    }

    pub fn is_erasing(&self) -> bool {
        self.phase == Phase::Erase
    }

    pub fn draw(&mut self, term: &mut Term, t: f64, pal: &Palette, glyphs: &Glyphs) {
        let attr = match self.kind.as_str() {
            "news" => pal.news,
            "local" => pal.local,
            "summary" => pal.reader,
            _ => pal.poetic,
        };
        let chars: Vec<char> = self.text.chars().collect();
        let n = chars.len() as i32;
        // Quantized salt so scramble cells stay dirty less often.
        let salt = (t * 4.0) as i32;

        match self.phase {
            Phase::Reveal => {
                let head_i = self.head as i32;
                self.last_drawn_head = head_i;
                self.link_painted = false;

                let locked = (head_i - SCRAMBLE).clamp(0, n);
                if locked > 0 {
                    let text: String = chars[..locked as usize].iter().collect();
                    // Buffer path (deduped). OSC 8 applied once we enter dwell.
                    term.span_cells(self.row, self.x0 - 1, attr, &format!(" {text}"));
                }
                let start = locked.max(0);
                let end = head_i.min(n);
                for i in start..end {
                    let g = glyph_at(self.row, self.x0 + i as isize, salt, glyphs);
                    term.cell(self.row, self.x0 + i as isize, pal.scramble, g);
                }
            }
            Phase::Dwell => {
                let shown = format!(" {} ", self.text);
                // Frame-buffer paint (stable → present() no-ops after first frame).
                term.span_cells(self.row, self.x0 - 1, attr, &shown);
                // OSC 8 once so shift-click still opens the article.
                if self.url.is_some() && !self.link_painted {
                    let attr_sgr = term
                        .styles()
                        .get(attr as usize)
                        .cloned()
                        .unwrap_or_else(|| "\x1b[0m".into());
                    term.span_immediate(
                        self.row,
                        self.x0 - 1,
                        &attr_sgr,
                        &shown,
                        self.url.as_deref(),
                    );
                    self.link_painted = true;
                }
            }
            Phase::Erase => {
                self.link_painted = false;
                let erase_i = self.erase as i32;
                self.last_drawn_head = erase_i;

                let gone = (erase_i - SCRAMBLE).clamp(0, n);
                let end_x = self.x0 + gone as isize + if gone == n { 2 } else { 0 };
                for x in (self.x0 - 1)..end_x {
                    let (style, ch) = residue_at(self.row, x, t, glyphs, pal);
                    term.cell(self.row, x, style, ch);
                }
                let start = gone.max(0);
                let end = erase_i.min(n);
                let trail_style = *pal.trail.last().unwrap_or(&pal.dim);
                for i in start..end {
                    let g = glyph_at(self.row, self.x0 + i as isize, salt, glyphs);
                    term.cell(self.row, self.x0 + i as isize, trail_style, g);
                }
                if erase_i < n {
                    let start = erase_i.max(0) as usize;
                    let rest: String = chars[start..].iter().collect();
                    term.span_cells(
                        self.row,
                        self.x0 + start as isize,
                        attr,
                        &format!("{rest} "),
                    );
                }
            }
        }
    }
}
