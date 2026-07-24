use libc::{self, termios as Termios, winsize, TCSADRAIN, TIOCGWINSZ};
use std::io::{self, Read, Write};
use std::mem;
use std::os::unix::io::AsRawFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// Style index into the current frame's SGR table (0 = reset/blank).
pub type StyleId = u8;

/// True while the alt screen / raw mode / mouse tracking is active.
static RAW_ACTIVE: AtomicBool = AtomicBool::new(false);

// Stashed for async-signal-safe emergency restore on SIGINT/SIGTERM/SIGQUIT.
static mut EMERGENCY_FD: i32 = -1;
static mut EMERGENCY_SAVED: Option<Termios> = None;

/// Sequences that must always run on exit so the shell never keeps mouse reporting.
const LEAVE_SEQ: &[u8] =
    b"\x1b[?1006l\x1b[?1000l\x1b[?1003l\x1b[?1002l\x1b[0m\x1b[?7h\x1b[?25h\x1b[?1049l";

/// Restore the terminal from a signal handler (or panic path). Only uses
/// async-signal-safe syscalls: `write` + `tcsetattr`.
pub fn emergency_restore() {
    if !RAW_ACTIVE.swap(false, Ordering::SeqCst) {
        return;
    }
    unsafe {
        let _ = libc::write(
            libc::STDOUT_FILENO,
            LEAVE_SEQ.as_ptr() as *const libc::c_void,
            LEAVE_SEQ.len(),
        );
        if let Some(ref saved) = EMERGENCY_SAVED {
            let fd = if EMERGENCY_FD >= 0 {
                EMERGENCY_FD
            } else {
                libc::STDIN_FILENO
            };
            libc::tcsetattr(fd, TCSADRAIN, saved);
        }
        EMERGENCY_SAVED = None;
        EMERGENCY_FD = -1;
    }
}

extern "C" fn on_fatal_signal(sig: i32) {
    emergency_restore();
    // Re-raise with default disposition so the shell sees the right exit status.
    unsafe {
        libc::signal(sig, libc::SIG_DFL);
        libc::raise(sig);
    }
}

fn install_fatal_handlers() {
    unsafe {
        libc::signal(libc::SIGINT, on_fatal_signal as *const () as usize);
        libc::signal(libc::SIGTERM, on_fatal_signal as *const () as usize);
        libc::signal(libc::SIGQUIT, on_fatal_signal as *const () as usize);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Cell {
    style: StyleId,
    ch: char,
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            style: 0,
            ch: ' ',
        }
    }
}

/// Minimal raw-terminal layer with a dirty frame buffer.
///
/// Drawing goes into an off-screen grid; `present` only emits cells that
/// changed since the last frame, batched into horizontal runs. That keeps
/// the PTY quiet enough that WezTerm (and other panes on the same tab)
/// stay responsive.
pub struct Term {
    fd: i32,
    /// Direct escape writes (enter/leave, clear, panels, hyperlinks).
    immediate: Vec<u8>,
    /// Diff output assembled in present().
    out: Vec<u8>,
    pub w: usize,
    pub h: usize,
    saved: Option<Termios>,
    mouse: bool,
    /// Logical width drawn (last column often avoided to skip scroll).
    draw_w: usize,
    cur: Vec<Cell>,
    prev: Vec<Cell>,
    /// SGR sequences indexed by StyleId. styles[0] is always reset.
    styles: Vec<String>,
    /// Force full redraw next present (resize / clear).
    force: bool,
}

impl Term {
    pub fn new() -> io::Result<Self> {
        let fd = io::stdin().as_raw_fd();
        let (w, h) = terminal_size(fd);
        let draw_w = w.saturating_sub(1).max(1);
        let n = draw_w.saturating_mul(h.max(1));
        Ok(Self {
            fd,
            immediate: Vec::with_capacity(4096),
            out: Vec::with_capacity(64 * 1024),
            w,
            h,
            saved: None,
            mouse: false,
            draw_w,
            cur: vec![Cell::default(); n],
            prev: vec![Cell::default(); n],
            styles: vec!["\x1b[0m".into()],
            force: true,
        })
    }

    pub fn enter(&mut self, mouse: bool) -> io::Result<()> {
        let mut saved: Termios = unsafe { mem::zeroed() };
        if unsafe { libc::tcgetattr(self.fd, &mut saved) } != 0 {
            return Err(io::Error::last_os_error());
        }
        self.saved = Some(saved);
        // Stash for signal-handler restore before we touch the terminal.
        unsafe {
            EMERGENCY_FD = self.fd;
            EMERGENCY_SAVED = Some(saved);
        }
        install_fatal_handlers();

        let mut raw = saved;
        // cbreak-ish: no echo, no canonical. Clear ISIG so Ctrl-C is delivered
        // as byte 3 and we can leave() cleanly (default SIGINT would skip Drop
        // and leave mouse tracking on — clicks then type into the shell).
        raw.c_lflag &= !(libc::ECHO | libc::ICANON | libc::ISIG);
        raw.c_cc[libc::VMIN] = 0;
        raw.c_cc[libc::VTIME] = 0;
        if unsafe { libc::tcsetattr(self.fd, TCSADRAIN, &raw) } != 0 {
            return Err(io::Error::last_os_error());
        }
        self.mouse = mouse;
        RAW_ACTIVE.store(true, Ordering::SeqCst);
        self.immediate
            .extend_from_slice(b"\x1b[?1049h\x1b[?25l\x1b[?7l\x1b[2J");
        if mouse {
            // 1000 = click tracking, 1006 = SGR encoding (what WezTerm uses).
            self.immediate.extend_from_slice(b"\x1b[?1000h\x1b[?1006h");
        }
        self.force = true;
        self.flush_immediate()
    }

