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
        .or_else(|| {
            theme
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
        })
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

#[derive(Debug, Clone)]
pub struct Palette {
    pub head: String,
    pub trail: Vec<String>,
    pub residue: Vec<String>,
    pub reader: String,
    pub dim: String,
    pub blank: String,
    pub news: String,
    pub local: String,
    pub poetic: String,
    pub scramble: String,
}

fn build_theme_palette(t: &OmarchyColors, focus: bool) -> Palette {
    let bg = t.bg;
    let green = t.green;
    let fgc = t.fg;
    let g = |f: f64| fg(mix(green, bg, f), &[]);

    let mut pal = Palette {
        head: fg(t.bwhite, &[1]),
        trail: vec![g(0.0), g(0.15), g(0.35), g(0.55), g(0.7), g(0.8)],
        residue: vec![
            g(0.6),
            g(0.72),
            g(0.8),
            fg(mix(fgc, bg, 0.85), &[]),
        ],
        reader: fg(mix(fgc, bg, 0.12), &[]),
        dim: fg(mix(fgc, bg, 0.5), &[]),
        blank: sgr(&[0]),
        news: String::new(),
        local: String::new(),
        poetic: String::new(),
        scramble: String::new(),
    };
    if focus {
        pal.news = fg(fgc, &[1]);
        pal.local = fg(t.cyan, &[1]);
        pal.poetic = fg(t.yellow, &[]);
        pal.scramble = fg(t.bwhite, &[1]);
    } else {
        pal.news = fg(mix(green, fgc, 0.45), &[]);
        pal.local = fg(mix(t.cyan, bg, 0.2), &[]);
        pal.poetic = fg(mix(t.yellow, bg, 0.45), &[]);
        pal.scramble = fg(t.bgreen, &[1]);
    }
    pal
}

/// focus=false: messages sit embedded in the code.
/// focus=true: headlines surface — bright white, full contrast.
pub fn build_palette(basic: bool, focus: bool, theme: Option<&OmarchyColors>) -> Palette {
    if let Some(t) = theme {
        if !basic {
            return build_theme_palette(t, focus);
        }
    }
    if !basic {
        let mut pal = Palette {
            head: sgr(&[0, 1, 38, 5, 48]),
            trail: vec![
                sgr(&[0, 38, 5, 46]),
                sgr(&[0, 38, 5, 40]),
                sgr(&[0, 38, 5, 34]),
                sgr(&[0, 38, 5, 28]),
                sgr(&[0, 38, 5, 22]),
                sgr(&[0, 2, 38, 5, 22]),
            ],
            residue: vec![
                sgr(&[0, 38, 5, 22]),
                sgr(&[0, 2, 38, 5, 28]),
                sgr(&[0, 2, 38, 5, 22]),
                sgr(&[0, 2, 38, 5, 235]),
            ],
            reader: sgr(&[0, 38, 5, 250]),
            dim: sgr(&[0, 38, 5, 241]),
            blank: sgr(&[0]),
            news: String::new(),
            local: String::new(),
            poetic: String::new(),
            scramble: String::new(),
        };
        if focus {
            pal.news = sgr(&[0, 1, 38, 5, 255]);
            pal.local = sgr(&[0, 1, 38, 5, 87]);
            pal.poetic = sgr(&[0, 38, 5, 222]);
            pal.scramble = sgr(&[0, 1, 38, 5, 231]);
        } else {
            pal.news = sgr(&[0, 38, 5, 120]);
            pal.local = sgr(&[0, 38, 5, 80]);
            pal.poetic = sgr(&[0, 38, 5, 137]);
            pal.scramble = sgr(&[0, 1, 38, 5, 83]);
        }
        return pal;
    }

    let g = sgr(&[0, 32]);
    let gd = sgr(&[0, 2, 32]);
    let mut pal = Palette {
        head: sgr(&[0, 1, 32]),
        trail: vec![
            sgr(&[0, 1, 32]),
            g.clone(),
            g,
            gd.clone(),
            gd.clone(),
            gd.clone(),
        ],
        residue: vec![gd],
        reader: sgr(&[0, 37]),
        dim: sgr(&[0, 2, 37]),
        blank: sgr(&[0]),
        news: String::new(),
        local: String::new(),
        poetic: String::new(),
        scramble: String::new(),
    };
    if focus {
        pal.news = sgr(&[0, 1, 37]);
        pal.local = sgr(&[0, 1, 36]);
        pal.poetic = sgr(&[0, 33]);
        pal.scramble = sgr(&[0, 1, 37]);
    } else {
        pal.news = sgr(&[0, 1, 32]);
        pal.local = sgr(&[0, 36]);
        pal.poetic = sgr(&[0, 33]);
        pal.scramble = sgr(&[0, 1, 32]);
    }
    pal
}

pub const GLYPHS_KATA: &str =
    "ｦｧｨｩｪｫｬｭｮｯｰｱｲｳｴｵｶｷｸｹｺｻｼｽｾｿﾀﾁﾂﾃﾄﾅﾆﾇﾈﾉﾊﾋﾌﾍﾎﾏﾐﾑﾒﾓﾔﾕﾖﾗﾘﾙﾚﾛﾜﾝ0123456789:･=*+<>";
pub const GLYPHS_ASCII: &str = "abcdefghijklmnopqrstuvwxyz0123456789@#$%&*+=<>:~";
pub const SCRAMBLE: i32 = 4;
