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

- **News** — live headlines, narrowed by your topics, decoded into the
  stream. **Click a headline and its story decodes into the rain** as a
  short summary; shift-click opens the article in your browser (headlines
  are real OSC 8 hyperlinks).
- **Local intel** — tune to any towns or regions and read what is
  happening there, down to village-notice level.
- **Poetic** — true things happening right now, for scale and gratitude,
  drawn from a growing set of veins: the sky, the sea, deep time, the body,
  creatures, other people. Live counters (people born since you opened the
  window, what Voyager 1 did while you watched), tonight's actual moon phase,
  where the sun is rising at this second, what the season is doing at the
  poles. Every line is true, and more are finding their way in.

Written in Rust. One binary. Runs anywhere a modern terminal exists.

## Install

From source (Rust 1.70+):

```sh
git clone https://github.com/tomdavenport/meanwhile.git
cd meanwhile
cargo install --path .
meanwhile
```

Or build and link without installing to cargo's bin dir:

```sh
cargo build --release
ln -sf "$PWD/target/release/meanwhile" ~/.local/bin/meanwhile
```

On Arch: `yay -S meanwhile-rain` (the plain name was taken by the old
Lotus Sametime library, of all things).

News comes from **ordinary RSS/Atom feeds** (and a couple of free public
JSON APIs) — no API key, no AI search.

**Places** (`g`) map to national outlets, for example:

| place | sources |
|-------|---------|
| New Zealand | RNZ, Stuff |
| Australia | ABC, SBS, SMH |
| UK | BBC, Guardian, Sky, Independent |
| US | NPR, PBS, NYT, Politico |
| Canada | CBC, Globe and Mail |
| Japan | NHK |
| India | Times of India, BBC India |
| China / HK | SCMP, BBC China |
| France / Germany | France 24, DW |
| … | Europe, Middle East, Africa, LatAm BBC/Guardian/Al Jazeera packs |

**Topics** (`t`): world, technology, science, business, sport, politics,
culture, or a country name. Technology includes **Hacker News top stories
from the last 24 hours** (official Firebase API by score — not `hnrss/best`,
which is a longer-horizon ranking). Topic `hacker news` / `hn` is HN-only.

Paste any extra RSS/Atom URLs into `extra_feeds` in the config.

Click a headline to decode its feed blurb into the rain. An optional
[Exa](https://exa.ai) key (`EXA_API_KEY` in the environment or
`~/.config/meanwhile/.env`) only upgrades that summary — headlines work
without it. Pass `--offline` for poetic lines only.

## Keys

| key | action | key | action |
|-----|--------|-----|--------|
| `click` | **decode a story into the rain** | `q` | quit |
| `enter` | pick a story to decode (↑/↓ + enter) | `space` | pause |
| `t` | edit topics | `g` | edit places (local intel) |
| `f` | focus — surface the text | `n` / `o` | a headline / something true |
| `m` / `p` | toggle news / poetic | `r` | refresh headlines |
| `+` / `-` | speed | `s` / `?` | status / help |

Editors: type + enter adds, `1`–`9` removes, esc closes. Changes persist
and refetch immediately.

## Reading modes

By default text sits **embedded** — a shade above the field, part of the
code. Press `f` for **focus** and headlines surface in full contrast when
you actually want to read the world. The preference sticks.

Click any headline (or press `enter` to pick one) and its story decodes
into the stream as a short summary — you never leave the rain:

![summary](shots/summary.png)

## Theming — inherits your terminal

With `"theme": "auto"` (the default), meanwhile rains in *your* palette:

1. **Live terminal colors** via OSC (WezTerm, kitty, foot, …) — whatever
   scheme is active right now
2. **WezTerm config** (`color_scheme` / embedded palette table)
3. **Starship** palette (your prompt colors, e.g. Catppuccin Mocha)
4. **Omarchy** active theme, when present

Switch your WezTerm scheme or edit the config and the rain follows within
seconds. Set `"theme": "matrix"` (or `--theme matrix`) to force classic
green. Plain ANSI is used only when the terminal has no color support.

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
| `refresh_minutes`, `hours_back` | news freshness window |
| `extra_feeds` | extra RSS/Atom URLs to always pull |
| `env_files` | optional paths for `EXA_API_KEY` (summaries only) |

Headlines are cached in `~/.cache/meanwhile/` so launch is instant.

## CLI

```
meanwhile [OPTIONS]

      --offline          poetic lines only, no news fetch
      --ascii            ASCII glyphs (no katakana)
      --topics <TOPICS>  comma-separated topics, overrides config
      --places <PLACES>  comma-separated places for local intel
      --speed <SPEED>    speed multiplier
      --theme <THEME>    auto | matrix
  -h, --help
  -V, --version
```

## Notes

- Plain click decodes a summary in-app; shift-click follows the OSC 8
  hyperlink (kitty, Alacritty, ghostty, foot, WezTerm and most modern
  terminals; inside tmux you need a recent tmux). Set `"mouse": false`
  in the config to leave the mouse alone entirely.
- CPU ~1–2% of one core; it's just characters.
- Every so often — not often — the rain has something to say to you
  directly. If you're impatient, you know whose name to type.

## License

MIT
