use crate::term::StyleId;
use serde::Deserialize;
use std::fs;
use std::io::{Read, Write};
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

pub fn sgr(codes: &[u32]) -> String {
    let parts: Vec<String> = codes.iter().map(|c| c.to_string()).collect();
    format!("\x1b[{}m", parts.join(";"))
}

fn hex_rgb(s: &str) -> Option<(u8, u8, u8)> {
    let s = s.trim().trim_start_matches('#').trim_matches('"').trim_matches('\'');
    if s.len() < 6 {
        return None;
    }
    let r = u8::from_str_radix(&s[0..2], 16).ok()?;
    let g = u8::from_str_radix(&s[2..4], 16).ok()?;
    let b = u8::from_str_radix(&s[4..6], 16).ok()?;
    Some((r, g, b))
}

fn mix(a: (u8, u8, u8), b: (u8, u8, u8), f: f64) -> (u8, u8, u8) {
    (
        (a.0 as f64 + (b.0 as f64 - a.0 as f64) * f).round() as u8,
        (a.1 as f64 + (b.1 as f64 - a.1 as f64) * f).round() as u8,
        (a.2 as f64 + (b.2 as f64 - a.2 as f64) * f).round() as u8,
    )
}

fn fg(rgb: (u8, u8, u8), pre: &[u32]) -> String {
    let mut codes = vec![0];
    codes.extend_from_slice(pre);
    codes.extend_from_slice(&[38, 2, rgb.0 as u32, rgb.1 as u32, rgb.2 as u32]);
    sgr(&codes)
}

/// Resolved terminal palette used to build the rain colors.
#[derive(Debug, Clone, PartialEq)]
pub struct ThemeColors {
    pub name: String,
    pub bg: (u8, u8, u8),
    pub fg: (u8, u8, u8),
    pub green: (u8, u8, u8),
    pub bgreen: (u8, u8, u8),
    pub yellow: (u8, u8, u8),
    pub cyan: (u8, u8, u8),
    pub bwhite: (u8, u8, u8),
}

// ---------------------------------------------------------------------------
// auto: live terminal → WezTerm config → Starship → Omarchy → none
// ---------------------------------------------------------------------------

/// File-based auto theme (no TTY queries). Prefer [`query_terminal_theme`]
/// when a real terminal is available — that inherits the *active* scheme.
pub fn load_auto_theme() -> Option<ThemeColors> {
    load_wezterm_colors()
        .or_else(load_starship_colors)
        .or_else(load_omarchy_colors)
}

/// Paths we watch for live theme reloads.
pub fn theme_sources_mtime() -> Option<SystemTime> {
    let mut best: Option<SystemTime> = None;
    for p in theme_watch_paths() {
        if let Ok(meta) = fs::metadata(&p) {
            if let Ok(m) = meta.modified() {
                best = Some(match best {
                    Some(b) if b >= m => b,
                    _ => m,
                });
            }
        }
        // also watch symlink target for Omarchy-style current/theme
        if let Ok(meta) = fs::symlink_metadata(&p) {
            if let Ok(m) = meta.modified() {
                best = Some(match best {
                    Some(b) if b >= m => b,
                    _ => m,
                });
            }
        }
    }
    best
}

fn theme_watch_paths() -> Vec<PathBuf> {
    let mut paths = wezterm_config_candidates();
    paths.push(starship_path());
    paths.push(omarchy_theme_path());
    paths.push(omarchy_theme_path().join("alacritty.toml"));
    paths
}

// ---------------------------------------------------------------------------
// OSC 10/11/4 — ask the terminal for its active palette (WezTerm, kitty, …)
// ---------------------------------------------------------------------------

