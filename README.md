# meanwhile

**Horizontal matrix rain of things happening right now.**

A wall of code sweeps across your terminal. Embedded in the noise — readable
if you look — are real things: live news headlines, local intel for places
you care about, and true, quietly poetic facts about what is happening
somewhere on Earth at this exact moment.

![meanwhile](shots/hero.png)

In *The Matrix*, the operators stopped seeing code and started seeing the
world through it. That's the idea: an ambient screen you can actually read.
Glance at it and it's rain; look at it and it's the world.

- **News** — headlines from ordinary **RSS/Atom** feeds (and a couple of free
  public JSON APIs), matched to your topics and places. **Click a headline**
  and its story decodes into the rain as a short blurb; **shift-click** opens
  the article in your browser.
- **Local intel** — tune places (`g`) such as New Zealand, UK, or Japan and
  read national/regional coverage from open feeds (RNZ, Stuff, BBC, NHK, …).
- **Poetic** — true things happening right now, for scale and gratitude:
  live counters, tonight's moon phase, where the sun is rising, seasonal
  lines. Every one is true.

**Written in Rust** — a single native binary, no Python runtime, low CPU.

## This fork

This is a **Rust port and continuation** of the original project:

- **Original idea & Python prototype:** [Tom Davenport](https://github.com/tomdavenport) —
  [tomdavenport/meanwhile](https://github.com/tomdavenport/meanwhile).  
  Credit where it's due: a lovely ambient concept, and a joy to reimplement.
- **This repository:** [brendan-jarvis/meanwhile](https://github.com/brendan-jarvis/meanwhile)

Notable changes in this fork include the Rust rewrite, RSS-first news
(no search API required for headlines), broader national feed maps, terminal
theme inheritance (WezTerm / Starship / OSC / Omarchy), feed diagnostics, and
performance work aimed at multi-pane terminals like WezTerm.

## Install

Rust 1.70+ (edition 2021):

```sh
git clone https://github.com/brendan-jarvis/meanwhile.git
cd meanwhile
cargo install --path .
meanwhile
```

Or build without installing into Cargo's bin dir:

```sh
cargo build --release
ln -sf "$PWD/target/release/meanwhile" ~/.local/bin/meanwhile
```

Arch users may still find an AUR package for the upstream project
(`meanwhile-rain`); packaging files under `packaging/` describe a cargo-based
build if you want to package this fork yourself.

## News & places

Headlines come from **open feeds** — no API key and no AI search for the rain
itself.

**Places** (`g`) map to national outlets, for example:

| place | sources (examples) |
|-------|--------------------|
| New Zealand | RNZ, Stuff |
| Australia | ABC, SBS, SMH |
| UK | BBC, Guardian, Sky, Independent |
| US | NPR, PBS, NYT, Politico |
| Canada | CBC, Globe and Mail |
| Japan | NHK |
| India | Times of India, BBC India |
| China / HK | SCMP, BBC China |
| France / Germany | France 24, DW |
| … | Europe, Middle East, Africa, LatAm packs |

**Topics** (`t`): world, technology, science, business, sport, politics,
culture, or a country name. Technology includes **Hacker News** top-of-list
items via the free Firebase API (not `hnrss/best`, which is a longer-horizon
ranking). Topic `hacker news` / `hn` is HN-only.

Add arbitrary feed URLs with `extra_feeds` in the config. Click decodes the
feed blurb into the rain; an optional [Exa](https://exa.ai) key
(`EXA_API_KEY`) only upgrades that summary. Pass `--offline` for poetic-only.

## Keys

| key | action | key | action |
|-----|--------|-----|--------|
| `click` | **decode a story into the rain** | `shift-click` | open article in browser |
| `enter` | pick a story to decode (↑/↓ + enter) | `space` | pause |
| `t` | edit topics | `g` | edit places (local intel) |
| `f` | focus — surface the text | `n` / `o` | a headline / something true |
| `m` / `p` | toggle news / poetic | `r` | refresh headlines |
| `+` / `-` | speed | `s` / `d` / `?` | status / feed debug / help |
| `q` | quit | | |

Editors: type + enter adds, `1`–`9` removes, esc closes. Changes persist
and refetch immediately.

## Reading modes

By default text sits **embedded** — a shade above the field, part of the
code. Press `f` for **focus** and headlines surface in full contrast when
you actually want to read the world. The preference sticks.

Click any headline (or press `enter` to pick one) and its story expands
**from that line** into a short summary — you never leave the rain:

![summary](shots/summary.png)

## Theming

With `"theme": "auto"` (the default), meanwhile rains in *your* palette:

1. **Live terminal colors** via OSC (WezTerm, kitty, foot, …)
2. **WezTerm config** (`color_scheme` / embedded palette table)
3. **Starship** palette (e.g. Catppuccin Mocha)
4. **Omarchy** active theme, when present

Set `"theme": "matrix"` (or `--theme matrix`) for classic phosphor green.

![omarchy theming](shots/omarchy-theme.png)

## Config

`~/.config/meanwhile/config.json`, created on first run:

| key | meaning |
|-----|---------|
| `topics` | what the news follows (also: `t` in-app) |
| `places` | places for local intel (also: `g` in-app) |
| `poetic_ratio` | fraction of lines that are poetic |
| `density`, `speed`, `message_every_seconds` | feel of the rain (density default **0.45**) |
| `fps` | redraw rate (default **8** — quiet ambient pace; 4–30) |
| `focus`, `theme`, `show_source`, `ascii_only` | look |
| `refresh_minutes` | how often to re-pull feeds |
| `extra_feeds` | extra RSS/Atom URLs to always pull |
| `env_files` | optional paths for `EXA_API_KEY` (summaries only) |
| `mouse` | set `false` to leave the mouse alone |

Cache and diagnostics live under `~/.cache/meanwhile/` (`headlines.json`,
`last-fetch.log`).

## CLI

```
meanwhile [OPTIONS]

      --offline          poetic lines only, no news fetch
      --ascii            ASCII glyphs (no katakana)
      --topics <TOPICS>  comma-separated topics, overrides config
      --places <PLACES>  comma-separated places for local intel
      --speed <SPEED>    speed multiplier
      --theme <THEME>    auto | matrix
  -v, --verbose          log each feed fetch to stderr
      --check-feeds      fetch once, print diagnostics, exit (no TUI)
  -h, --help
  -V, --version
```

### Debugging feeds

```sh
# one-shot NZ check (no rain UI)
meanwhile --check-feeds --places "New Zealand"

# same, with timing lines on stderr
meanwhile --check-feeds -v --places "New Zealand"

# while running: press d for the last fetch log, r to retry
# log file: ~/.cache/meanwhile/last-fetch.log
```

## Notes

- Plain click decodes in-app; **shift-click opens the browser** (handled by
  meanwhile so it works under WezTerm mouse tracking). OSC 8 is still emitted
  as a bonus for terminals that honour it.
- Defaults are tuned for multi-pane use (~8 fps, modest density); raise
  `fps` / `density` if you want a denser wall.
- Every so often — not often — the rain has something to say to you
  directly. If you're impatient, you know whose name to type.

## License

MIT — see [LICENSE](LICENSE). Original copyright Tom Davenport; this fork
continues under the same license.
