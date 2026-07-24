use crate::theme::{Palette, SCRAMBLE};
use crate::term::Term;
use rand::Rng;

pub fn glyph_at(row: isize, x: isize, salt: i32, glyphs: &str) -> String {
    let chars: Vec<char> = glyphs.chars().collect();
    let len = chars.len() as i64;
    if len == 0 {
        return " ".into();
    }
    let idx = ((x as i64)
        .wrapping_mul(73856093)
        ^ (row as i64).wrapping_mul(19349663)
        ^ (salt as i64).wrapping_mul(83492791))
    .rem_euclid(len) as usize;
    chars[idx].to_string()
}

/// (esc, glyph) for the settled code field at a cell; evolves slowly.
pub fn residue_at(
    row: isize,
    x: isize,
    t: f64,
    glyphs: &str,
    pal: &Palette,
) -> (String, String) {
    let rh = ((x as i64)
        .wrapping_mul(2654435761u32 as i64)
        ^ (row as i64).wrapping_mul(40503)
        ^ ((t / 7.0) as i64).wrapping_mul(69069))
        & 0xFFFFFF;
    if rh % 100 < 22 {
        return (pal.blank.clone(), " ".into());
    }
    let esc = pal.residue[(rh as usize) % pal.residue.len()].clone();
    let ch = glyph_at(row, x, ((rh >> 8) & 0xFF) as i32, glyphs);
    (esc, ch)
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
        glyphs: &str,
        mut guard: impl FnMut(isize, isize) -> bool,
    ) {
        let hi = self.head as i32;
        let tail = hi - self.length + 1;
        if self.eraser {
            for x in (self.last_head + 1)..=hi {
                if !guard(self.row, x as isize) {
                    term.cell(self.row, x as isize, &pal.blank, " ");
                }
            }
            for x in self.last_tail..tail {
                if !guard(self.row, x as isize) {
                    let (esc, ch) = residue_at(self.row, x as isize, t, glyphs, pal);
                    term.cell(self.row, x as isize, &esc, &ch);
                }
            }
            self.last_head = hi;
            self.last_tail = self.last_tail.max(tail);
            return;
        }
        for x in self.last_tail..tail {
            if !guard(self.row, x as isize) {
                let (esc, ch) = residue_at(self.row, x as isize, t, glyphs, pal);
                term.cell(self.row, x as isize, &esc, &ch);
            }
        }
        self.last_head = hi;
        self.last_tail = self.last_tail.max(tail);
        for d in 0..self.length {
            let x = hi - d;
            if guard(self.row, x as isize) {
                continue;
            }
            let salt = (t * 2.5
                + ((x * 7 + self.row as i32 * 13) % 23) as f64 / 23.0)
                as i32;
            let ch = glyph_at(self.row, x as isize, salt, glyphs);
            if d == 0 {
                term.cell(self.row, x as isize, &pal.head, &ch);
            } else {
                let band = (((d as f64 / self.length as f64) * pal.trail.len() as f64) as usize)
                    .min(pal.trail.len() - 1);
                term.cell(self.row, x as isize, &pal.trail[band], &ch);
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

    pub fn draw(&self, term: &mut Term, t: f64, pal: &Palette, glyphs: &str) {
        let attr = match self.kind.as_str() {
            "news" => &pal.news,
            "local" => &pal.local,
            "summary" => &pal.reader,
            _ => &pal.poetic,
        };
        let chars: Vec<char> = self.text.chars().collect();
        let n = chars.len() as i32;

        match self.phase {
            Phase::Reveal => {
                let locked = (self.head as i32 - SCRAMBLE).clamp(0, n);
                if locked > 0 {
                    let text: String = chars[..locked as usize].iter().collect();
                    term.span(
                        self.row,
                        self.x0 - 1,
                        attr,
                        &format!(" {text}"),
                        self.url.as_deref(),
                    );
                }
                let start = locked.max(0);
                let end = (self.head as i32).min(n);
                for i in start..end {
                    let g = glyph_at(self.row, self.x0 + i as isize, (t * 12.0) as i32, glyphs);
                    term.cell(self.row, self.x0 + i as isize, &pal.scramble, &g);
                }
            }
            Phase::Dwell => {
                term.span(
                    self.row,
                    self.x0 - 1,
                    attr,
                    &format!(" {} ", self.text),
                    self.url.as_deref(),
                );
            }
            Phase::Erase => {
                let gone = (self.erase as i32 - SCRAMBLE).clamp(0, n);
                let end_x = self.x0 + gone as isize + if gone == n { 2 } else { 0 };
                for x in (self.x0 - 1)..end_x {
                    let (esc, ch) = residue_at(self.row, x, t, glyphs, pal);
                    term.cell(self.row, x, &esc, &ch);
                }
                let start = gone.max(0);
                let end = (self.erase as i32).min(n);
                for i in start..end {
                    let g = glyph_at(self.row, self.x0 + i as isize, (t * 12.0) as i32, glyphs);
                    term.cell(
                        self.row,
                        self.x0 + i as isize,
                        pal.trail.last().unwrap_or(&pal.dim),
                        &g,
                    );
                }
                if (self.erase as i32) < n {
                    let start = (self.erase as i32).max(0) as usize;
                    let rest: String = chars[start..].iter().collect();
                    term.span(
                        self.row,
                        self.x0 + start as isize,
                        attr,
                        &format!("{rest} "),
                        self.url.as_deref(),
                    );
                }
            }
        }
    }
}