/// Query the active terminal palette via OSC. Must be called with the TTY
/// already in non-canonical mode (after `Term::enter`).
pub fn query_terminal_theme() -> Option<ThemeColors> {
    // Drain any pending input first so we don't confuse mouse bytes with replies.
    drain_stdin(Duration::from_millis(5));

    let bg = query_osc("11")?;
    let fg = query_osc("10").unwrap_or((0xcd, 0xd6, 0xf4));
    // ANSI: 2=green, 10=bright green, 3=yellow, 6=cyan, 7=white, 15=bright white
    let green = query_osc("4;2").unwrap_or(fg);
    let bgreen = query_osc("4;10").unwrap_or(green);
    let yellow = query_osc("4;3").unwrap_or(fg);
    let cyan = query_osc("4;6").unwrap_or(fg);
    let bwhite = query_osc("4;15").unwrap_or(fg);

    Some(ThemeColors {
        name: terminal_theme_name(),
        bg,
        fg,
        green,
        bgreen,
        yellow,
        cyan,
        bwhite,
    })
}

fn terminal_theme_name() -> String {
    if let Ok(prog) = std::env::var("TERM_PROGRAM") {
        if !prog.is_empty() {
            return format!("{prog} terminal");
        }
    }
    "terminal".into()
}

fn query_osc(param: &str) -> Option<(u8, u8, u8)> {
    let mut out = std::io::stdout();
    // ST-terminated query (WezTerm, xterm, kitty)
    let q = format!("\x1b]{param}?\x1b\\");
    out.write_all(q.as_bytes()).ok()?;
    out.flush().ok()?;

    let reply = read_osc_reply(Duration::from_millis(120))?;
    parse_osc_rgb(&reply)
}

fn drain_stdin(budget: Duration) {
    let deadline = std::time::Instant::now() + budget;
    let mut buf = [0u8; 256];
    while std::time::Instant::now() < deadline {
        let mut fds = libc::pollfd {
            fd: std::io::stdin().as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let n = unsafe { libc::poll(&mut fds, 1, 0) };
        if n <= 0 {
            break;
        }
        let _ = std::io::stdin().read(&mut buf);
    }
}

fn read_osc_reply(timeout: Duration) -> Option<Vec<u8>> {
    let fd = std::io::stdin().as_raw_fd();
    let mut acc = Vec::with_capacity(128);
    let deadline = std::time::Instant::now() + timeout;
    let mut buf = [0u8; 256];

    while std::time::Instant::now() < deadline {
        let remain = deadline.saturating_duration_since(std::time::Instant::now());
        let mut fds = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        let ms = remain.as_millis().min(i32::MAX as u128) as i32;
        let n = unsafe { libc::poll(&mut fds, 1, ms.max(0)) };
        if n <= 0 {
            continue;
        }
        match std::io::stdin().read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                acc.extend_from_slice(&buf[..n]);
                // BEL or ST ends the reply
                if acc.contains(&0x07)
                    || acc.windows(2).any(|w| w == b"\x1b\\")
                    || acc.windows(2).any(|w| w == [0x1b, b'\\'])
                {
                    return Some(acc);
                }
                // also accept if we clearly have rgb: already and a terminator-ish length
                if acc.windows(4).any(|w| w == b"rgb:") && acc.len() > 20 {
                    // wait a tiny bit more for terminator
                    if acc.len() > 40 {
                        return Some(acc);
                    }
                }
            }
            Err(_) => break,
        }
    }
    if acc.is_empty() {
        None
    } else {
        Some(acc)
    }
}

/// Parse `rgb:RRRR/GGGG/BBBB` (or 2-digit) out of an OSC reply.
fn parse_osc_rgb(raw: &[u8]) -> Option<(u8, u8, u8)> {
    let s = String::from_utf8_lossy(raw);
    let idx = s.find("rgb:")?;
    let rest = &s[idx + 4..];
    let end = rest
        .find(|c: char| c == '\x07' || c == '\x1b' || c == ';' || c.is_whitespace())
        .unwrap_or(rest.len());
    let body = &rest[..end];
    let mut parts = body.split('/');
    let r = parse_osc_component(parts.next()?)?;
    let g = parse_osc_component(parts.next()?)?;
    let b = parse_osc_component(parts.next()?)?;
    Some((r, g, b))
}

