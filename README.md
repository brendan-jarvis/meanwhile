# meanwhile

Horizontal matrix rain of things happening right now.

A full wall of code sweeps across the terminal — streams leave settled
glyphs behind, erasers carve moving windows of darkness through them —
and out of the noise, real things decode into view:

- **News** (light green; bright white in focus mode) — live headlines from the [Exa](https://exa.ai) API
  (`fast` search, their cheapest tier), narrowed by your topics.
  Headlines are OSC 8 hyperlinks: click (or ctrl+click, terminal-dependent)
  to open the story.
- **Local intel** (cyan) — tune to one or more places (a town is enough)
  and read what is happening there.
- **Poetic** (amber) — true things happening somewhere on Earth right now:
  live counters (births, lightning, Voyager 1), the moon's actual phase
  tonight, city clocks, seasonal truths. For scale, and gratitude.

No news key? It runs poetic-only. Stdlib-only Python, no dependencies.

## Run

```sh
meanwhile                     # if installed on PATH
python3 meanwhile.py          # or directly
```

Options: `--offline` · `--ascii` · `--topics "climate, space"` ·
`--places "Bristol, Marlborough"` · `--speed 1.5`

## Keys

| key | action | key | action |
|-----|--------|-----|--------|
| `q` | quit | `space` | pause |
| `n` | a headline now | `o` | something true now |
| `t` | **edit topics** | `g` | **edit places (local intel)** |
| `m` | toggle news | `p` | toggle poetic |
| `+` / `-` | speed | `r` | refresh headlines |
| `f` | focus mode (surface text) | `s` / `?` | status / help |

In the topic/place editors: type + enter adds an entry, `1`–`9` removes
one, esc closes. Edits persist to config and refetch immediately.

## Config

`~/.config/meanwhile/config.json` (created on first run):

- `topics` — what the news pull follows (also editable in-app with `t`)
- `places` — places for local intel (also editable in-app with `g`)
- `poetic_ratio` — fraction of lines that are poetic (0–1)
- `density`, `speed`, `message_every_seconds` — feel of the rain
- `env_files` — where to find `EXA_API_KEY` if it isn't in the environment
- `refresh_minutes`, `hours_back` — news freshness window

Headlines are cached in `~/.cache/meanwhile/` so launches show news
instantly while the first fetch runs.

## Clickable headlines

Headlines are emitted as OSC 8 hyperlinks. Supported by kitty, Alacritty,
ghostty, foot, WezTerm and most modern terminals (activation varies:
plain click or ctrl/cmd+click). Inside tmux you need a recent tmux with
hyperlink support for them to pass through.

## Install

```sh
ln -sf ~/dev/meanwhile/meanwhile.py ~/.local/bin/meanwhile
chmod +x ~/dev/meanwhile/meanwhile.py
```
