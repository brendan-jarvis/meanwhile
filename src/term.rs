use libc::{self, termios as Termios, winsize, TCSADRAIN, TIOCGWINSZ};
use std::io::{self, Read, Write};
use std::mem;
use std::os::unix::io::AsRawFd;
use std::time::Duration;

/// Minimal raw-terminal layer: alt screen, cbreak keys, buffered writes.
pub struct Term {
    fd: i32,
    buf: String,
    pub w: usize,
    pub h: usize,
    saved: Option<Termios>,
    mouse: bool,
}

impl Term {
    pub fn new() -> io::Result<Self> {
        let fd = io::stdin().as_raw_fd();
        let (w, h) = terminal_size(fd);
        Ok(Self {
            fd,
            buf: String::with_capacity(64 * 1024),
            w,
            h,
            saved: None,
            mouse: false,
        })
    }

    pub fn enter(&mut self, mouse: bool) -> io::Result<()> {
        let mut saved: Termios = unsafe { mem::zeroed() };
        if unsafe { libc::tcgetattr(self.fd, &mut saved) } != 0 {
            return Err(io::Error::last_os_error());
        }
        self.saved = Some(saved);
        let mut raw = saved;
        // cbreak-ish: no echo, no canonical, keep signals
        raw.c_lflag &= !(libc::ECHO | libc::ICANON);
        raw.c_cc[libc::VMIN] = 0;
        raw.c_cc[libc::VTIME] = 0;
        if unsafe { libc::tcsetattr(self.fd, TCSADRAIN, &raw) } != 0 {
            return Err(io::Error::last_os_error());
        }
        self.mouse = mouse;
        self.out("\x1b[?1049h\x1b[?25l\x1b[?7l\x1b[2J");
        if mouse {
            self.out("\x1b[?1000h\x1b[?1006h");
        }
        self.flush()
    }

    pub fn leave(&mut self) {
        self.buf.clear();
        self.out("\x1b[?1006l\x1b[?1000l\x1b[0m\x1b[?7h\x1b[?25h\x1b[?1049l");
        let _ = self.flush();
        if let Some(saved) = self.saved.take() {
            unsafe {
                libc::tcsetattr(self.fd, TCSADRAIN, &saved);
            }
        }
    }

    pub fn resize(&mut self) {
        let (w, h) = terminal_size(self.fd);
        self.w = w;
        self.h = h;
    }

    pub fn out(&mut self, s: &str) {
        self.buf.push_str(s);
    }

    pub fn cell(&mut self, y: isize, x: isize, esc: &str, ch: &str) {
        if y >= 0 && (y as usize) < self.h && x >= 0 && (x as usize) < self.w.saturating_sub(1) {
            self.buf.push_str(&format!(
                "\x1b[{};{}H{}{}",
                y + 1,
                x + 1,
                esc,
                ch
            ));
        }
    }

    pub fn span(&mut self, y: isize, x: isize, esc: &str, text: &str, url: Option<&str>) {
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
        // clip by display width roughly (char count; fine for our glyphs)
        if text.chars().count() > max_len {
            text = text.chars().take(max_len).collect();
        }
        if text.is_empty() {
            return;
        }
        if let Some(u) = url {
            text = format!("\x1b]8;;{u}\x1b\\{text}\x1b]8;;\x1b\\");
        }
        self.buf
            .push_str(&format!("\x1b[{};{}H{}{}", y + 1, x + 1, esc, text));
    }

    pub fn flush(&mut self) -> io::Result<()> {
        if !self.buf.is_empty() {
            let mut out = io::stdout();
            out.write_all(self.buf.as_bytes())?;
            out.flush()?;
            self.buf.clear();
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

fn terminal_size(fd: i32) -> (usize, usize) {
    let mut ws: winsize = unsafe { mem::zeroed() };
    if unsafe { libc::ioctl(fd, TIOCGWINSZ, &mut ws) } == 0 && ws.ws_col > 0 && ws.ws_row > 0 {
        (ws.ws_col as usize, ws.ws_row as usize)
    } else {
        (80, 24)
    }
}

pub fn is_tty() -> bool {
    unsafe { libc::isatty(io::stdin().as_raw_fd()) != 0 && libc::isatty(io::stdout().as_raw_fd()) != 0 }
}