fn parse_osc_component(s: &str) -> Option<u8> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    // 4-digit: take high byte; 2-digit: as-is
    if s.len() >= 4 {
        u8::from_str_radix(&s[..2], 16).ok()
    } else if s.len() >= 2 {
        u8::from_str_radix(&s[..2], 16).ok()
    } else {
        u8::from_str_radix(s, 16).ok()
    }
}

// ---------------------------------------------------------------------------
// WezTerm config
// ---------------------------------------------------------------------------

fn wezterm_config_candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    for p in [
        home.join(".wezterm.lua"),
        home.join(".config/wezterm/wezterm.lua"),
        home.join("config/wezterm/wezterm.lua"),
        home.join("repos/config/wezterm/wezterm.lua"),
        PathBuf::from("/mnt/c/Users")
            .join(std::env::var("USER").unwrap_or_default())
            .join(".wezterm.lua"),
    ] {
        if p.is_file() {
            out.push(p);
        }
    }
    // Windows user home when USER differs (common: WSL user ≠ Windows user)
    if let Ok(entries) = fs::read_dir("/mnt/c/Users") {
        for e in entries.flatten() {
            let p = e.path().join(".wezterm.lua");
            if p.is_file() && !out.contains(&p) {
                out.push(p);
            }
            let p2 = e.path().join(".config/wezterm/wezterm.lua");
            if p2.is_file() && !out.contains(&p2) {
                out.push(p2);
            }
        }
    }
    out
}

fn load_wezterm_colors() -> Option<ThemeColors> {
    for path in wezterm_config_candidates() {
        if let Some(c) = parse_wezterm_lua(&path) {
            return Some(c);
        }
    }
    None
}

fn parse_wezterm_lua(path: &Path) -> Option<ThemeColors> {
    let text = fs::read_to_string(path).ok()?;

    // Prefer an explicit embedded palette table (matches your mocha = { ... }).
    if let Some(c) = palette_from_lua_hex_table(&text) {
        return Some(c);
    }

    // color_scheme = "Catppuccin Mocha"
    let scheme = text.lines().find_map(|line| {
        let line = line.trim();
        if line.starts_with("--") {
            return None;
        }
        let key = "color_scheme";
        let i = line.find(key)?;
        let rest = &line[i + key.len()..];
        let rest = rest.trim_start().trim_start_matches('=').trim_start();
        let quote = rest.chars().next()?;
        if quote != '"' && quote != '\'' {
            return None;
        }
        let body = rest[1..].split(quote).next()?;
        Some(body.to_string())
    });

    if let Some(name) = scheme {
        if let Some(c) = known_color_scheme(&name) {
            return Some(c);
        }
    }
    None
}