    pub fn leave(&mut self) {
        if !RAW_ACTIVE.swap(false, Ordering::SeqCst) && self.saved.is_none() {
            return;
        }
        self.immediate.clear();
        self.immediate.extend_from_slice(LEAVE_SEQ);
        let _ = self.flush_immediate();
        if let Some(saved) = self.saved.take() {
            unsafe {
                libc::tcsetattr(self.fd, TCSADRAIN, &saved);
                EMERGENCY_SAVED = None;
                EMERGENCY_FD = -1;
            }
        }
    }

    pub fn resize(&mut self) {
        let (w, h) = terminal_size(self.fd);
        self.w = w;
        self.h = h;
        self.draw_w = w.saturating_sub(1).max(1);
        let n = self.draw_w.saturating_mul(h.max(1));
        self.cur.clear();
        self.cur.resize(n, Cell::default());
        self.prev.clear();
        self.prev.resize(n, Cell::default());
        self.force = true;
    }

    /// Replace the style table for this session. Index 0 must stay reset.
    pub fn set_styles(&mut self, styles: Vec<String>) {
        self.styles = styles;
        if self.styles.is_empty() {
            self.styles.push("\x1b[0m".into());
        }
        self.force = true;
    }

    pub fn styles(&self) -> &[String] {
        &self.styles
    }

    /// Clear the alt screen and mark buffer dirty.
    pub fn clear_screen(&mut self) {
        self.immediate.extend_from_slice(b"\x1b[2J");
        for c in &mut self.cur {
            *c = Cell::default();
        }
        for c in &mut self.prev {
            *c = Cell::default();
        }
        self.force = true;
    }

    #[inline]
    fn idx(&self, y: usize, x: usize) -> Option<usize> {
        if y < self.h && x < self.draw_w {
            Some(y * self.draw_w + x)
        } else {
            None
        }
    }

    /// Plot one cell into the off-screen buffer (no PTY write yet).
    pub fn cell(&mut self, y: isize, x: isize, style: StyleId, ch: char) {
        if y < 0 || x < 0 {
            return;
        }
        if let Some(i) = self.idx(y as usize, x as usize) {
            self.cur[i] = Cell { style, ch };
        }
    }

    /// Plot a run of characters sharing one style.
    pub fn span_cells(&mut self, y: isize, x: isize, style: StyleId, text: &str) {
        if y < 0 {
            return;
        }
        let y = y as usize;
        if y >= self.h {
            return;
        }
        let mut x = x;
        for ch in text.chars() {
            if x < 0 {
                x += 1;
                continue;
            }
            if let Some(i) = self.idx(y, x as usize) {
                self.cur[i] = Cell { style, ch };
            } else {
                break;
            }
            x += 1;
        }
    }

    /// Immediate-mode span for hyperlinks / overlays (writes on next flush).
    pub fn span_immediate(&mut self, y: isize, x: isize, esc: &str, text: &str, url: Option<&str>) {
        if y < 0 || (y as usize) >= self.h || x >= self.w as isize - 1 {
            return;
        }
        let mut text = text.to_string();
        let mut x = x;
        if x < 0 {
            let skip = (-x) as usize;
            if skip >= text.chars().count() {
                return;
            }
            text = text.chars().skip(skip).collect();
            x = 0;
        }
        let max_len = self.w.saturating_sub(1).saturating_sub(x as usize);
        if text.chars().count() > max_len {
            text = text.chars().take(max_len).collect();
        }
        if text.is_empty() {
            return;
        }
        // Also stamp into the buffer so the next diff doesn't clobber the link.
        // Style 0 + content — present() will skip if we mark prev to match after.
        // Better: write immediate and set prev+cur so diff ignores.
        let style0 = 0u8;
        let mut cx = x;
        for ch in text.chars() {
            if let Some(i) = self.idx(y as usize, cx as usize) {
                let cell = Cell {
                    style: style0,
                    ch,
                };
                self.cur[i] = cell;
                self.prev[i] = cell; // already "on screen" via immediate write
            }
            cx += 1;
        }

        write_cup(&mut self.immediate, y + 1, x + 1);
        self.immediate.extend_from_slice(esc.as_bytes());
        if let Some(u) = url {
            self.immediate.extend_from_slice(b"\x1b]8;;");
            self.immediate.extend_from_slice(u.as_bytes());
            self.immediate.extend_from_slice(b"\x1b\\");
            self.immediate.extend_from_slice(text.as_bytes());
            self.immediate.extend_from_slice(b"\x1b]8;;\x1b\\");
        } else {
            self.immediate.extend_from_slice(text.as_bytes());
        }
    }

