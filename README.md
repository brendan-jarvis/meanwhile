# meanwhile

**Horizontal matrix rain of things happening right now.**

A wall of code sweeps across your terminal. Embedded in the noise — readable
if you look — are real things: live news headlines, local intel for places
you care about, and true, quietly poetic facts about what is happening
somewhere on Earth at this exact moment.

```
 :ﾌ ｿ+ﾛ 1 ｱ8     2ﾏ ｺﾋｼｯｽ ｼｩｵﾌﾔﾑﾃ the sun is climbing over Auckland about now  7ﾋﾝﾑﾝｸｾ ﾘ*ｷ >ﾊｪ
9 :ｭｷｰｫﾑﾅｭｸｴ   ﾁｾｭ ﾍ･+ﾒ:ｹﾕﾔﾚﾘｬﾌﾃｪｽｱｲ< ｭｯｺｺ ﾉ ｯｧｴﾃ 3 ﾏﾈ7 ･ ﾒｸﾈ  5ｫｽ ﾖ2+ﾊｬｪﾜｻ  ｿ3ﾜ ･ ｲ ﾁﾕﾝｫｭ6ｮ
ﾍﾗｯ9ｰ Trump 'Planning for Wider War' in Iran — With Possible Ground Invasion — as US Bombing…
 ﾋｦ ｽﾏﾏ6 ｷﾄﾙ3ﾐｴｦ41 ﾗｼ ﾅﾒｱﾙﾄﾁﾇﾒ6ﾚﾕｽ4    ﾇ ｹﾐ  8 ｫｯ ﾀﾘﾘ ﾒ+ｴﾓｴ>ﾃ   ﾛ            ｫｭｸ  ｨﾓﾖｶ ｮ5<ﾏ
>ﾁｮﾘｩ<ﾚ ﾁｾﾔ30ｴｦ As it happened: Burnham puts his stamp on Cabinet += ｳﾜ ﾘ3ﾇﾈ  ｼ ﾏ4ｧ ﾘﾓ>ｩﾏﾔﾍ
ﾆﾒ95ﾙｯｽ ｶｸ ﾖ6ﾋ ｨﾗ lightning has <ﾐﾙﾀﾍﾛ2ﾈ:<ｹｶ1ﾘ24ｱｮﾚｺｰﾔﾑ0ﾊﾕｫｱﾖｭ=ﾙ +ﾝ1ﾌ ﾔ *ﾓﾃｩ  *<ﾑ<ﾌﾐｻﾕｹ5ﾆｫ
```

In *The Matrix*, the operators stopped seeing code and started seeing the
world through it. That's the idea: an ambient screen you can actually read.
Glance at it and it's rain; look at it and it's the world.

- **News** — live headlines, narrowed by your topics, decoded into the
  stream. Every headline is a real hyperlink (OSC 8): click it to open the
  story, or press `enter` and read the article *inside the rain*.
- **Local intel** — tune to any towns or regions and read what is
  happening there, down to village-notice level.
- **Poetic** — true things happening right now, for scale and gratitude:
  how many people were born since you opened the window, tonight's actual
  moon phase, where the sun is rising at this moment, what Voyager 1 has
  done while you watched.

Zero dependencies. One Python file. Runs anywhere Python 3.11+ and a
terminal exist.

## Install

```sh
git clone https://github.com/tomdavenport/meanwhile.git
cd meanwhile
ln -sf "$PWD/meanwhile.py" ~/.local/bin/meanwhile
meanwhile
```

News comes from the [Exa](https://exa.ai) search API (`fast` type — their
cheapest tier, a few requests per refresh). Put `EXA_API_KEY` in your
environment (or point `env_files` in the config at a `.env` that has it).
**No key? It still runs**, poetic-only.

## Keys

| key | action | key | action |
|-----|--------|-----|--------|
| `enter` | **read an article in the rain** | `q` | quit |
| `t` | edit topics | `g` | edit places (local intel) |
| `f` | focus — surface the text | `space` | pause |
| `n` | a headline now | `o` | something true now |
| `m` / `p` | toggle news / poetic | `r` | refresh headlines |
| `+` / `-` | speed | `s` / `?` | status / help |

Editors: type + enter adds, `1`–`9` removes, esc closes. Changes persist
and refetch immediately. In the reader: `j`/`k`/`space` scroll, `q` closes.

## Reading modes

By default text sits **embedded** — a shade above the field, part of the
code. Press `f` for **focus** and headlines surface in full contrast when
you actually want to read the world. The preference sticks.

## Theming — including Omarchy, out of the box

On [Omarchy](https://omarchy.org), meanwhile adopts your active theme
automatically: it reads the current theme's colors and rains in *your*
palette — and when you switch themes, the rain follows within seconds,
live. No configuration.

Elsewhere it defaults to classic matrix green (256-color), with a plain
ANSI fallback. Set `"theme": "matrix"` in the config to force green
anywhere.

## Config

`~/.config/meanwhile/config.json`, created on first run:

| key | meaning |
|-----|---------|
| `topics` | what the news follows (also: `t` in-app) |
| `places` | places for local intel (also: `g` in-app) |
| `poetic_ratio` | fraction of lines that are poetic |
| `density`, `speed`, `message_every_seconds` | feel of the rain |
| `focus`, `theme`, `show_source`, `ascii_only` | look |
| `refresh_minutes`, `hours_back` | news freshness window |
| `env_files` | where to find `EXA_API_KEY` |

Headlines are cached in `~/.cache/meanwhile/` so launch is instant.

## Notes

- Clickable links need a terminal with OSC 8 support (kitty, Alacritty,
  ghostty, foot, WezTerm and most modern terminals; inside tmux you need a
  recent tmux). The built-in reader works everywhere.
- CPU ~1–2% of one core; it's just characters.

## License

MIT