/// Pull bg/fg/green/yellow/cyan from a Lua table of `name = "#rrggbb"` pairs.
fn palette_from_lua_hex_table(text: &str) -> Option<ThemeColors> {
    let mut map: std::collections::HashMap<String, (u8, u8, u8)> =
        std::collections::HashMap::new();
    for line in text.lines() {
        let line = line.trim().trim_end_matches(',');
        // green = "#a6e3a1"
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let k = k.trim().to_lowercase();
        if k.contains(' ') || k.starts_with("--") || k.starts_with("config") {
            continue;
        }
        let v = v.trim().trim_matches(',').trim();
        if !v.contains('#') {
            continue;
        }
        // extract #rrggbb
        if let Some(pos) = v.find('#') {
            let hex = &v[pos..];
            let hex = hex.split(|c: char| !c.is_ascii_hexdigit() && c != '#').next()?;
            if let Some(rgb) = hex_rgb(hex) {
                map.insert(k, rgb);
            }
        }
    }

    // Need at least a green-ish rain color and a background.
    let bg = map
        .get("base")
        .or_else(|| map.get("background"))
        .or_else(|| map.get("bg"))
        .or_else(|| map.get("crust"))
        .or_else(|| map.get("mantle"))
        .copied()?;
    let fg = map
        .get("text")
        .or_else(|| map.get("foreground"))
        .or_else(|| map.get("fg"))
        .copied()
        .unwrap_or((0xcd, 0xd6, 0xf4));
    let green = map
        .get("green")
        .or_else(|| map.get("teal"))
        .copied()?;
    let yellow = map
        .get("yellow")
        .or_else(|| map.get("peach"))
        .copied()
        .unwrap_or(fg);
    let cyan = map
        .get("teal")
        .or_else(|| map.get("sky"))
        .or_else(|| map.get("sapphire"))
        .or_else(|| map.get("cyan"))
        .copied()
        .unwrap_or(green);
    let bwhite = map
        .get("text")
        .or_else(|| map.get("subtext1"))
        .copied()
        .unwrap_or(fg);
    let bgreen = map.get("green").copied().unwrap_or(green);

    Some(ThemeColors {
        name: "wezterm".into(),
        bg,
        fg,
        green,
        bgreen,
        yellow,
        cyan,
        bwhite,
    })
}

fn known_color_scheme(name: &str) -> Option<ThemeColors> {
    let n = name.to_lowercase();
    // Catppuccin family (WezTerm built-ins)
    if n.contains("catppuccin") && n.contains("mocha") {
        return Some(catppuccin_mocha());
    }
    if n.contains("catppuccin") && n.contains("macchiato") {
        return Some(catppuccin_macchiato());
    }
    if n.contains("catppuccin") && n.contains("frappe") {
        return Some(catppuccin_frappe());
    }
    if n.contains("catppuccin") && n.contains("latte") {
        return Some(catppuccin_latte());
    }
    None
}

fn catppuccin_mocha() -> ThemeColors {
    ThemeColors {
        name: "Catppuccin Mocha".into(),
        bg: hex_rgb("#1e1e2e").unwrap(),
        fg: hex_rgb("#cdd6f4").unwrap(),
        green: hex_rgb("#a6e3a1").unwrap(),
        bgreen: hex_rgb("#a6e3a1").unwrap(),
        yellow: hex_rgb("#f9e2af").unwrap(),
        cyan: hex_rgb("#94e2d5").unwrap(),
        bwhite: hex_rgb("#cdd6f4").unwrap(),
    }
}

fn catppuccin_macchiato() -> ThemeColors {
    ThemeColors {
        name: "Catppuccin Macchiato".into(),
        bg: hex_rgb("#24273a").unwrap(),
        fg: hex_rgb("#cad3f5").unwrap(),
        green: hex_rgb("#a6da95").unwrap(),
        bgreen: hex_rgb("#a6da95").unwrap(),
        yellow: hex_rgb("#eed49f").unwrap(),
        cyan: hex_rgb("#8bd5ca").unwrap(),
        bwhite: hex_rgb("#cad3f5").unwrap(),
    }
}

fn catppuccin_frappe() -> ThemeColors {
    ThemeColors {
        name: "Catppuccin Frappe".into(),
        bg: hex_rgb("#303446").unwrap(),
        fg: hex_rgb("#c6d0f5").unwrap(),
        green: hex_rgb("#a6d189").unwrap(),
        bgreen: hex_rgb("#a6d189").unwrap(),
        yellow: hex_rgb("#e5c890").unwrap(),
        cyan: hex_rgb("#81c8be").unwrap(),
        bwhite: hex_rgb("#c6d0f5").unwrap(),
    }
}