    /// Emit only changed cells, wrapped in a synchronized-update pair when supported.
    /// If nothing changed and there are no immediate writes, this is a pure no-op.
    pub fn present(&mut self) -> io::Result<()> {
        let force = self.force;
        self.force = false;
        let has_immediate = !self.immediate.is_empty();

        // Fast path: no force, no overlays — scan for any dirty cell first.
        if !force && !has_immediate {
            let mut dirty = false;
            for (c, p) in self.cur.iter().zip(self.prev.iter()) {
                if c != p {
                    dirty = true;
                    break;
                }
            }
            if !dirty {
                return Ok(());
            }
        }

        self.out.clear();
        // Begin synchronized update (WezTerm, kitty, foot, ghostty, …)
        self.out.extend_from_slice(b"\x1b[?2026h");

        let h = self.h;
        let dw = self.draw_w;
        let n_styles = self.styles.len();

        for y in 0..h {
            let row = y * dw;
            let mut x = 0;
            while x < dw {
                let i = row + x;
                let cell = self.cur[i];
                if !force && cell == self.prev[i] {
                    x += 1;
                    continue;
                }
                // Start a run of dirty cells.
                let run_start = x;
                x += 1;
                while x < dw {
                    let j = row + x;
                    let c = self.cur[j];
                    if !force && c == self.prev[j] {
                        break;
                    }
                    x += 1;
                }
                // Emit the run.
                write_cup(&mut self.out, (y + 1) as isize, (run_start + 1) as isize);
                let mut cur_style = 255u8; // impossible
                for rx in run_start..x {
                    let c = self.cur[row + rx];
                    if c.style != cur_style {
                        let sid = c.style as usize;
                        if sid < n_styles {
                            self.out.extend_from_slice(self.styles[sid].as_bytes());
                        } else {
                            self.out.extend_from_slice(self.styles[0].as_bytes());
                        }
                        cur_style = c.style;
                    }
                    let mut buf = [0u8; 4];
                    let s = c.ch.encode_utf8(&mut buf);
                    self.out.extend_from_slice(s.as_bytes());
                    self.prev[row + rx] = c;
                }
            }
        }

        self.out.extend_from_slice(b"\x1b[?2026l");

        // Immediate overlays (panels, hyperlinks, raw clears) after the field.
        if !self.immediate.is_empty() {
            self.out.extend_from_slice(&self.immediate);
            self.immediate.clear();
        }

        // Always more than the empty sync pair if we got here with work to do.
        let mut stdout = io::stdout().lock();
        stdout.write_all(&self.out)?;
        stdout.flush()?;
        Ok(())
    }

    fn flush_immediate(&mut self) -> io::Result<()> {
        if !self.immediate.is_empty() {
            let mut stdout = io::stdout().lock();
            stdout.write_all(&self.immediate)?;
            stdout.flush()?;
            self.immediate.clear();
        }
        Ok(())
    }

    pub fn read(&mut self, timeout: Duration) -> Vec<u8> {
        let mut fds = libc::pollfd {
            fd: self.fd,
            events: libc::POLLIN,
            revents: 0,
        };
        let ms = timeout.as_millis().min(i32::MAX as u128) as i32;
        let n = unsafe { libc::poll(&mut fds, 1, ms.max(0)) };
        if n <= 0 {
            return Vec::new();
        }
        let mut buf = [0u8; 256];
        match io::stdin().read(&mut buf) {
            Ok(n) if n > 0 => buf[..n].to_vec(),
            _ => Vec::new(),
        }
    }
}

impl Drop for Term {
    fn drop(&mut self) {
        self.leave();
    }
}

fn write_cup(buf: &mut Vec<u8>, row: isize, col: isize) {
    // \x1b[row;colH without format! allocation
    buf.push(0x1b);
    buf.push(b'[');
    push_u32(buf, row.max(1) as u32);
    buf.push(b';');
    push_u32(buf, col.max(1) as u32);
    buf.push(b'H');
}

fn push_u32(buf: &mut Vec<u8>, mut n: u32) {
    let mut tmp = [0u8; 10];
    let mut i = 10;
    if n == 0 {
        buf.push(b'0');
        return;
    }
    while n > 0 {
        i -= 1;
        tmp[i] = b'0' + (n % 10) as u8;
        n /= 10;
    }
    buf.extend_from_slice(&tmp[i..]);
}

fn terminal_size(fd: i32) -> (usize, usize) {
    let mut ws: winsize = unsafe { mem::zeroed() };
    if unsafe { libc::ioctl(fd, TIOCGWINSZ, &mut ws) } == 0 && ws.ws_col > 0 && ws.ws_row > 0 {
        (ws.ws_col as usize, ws.ws_row as usize)
    } else {
        (80, 24)
    }
}

pub fn is_tty() -> bool {
    unsafe {
        libc::isatty(io::stdin().as_raw_fd()) != 0 && libc::isatty(io::stdout().as_raw_fd()) != 0
    }
}
