mod app;
mod config;
mod news;
mod poetic;
mod rain;
mod term;
mod theme;

use app::App;
use clap::Parser;
use config::{load_config, resolve_api_key, VERSION};
use news::Newsfeed;
use std::process;
use std::sync::Arc;
use term::{is_tty, Term};

/// meanwhile — horizontal matrix rain of things happening right now
#[derive(Parser, Debug)]
#[command(name = "meanwhile", version = VERSION)]
struct Cli {
    /// poetic lines only, no news fetch
    #[arg(long)]
    offline: bool,

    /// ASCII glyphs (no katakana)
    #[arg(long)]
    ascii: bool,

    /// comma-separated topics, overrides config
    #[arg(long)]
    topics: Option<String>,

    /// comma-separated places for local intel, overrides config
    #[arg(long)]
    places: Option<String>,

    /// speed multiplier
    #[arg(long)]
    speed: Option<f64>,

    /// palette source
    #[arg(long, value_parser = ["auto", "matrix"])]
    theme: Option<String>,
}

fn main() {
    let args = Cli::parse();

    if !is_tty() {
        eprintln!("meanwhile needs an interactive terminal");
        process::exit(1);
    }

    let mut cfg = load_config();
    if let Some(topics) = args.topics {
        cfg.topics = topics
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
    }
    if let Some(places) = args.places {
        cfg.places = places
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
    }
    if let Some(speed) = args.speed {
        cfg.speed = speed;
    }
    if let Some(theme) = args.theme {
        cfg.theme = theme;
    }
    if args.ascii {
        cfg.ascii_only = true;
    }

    // Headlines come from RSS (no key). Exa key is optional for richer summaries.
    let key = if args.offline {
        None
    } else {
        resolve_api_key(&cfg)
    };
    let feed = Arc::new(Newsfeed::new(cfg.clone(), key, args.offline));
    feed.start();

    let term = match Term::new() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("meanwhile: {e}");
            process::exit(1);
        }
    };

    let mut app = App::new(term, cfg, feed);
    if let Err(e) = app.run() {
        eprintln!("meanwhile: {e}");
        process::exit(1);
    }
}