fn catppuccin_latte() -> ThemeColors {
    ThemeColors {
        name: "Catppuccin Latte".into(),
        bg: hex_rgb("#eff1f5").unwrap(),
        fg: hex_rgb("#4c4f69").unwrap(),
        green: hex_rgb("#40a02b").unwrap(),
        bgreen: hex_rgb("#40a02b").unwrap(),
        yellow: hex_rgb("#df8e1d").unwrap(),
        cyan: hex_rgb("#179299").unwrap(),
        bwhite: hex_rgb("#4c4f69").unwrap(),
    }
}

// ---------------------------------------------------------------------------
// Starship (your zsh prompt palette)
// ---------------------------------------------------------------------------

fn starship_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".config/starship.toml")
}

fn load_starship_colors() -> Option<ThemeColors> {
    let text = fs::read_to_string(starship_path()).ok()?;
    // palette = 'catppuccin_mocha'
    let palette_name = text.lines().find_map(|line| {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("palette") {
            let rest = rest.trim().trim_start_matches('=').trim();
            let rest = rest.trim_matches('"').trim_matches('\'');
            if !rest.is_empty() && !rest.starts_with('[') {
                return Some(rest.to_string());
            }
        }
        None
    });

    // Prefer named palette table; else first [palettes.*] block with green+base/text
    if let Some(name) = &palette_name {
        if let Some(c) = starship_palette_table(&text, name) {
            return Some(c);
        }
        // name may match known scheme
        if let Some(c) = known_color_scheme(name) {
            return Some(c);
        }
    }

    // Scan all [palettes.foo]
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("[palettes.") {
            let name = rest.trim_end_matches(']').trim();
            if let Some(c) = starship_palette_table(&text, name) {
                return Some(c);
            }
        }
    }
    None
}

fn starship_palette_table(text: &str, name: &str) -> Option<ThemeColors> {
    let header = format!("[palettes.{name}]");
    let start = text.find(&header)?;
    let body = &text[start + header.len()..];
    let end = body.find("\n[").unwrap_or(body.len());
    let section = &body[..end];

    let mut map = std::collections::HashMap::new();
    for line in section.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let k = k.trim().to_lowercase();
        let v = v.trim().trim_matches('"').trim_matches('\'');
        if let Some(rgb) = hex_rgb(v) {
            map.insert(k, rgb);
        }
    }

    let bg = map
        .get("base")
        .or_else(|| map.get("crust"))
        .or_else(|| map.get("mantle"))
        .copied()?;
    let fg = map.get("text").copied().unwrap_or((0xcd, 0xd6, 0xf4));
    let green = map.get("green").copied()?;
    let yellow = map
        .get("yellow")
        .or_else(|| map.get("peach"))
        .copied()
        .unwrap_or(fg);
    let cyan = map
        .get("teal")
        .or_else(|| map.get("sky"))
        .copied()
        .unwrap_or(green);

    Some(ThemeColors {
        name: format!("starship:{name}"),
        bg,
        fg,
        green,
        bgreen: green,
        yellow,
        cyan,
        bwhite: fg,
    })
}

// ---------------------------------------------------------------------------
// Omarchy (original auto path)
// ---------------------------------------------------------------------------

pub fn omarchy_theme_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".config/omarchy/current/theme")
}

#[derive(Deserialize)]
struct AlacrittyToml {
    colors: AlacrittyColors,
}

#[derive(Deserialize)]
struct AlacrittyColors {
    primary: Primary,
    normal: Normal,
    #[serde(default)]
    bright: Bright,
}

#[derive(Deserialize)]
struct Primary {
    background: String,
    foreground: String,
}

#[derive(Deserialize)]
struct Normal {
    green: String,
    yellow: String,
    cyan: String,
}

#[derive(Deserialize, Default)]
struct Bright {
    green: Option<String>,
    white: Option<String>,
}

