# meanwhile

Horizontal matrix rain of things happening right now.

Green noise streams sweep across the terminal like cmatrix turned on its side.
Out of the noise, real things decode into view:

- **News** (white) — live headlines pulled from the [Exa](https://exa.ai) API,
  narrowed by your topics, refreshed every 15 minutes.
- **Poetic** (amber) — true things happening somewhere on Earth at this moment:
  live counters (births, lightning strikes, Voyager 1's distance), city clocks
  ("the sun is climbing over Tokyo about now"), and a corpus of quiet facts.
  For a sense of scale, and gratitude.

No news key? It runs poetic-only. Stdlib-only Python, no dependencies.

## Run

```sh
meanwhile                     # if installed on PATH
python3 meanwhile.py          # or directly
```

Options: `--offline` (poetic only) · `--ascii` (no katakana) ·
`--topics "climate, space"` · `--speed 1.5`

## Keys

| key | action | key | action |
|-----|--------|-----|--------|
| `q` | quit | `space` | pause |
| `n` | a headline now | `o` | something true now |
| `m` | toggle news | `p` | toggle poetic |
| `+` / `-` | speed | `r` | refresh headlines |
| `s` | status bar | `?` | help |

## Config

`~/.config/meanwhile/config.json` (created on first run):

- `topics` — what the news pull is narrowed to
- `poetic_ratio` — fraction of lines that are poetic (0–1)
- `density`, `speed`, `message_every_seconds` — feel of the rain
- `env_files` — where to find `EXA_API_KEY` if it isn't in the environment
- `refresh_minutes`, `hours_back` — news freshness window

## Install

```sh
ln -sf ~/dev/meanwhile/meanwhile.py ~/.local/bin/meanwhile
chmod +x ~/dev/meanwhile/meanwhile.py
```
