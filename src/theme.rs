use crate::term::StyleId;
use serde::Deserialize;
use std::fs;
use std::path::PathBuf;
use std::time::SystemTime;

pub fn omarchy_theme_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".config/omarchy/current/theme")
}

pub fn sgr(codes: &[u32]) -> String {
    let parts: Vec<String> = codes.iter().map(|c| c.to_string()).collect();
    format!("\x1b[{}m", parts.join(";"))
}

fn hex_rgb(s: &str) -> (u8, u8, u8) {
    let s = s.trim_start_matches('#');
    if s.len() < 6 {
        return (0, 0, 0);
    }
    let r = u8::from_str_radix(&s[0..2], 16).unwrap_or(0);
    let g = u8::from_str_radix(&s[2..4], 16).unwrap_or(0);
    let b = u8::from_str_radix(&s[4..6], 16).unwrap_or(0);
    (r, g, b)
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

#[derive(Debug, Clone, PartialEq)]
pub struct OmarchyColors {
    pub name: String,
    pub bg: (u8, u8, u8),
    pub fg: (u8, u8, u8),
    pub green: (u8, u8, u8),
    pub bgreen: (u8, u8, u8),
    pub yellow: (u8, u8, u8),
    pub cyan: (u8, u8, u8),
    pub bwhite: (u8, u8, u8),
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

pub fn load_omarchy_colors() -> Option<OmarchyColors> {
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
    Some(OmarchyColors {
        name,
        bg: hex_rgb(&c.primary.background),
        fg: hex_rgb(&c.primary.foreground),
        green: hex_rgb(&c.normal.green),
        bgreen: hex_rgb(c.bright.green.as_deref().unwrap_or(&c.normal.green)),
        yellow: hex_rgb(&c.normal.yellow),
        cyan: hex_rgb(&c.normal.cyan),
        bwhite: hex_rgb(
            c.bright
                .white
                .as_deref()
                .unwrap_or(&c.primary.foreground),
        ),
    })
}

pub fn theme_mtime() -> Option<SystemTime> {
    fs::symlink_metadata(omarchy_theme_path())
        .ok()
        .and_then(|m| m.modified().ok())
}

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
            sgr: vec!["\x1b[0m".into()], // 0 = blank/reset
        }
    }

    fn add(&mut self, s: String) -> StyleId {
        let id = self.sgr.len() as StyleId;
        self.sgr.push(s);
        id
    }

    fn finish(self, head: StyleId, trail: Vec<StyleId>, residue: Vec<StyleId>,
              reader: StyleId, dim: StyleId, news: StyleId, local: StyleId,
              poetic: StyleId, scramble: StyleId) -> Palette {
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

fn build_theme_palette(t: &OmarchyColors, focus: bool) -> Palette {
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
/// focus=true: headlines surface — bright white, full contrast.
pub fn build_palette(basic: bool, focus: bool, theme: Option<&OmarchyColors>) -> Palette {
    if let Some(t) = theme {
        if !basic {
            return build_theme_palette(t, focus);
        }
    }
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