pub fn load_omarchy_colors() -> Option<ThemeColors> {
    let theme = omarchy_theme_path();
    let path = theme.join("alacritty.toml");
    let text = fs::read_to_string(&path).ok()?;
    let data: AlacrittyToml = toml::from_str(&text).ok()?;
    let c = data.colors;
    let name = fs::read_link(&theme)
        .ok()
        .and_then(|p| p.file_name().map(|s| s.to_string_lossy().into_owned()))
        .or_else(|| theme.file_name().map(|s| s.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "omarchy".into());
    Some(ThemeColors {
        name,
        bg: hex_rgb(&c.primary.background)?,
        fg: hex_rgb(&c.primary.foreground)?,
        green: hex_rgb(&c.normal.green)?,
        bgreen: hex_rgb(c.bright.green.as_deref().unwrap_or(&c.normal.green))?,
        yellow: hex_rgb(&c.normal.yellow)?,
        cyan: hex_rgb(&c.normal.cyan)?,
        bwhite: hex_rgb(
            c.bright
                .white
                .as_deref()
                .unwrap_or(&c.primary.foreground),
        )?,
    })
}

// ---------------------------------------------------------------------------
// Palette builder
// ---------------------------------------------------------------------------

/// Palette as compact style IDs + the SGR table for Term::set_styles.
#[derive(Debug, Clone)]
pub struct Palette {
    pub sgr: Vec<String>,
    pub head: StyleId,
    pub trail: Vec<StyleId>,
    pub residue: Vec<StyleId>,
    pub reader: StyleId,
    pub dim: StyleId,
    pub blank: StyleId,
    pub news: StyleId,
    pub local: StyleId,
    pub poetic: StyleId,
    pub scramble: StyleId,
}

struct StyleBuilder {
    sgr: Vec<String>,
}

impl StyleBuilder {
    fn new() -> Self {
        Self {
            sgr: vec!["\x1b[0m".into()],
        }
    }

    fn add(&mut self, s: String) -> StyleId {
        let id = self.sgr.len() as StyleId;
        self.sgr.push(s);
        id
    }

    fn finish(
        self,
        head: StyleId,
        trail: Vec<StyleId>,
        residue: Vec<StyleId>,
        reader: StyleId,
        dim: StyleId,
        news: StyleId,
        local: StyleId,
        poetic: StyleId,
        scramble: StyleId,
    ) -> Palette {
        Palette {
            sgr: self.sgr,
            head,
            trail,
            residue,
            reader,
            dim,
            blank: 0,
            news,
            local,
            poetic,
            scramble,
        }
    }
}

fn build_theme_palette(t: &ThemeColors, focus: bool) -> Palette {
    let bg = t.bg;
    let green = t.green;
    let fgc = t.fg;
    let mut b = StyleBuilder::new();
    let g = |f: f64| fg(mix(green, bg, f), &[]);

    let head = b.add(fg(t.bwhite, &[1]));
    let trail = vec![
        b.add(g(0.0)),
        b.add(g(0.15)),
        b.add(g(0.35)),
        b.add(g(0.55)),
        b.add(g(0.7)),
        b.add(g(0.8)),
    ];
    let residue = vec![
        b.add(g(0.6)),
        b.add(g(0.72)),
        b.add(g(0.8)),
        b.add(fg(mix(fgc, bg, 0.85), &[])),
    ];
    let reader = b.add(fg(mix(fgc, bg, 0.12), &[]));
    let dim = b.add(fg(mix(fgc, bg, 0.5), &[]));
    let (news, local, poetic, scramble) = if focus {
        (
            b.add(fg(fgc, &[1])),
            b.add(fg(t.cyan, &[1])),
            b.add(fg(t.yellow, &[])),
            b.add(fg(t.bwhite, &[1])),
        )
    } else {
        (
            b.add(fg(mix(green, fgc, 0.45), &[])),
            b.add(fg(mix(t.cyan, bg, 0.2), &[])),
            b.add(fg(mix(t.yellow, bg, 0.45), &[])),
            b.add(fg(t.bgreen, &[1])),
        )
    };
    b.finish(head, trail, residue, reader, dim, news, local, poetic, scramble)
}

/// focus=false: messages sit embedded in the code.
/// focus=true: headlines surface — full contrast.
pub fn build_palette(basic: bool, focus: bool, theme: Option<&ThemeColors>) -> Palette {
    // If we resolved a real palette (WezTerm/Starship/OSC/…), always use it —
    // truecolor SGR works even when TERM is a weak hint.
    if let Some(t) = theme {
        return build_theme_palette(t, focus);
    }
    let _ = basic; // only used for the matrix / ANSI fallbacks below
    let mut b = StyleBuilder::new();
    if !basic {
        let head = b.add(sgr(&[0, 1, 38, 5, 48]));
        let trail = vec![
            b.add(sgr(&[0, 38, 5, 46])),
            b.add(sgr(&[0, 38, 5, 40])),
            b.add(sgr(&[0, 38, 5, 34])),
            b.add(sgr(&[0, 38, 5, 28])),
            b.add(sgr(&[0, 38, 5, 22])),
            b.add(sgr(&[0, 2, 38, 5, 22])),
        ];
        let residue = vec![
            b.add(sgr(&[0, 38, 5, 22])),
            b.add(sgr(&[0, 2, 38, 5, 28])),
            b.add(sgr(&[0, 2, 38, 5, 22])),
            b.add(sgr(&[0, 2, 38, 5, 235])),
        ];
        let reader = b.add(sgr(&[0, 38, 5, 250]));
        let dim = b.add(sgr(&[0, 38, 5, 241]));
        let (news, local, poetic, scramble) = if focus {
            (
                b.add(sgr(&[0, 1, 38, 5, 255])),
                b.add(sgr(&[0, 1, 38, 5, 87])),
                b.add(sgr(&[0, 38, 5, 222])),
                b.add(sgr(&[0, 1, 38, 5, 231])),
            )
        } else {
            (
                b.add(sgr(&[0, 38, 5, 120])),
                b.add(sgr(&[0, 38, 5, 80])),
                b.add(sgr(&[0, 38, 5, 137])),
                b.add(sgr(&[0, 1, 38, 5, 83])),
            )
        };
        return b.finish(head, trail, residue, reader, dim, news, local, poetic, scramble);
    }

    let g = sgr(&[0, 32]);
    let gd = sgr(&[0, 2, 32]);
    let head = b.add(sgr(&[0, 1, 32]));
    let trail = vec![
        b.add(sgr(&[0, 1, 32])),
        b.add(g.clone()),
        b.add(g),
        b.add(gd.clone()),
        b.add(gd.clone()),
        b.add(gd.clone()),
    ];
    let residue = vec![b.add(gd)];
    let reader = b.add(sgr(&[0, 37]));
    let dim = b.add(sgr(&[0, 2, 37]));
    let (news, local, poetic, scramble) = if focus {
        (
            b.add(sgr(&[0, 1, 37])),
            b.add(sgr(&[0, 1, 36])),
            b.add(sgr(&[0, 33])),
            b.add(sgr(&[0, 1, 37])),
        )
    } else {
        (
            b.add(sgr(&[0, 1, 32])),
            b.add(sgr(&[0, 36])),
            b.add(sgr(&[0, 33])),
            b.add(sgr(&[0, 1, 32])),
        )
    };
    b.finish(head, trail, residue, reader, dim, news, local, poetic, scramble)
}

pub const GLYPHS_KATA: &str =
    "ｦｧｨｩｪｫｬｭｮｯｰｱｲｳｴｵｶｷｸｹｺｻｼｽｾｿﾀﾁﾂﾃﾄﾅﾆﾇﾈﾉﾊﾋﾌﾍﾎﾏﾐﾑﾒﾓﾔﾕﾖﾗﾘﾙﾚﾛﾜﾝ0123456789:･=*+<>";
pub const GLYPHS_ASCII: &str = "abcdefghijklmnopqrstuvwxyz0123456789@#$%&*+=<>:~";
pub const SCRAMBLE: i32 = 4;
