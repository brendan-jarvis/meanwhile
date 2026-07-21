#!/usr/bin/env python3
"""meanwhile — horizontal matrix rain of things happening right now.

A full wall of code sweeps across the terminal like cmatrix turned on its
side. Out of the noise, real things decode into view: live news headlines
(via the Exa API) in white — clickable in terminals that support hyperlinks —
local intel for a place you tune to in cyan, and true, quietly poetic facts
about what is happening somewhere on Earth right now in amber.

stdlib only. Run: meanwhile
"""

import argparse
import json
import math
import os
import random
import select
import shutil
import signal
import sys
import termios
import textwrap
import threading
import time
import tty
import urllib.error
import urllib.request
from datetime import datetime, timedelta, timezone
from pathlib import Path

VERSION = "0.2.0"
CONFIG_PATH = Path.home() / ".config" / "meanwhile" / "config.json"
CACHE_PATH = Path.home() / ".cache" / "meanwhile" / "headlines.json"

DEFAULT_CONFIG = {
    "topics": ["world news", "artificial intelligence", "uk"],
    "places": [],
    "refresh_minutes": 15,
    "hours_back": 36,
    "poetic_ratio": 0.4,
    "message_every_seconds": 1.8,
    "density": 0.75,
    "speed": 1.0,
    "focus": False,
    "theme": "auto",        # "auto" adopts the active Omarchy theme; "matrix" forces green
    "show_source": False,   # append the domain after each headline
    "ascii_only": False,
    "env_files": ["~/dev/tom-os/.env", "~/dev/exa-newsdesk/.env"],
}

GLYPHS_KATA = "ｦｧｨｩｪｫｬｭｮｯｰｱｲｳｴｵｶｷｸｹｺｻｼｽｾｿﾀﾁﾂﾃﾄﾅﾆﾇﾈﾉﾊﾋﾌﾍﾎﾏﾐﾑﾒﾓﾔﾕﾖﾗﾘﾙﾚﾛﾜﾝ0123456789:･=*+<>"
GLYPHS_ASCII = "abcdefghijklmnopqrstuvwxyz0123456789@#$%&*+=<>:~"
SCRAMBLE = 4  # cells of un-decoded shimmer at a message head

POETIC = {
    "sky": [
        "the light warming earth at this moment left the sun eight minutes ago",
        "a photon born in the sun's core wandered a hundred thousand years to its surface — then took eight minutes to reach you",
        "about two thousand thunderstorms are rolling across the planet right now",
        "every second, the sun turns four million tonnes of itself into light",
        "the moon drifts 3.8 centimetres farther from earth every year, and still it holds the tides",
        "about a hundred tonnes of stardust will settle on the earth today",
        "the pole star's light arriving tonight left it around the year 1600",
        "the ISS circles the earth every 92 minutes; its crew sees sixteen sunrises a day",
        "satellites are photographing the earth right now; you may be in one of the pictures",
        "about ten thousand working satellites are over your head tonight",
        "there are more trees on earth than stars in the milky way",
        "the northern lights are on right now, whether anyone is standing under them or not",
        "since the year 2000, there has never been a moment when every human being was on earth — someone is up there now",
    ],
    "sea": [
        "half the oxygen in your next breath came from the ocean",
        "the amazon is pouring a fifth of all the world's river water into the atlantic right now",
        "a message in a bottle is floating somewhere at sea right now",
        "every wave that has ever reached a shore was already old when it arrived",
        "the tide is coming in somewhere, always",
        "rain is falling on the open ocean right now, unwatched, in the middle of everything",
        "on some abyssal plain, a fallen whale is feeding a community that will eat for decades",
        "tonight the moon pulls a tide across every shore on earth",
    ],
    "deep time": [
        "mount everest is being pushed a few millimetres higher every year",
        "somewhere, rain that fell a thousand years ago is melting out of a glacier",
        "somewhere in california, a tree older than the pyramids is quietly adding a ring",
        "deep in a swedish forest, a spruce root system has been alive for nine and a half thousand years",
        "in the arctic, greenland sharks born before newton are still swimming",
        "beneath your feet spins an iron core nearly the size of the moon",
        "there is a desert where it has not rained in five hundred years; it is probably sunny there today",
        "the atlantic is about a coin's width wider than it was last year",
        "the continents drift about as fast as your fingernails grow",
        "somewhere in a cave, a stalactite just gained a drop; in a century it will show",
    ],
    "the body": [
        "the atoms in your left hand and your right were forged in different stars",
        "your body made about two million red blood cells in the last second",
        "you will blink about twenty thousand times today and remember none of them",
        "the iron in your blood was made in the death of a star",
        "the surface of the earth is carrying you at hundreds of miles an hour, and you cannot feel it",
    ],
    "creatures": [
        "bar-tailed godwits can fly eleven days across the pacific without landing once",
        "somewhere over the southern ocean, an albatross is asleep on the wing",
        "somewhere it is daytime, and a hummingbird's heart is doing twenty beats a second",
        "right now, some four hundred million cats are asleep",
        "an arctic tern sees more daylight than any creature on earth; one is in the light right now",
    ],
    "people": [
        "right now, about a million people are in the air",
        "somewhere, a baby just took their first breath",
        "somewhere, a library just opened for the morning",
        "somewhere, two people who will love each other haven't met yet",
        "east of the dateline it is yesterday; west of it, tomorrow is already underway",
        "right now, someone is laughing so hard they can't breathe",
        "eight billion hearts are beating at this moment, hardly any of them in step",
        "the wheat in your last meal was sown by someone you will never meet",
        "somewhere, a night-shift nurse is checking on a ward of sleeping strangers",
        "on the day side of earth, four billion people are going about their morning",
        "somewhere, a fisherman is hauling nets under stars you have never seen",
        "somewhere, a teacher just watched an idea land",
        "somewhere, an orchestra is tuning to the same A it has been for two centuries",
        "someone, somewhere, just finished the last page of their first novel",
        "somewhere, two strangers just swapped seats so a family could sit together",
        "somewhere, an apology twenty years late is finally being written",
    ],
}

# lines that are only true in part of the year, keyed by month group
SEASONAL = {
    (12, 1, 2): [
        "it is high summer in antarctica; the sun circles the sky without setting",
        "in the far north the sun barely clears the horizon, and the snow holds the light all day",
    ],
    (3, 4, 5): [
        "the cherry-blossom front is moving north through japan about now",
        "across the north, billions of birds are flying home for the spring",
    ],
    (6, 7, 8): [
        "above the arctic circle, the sun has not set for weeks",
        "it is midwinter in patagonia; snow is settling on the andes",
        "humpback whales are gathering in the warm seas off madagascar to breed",
    ],
    (9, 10, 11): [
        "monarch butterflies are streaming south across america toward mexico",
        "across the north, the forests are turning gold a little further south each day",
    ],
}

# (city, rough UTC offset in July) — for "the sun is rising over ..." lines
CITIES = [
    ("Apia", 13), ("Auckland", 12), ("Sydney", 10), ("Tokyo", 9),
    ("Shanghai", 8), ("Bangkok", 7), ("Dhaka", 6), ("Delhi", 5.5),
    ("Dubai", 4), ("Nairobi", 3), ("Istanbul", 3), ("Cairo", 3),
    ("Lagos", 1), ("London", 1), ("Reykjavik", 0), ("Praia", -1),
    ("Rio de Janeiro", -3), ("Buenos Aires", -3), ("Santiago", -4),
    ("New York", -4), ("Chicago", -5), ("Denver", -6),
    ("Los Angeles", -7), ("Anchorage", -8), ("Honolulu", -10),
]


def load_config():
    cfg = dict(DEFAULT_CONFIG)
    user = {}
    try:
        user = json.loads(CONFIG_PATH.read_text())
    except FileNotFoundError:
        pass
    except (json.JSONDecodeError, OSError):
        return cfg
    # migrate v0.1 auto-written defaults to the denser v0.2 feel
    if user.get("density") == 0.45:
        user.pop("density")
    if user.get("message_every_seconds") == 3.0:
        user.pop("message_every_seconds")
    if "env_file" in user:
        user.setdefault("env_files", [user.pop("env_file")])
    if "place" in user:
        old = (user.pop("place") or "").strip()
        if old:
            user.setdefault("places", [old])
    cfg.update(user)
    save_config(cfg)
    return cfg


def save_config(cfg):
    try:
        CONFIG_PATH.parent.mkdir(parents=True, exist_ok=True)
        CONFIG_PATH.write_text(json.dumps(cfg, indent=2) + "\n")
    except OSError:
        pass


def resolve_api_key(cfg):
    key = os.environ.get("EXA_API_KEY")
    if key:
        return key
    for env_file in cfg.get("env_files", []):
        try:
            for line in Path(env_file).expanduser().read_text().splitlines():
                if line.startswith("EXA_API_KEY="):
                    key = line.split("=", 1)[1].strip().strip('"').strip("'")
                    if key:
                        return key
        except OSError:
            continue
    return None


def moon_line():
    ref = datetime(2000, 1, 6, 18, 14, tzinfo=timezone.utc)  # a known new moon
    days = (datetime.now(timezone.utc) - ref).total_seconds() / 86400
    # the mean-synodic model lags the observed phase by about a third of a day
    phase = ((days + 0.35) % 29.530588) / 29.530588
    illum = (1 - math.cos(2 * math.pi * phase)) / 2
    if phase < 0.03 or phase > 0.97:
        name = "new — all shadow"
    elif phase < 0.22:
        name = "a waxing crescent"
    elif phase < 0.28:
        name = "at first quarter"
    elif phase < 0.47:
        name = "waxing gibbous"
    elif phase < 0.53:
        name = "full"
    elif phase < 0.72:
        name = "waning gibbous"
    elif phase < 0.78:
        name = "at last quarter"
    else:
        name = "a waning crescent"
    return f"tonight the moon is {name}, about {round(illum * 20) * 5}% lit"


def poetic_line(started_at):
    """One true thing. Static corpus, live counters, city clocks, sky state."""
    elapsed = time.time() - started_at
    computed = [moon_line()]
    if elapsed > 15:
        computed += [
            f"about {int(elapsed * 4.3):,} people have been born since you opened this window",
            f"lightning has struck the earth about {int(elapsed * 44):,} times since you started watching",
            f"voyager 1 is {int(elapsed * 17):,} km farther from home than when this screen lit up",
            f"the sun has carried you {int(elapsed * 29.8):,} km through space since you began watching",
            f"the earth has turned {elapsed * 360 / 86164:.2f} degrees since this began",
            f"your heart has beaten about {int(elapsed * 1.15):,} times while you watched",
            f"you have blinked about {int(elapsed * 0.28):,} times since this began",
            f"your body has made about {int(elapsed * 2.4):,} million red blood cells while you watched",
        ]
    month = datetime.now(timezone.utc).month
    for months, lines in SEASONAL.items():
        if month in months:
            computed.append(random.choice(lines))
            break
    utc_h = datetime.now(timezone.utc).hour + datetime.now(timezone.utc).minute / 60
    for city, off in random.sample(CITIES, len(CITIES)):
        local = (utc_h + off) % 24
        if 5 <= local < 7:
            computed.append(f"the sun is climbing over {city} about now")
            break
        if 17 <= local < 19:
            computed.append(f"dusk is settling over {city} about now")
            break
        if 0 <= local < 4:
            computed.append(f"it is deep night in {city}, and the city is mostly dreaming")
            break
    if random.random() < 0.5:
        return random.choice(computed)
    return random.choice(random.choice(list(POETIC.values())))


def clean_title(raw):
    """Collapse whitespace and strip trailing publication names ('… | The Verge')."""
    t = " ".join((raw or "").split())
    changed = True
    while changed:
        changed = False
        for sep in (" | ", " — ", " – ", " - "):
            if sep not in t:
                continue
            base, suffix = t.rsplit(sep, 1)
            if base and len(suffix.split()) <= 4 and not any(c.isdigit() for c in suffix):
                t, changed = base, True
                break
    return t


class Newsfeed(threading.Thread):
    """Background Exa fetcher. Never raises into the UI; degrades to poetic-only."""

    def __init__(self, cfg, api_key):
        super().__init__(daemon=True)
        self.cfg = cfg
        self.api_key = api_key
        self.lock = threading.Lock()
        self.wake = threading.Event()
        self.items = []       # [{"text", "url", "kind": "news"|"local"}]
        self.generation = 0   # bumped on every successful fetch
        self.status = "connecting" if api_key else "no api key — poetic only"
        self.fetched_at = None
        try:
            cached = json.loads(CACHE_PATH.read_text())
            if time.time() - cached.get("at", 0) < 86400 and cached.get("items"):
                same_places = self._places_key(cached.get("places", [])) == self._places_key(
                    cfg.get("places", [])
                )
                items = [i for i in cached["items"]
                         if i.get("kind") != "local" or same_places]
                if items:
                    self.items = items
                    self.generation = 1
                    self.status = f"{len(items)} cached headlines"
        except (OSError, json.JSONDecodeError, TypeError):
            pass

    @staticmethod
    def _places_key(places):
        return sorted(str(p).strip().casefold() for p in places if str(p).strip())

    def run(self):
        while True:
            if self.api_key:
                self.fetch()
            self.wake.wait(max(2, self.cfg["refresh_minutes"]) * 60)
            self.wake.clear()

    def _search(self, query, num, category=None, search_type="fast"):
        since = datetime.now(timezone.utc) - timedelta(hours=self.cfg["hours_back"])
        body = {
            "query": query,
            "type": search_type,
            "numResults": num,
            "startPublishedDate": since.strftime("%Y-%m-%dT%H:%M:%S.000Z"),
        }
        if category:
            body["category"] = category
        req = urllib.request.Request(
            "https://api.exa.ai/search",
            data=json.dumps(body).encode(),
            headers={"x-api-key": self.api_key, "content-type": "application/json"},
        )
        with urllib.request.urlopen(req, timeout=20) as resp:
            return json.loads(resp.read()).get("results", [])

    def _collect(self, results, kind, seen):
        out = []
        for r in results:
            title = clean_title(r.get("title"))
            url = r.get("url") or ""
            domain = ""
            if "://" in url:
                domain = url.split("://", 1)[1].split("/", 1)[0].removeprefix("www.")
            # strip a trailing pub name that clean_title kept (e.g. "- France 24",
            # whose digits defeat the generic rule) when it matches the domain root
            root = domain.split(".")[0].replace("-", "").lower() if domain else ""
            if len(root) >= 4:
                for sep in (" | ", " — ", " – ", " - "):
                    if sep in title:
                        base, suffix = title.rsplit(sep, 1)
                        norm = suffix.replace(" ", "").replace("-", "").lower()
                        if base and norm and (root in norm or norm in root):
                            title = base
                            break
            if not title or title.casefold() in seen:
                continue
            seen.add(title.casefold())
            text = f"{title}  ·  {domain}" if domain and self.cfg.get("show_source") else title
            out.append({"text": text, "url": url, "domain": domain, "kind": kind})
        return out

    def fetch(self):
        items, seen = [], set()
        places = [p.strip() for p in self.cfg.get("places", []) if str(p).strip()][:3]
        for place in places:
            try:
                results = self._search(
                    f"{place} — local news, events and what is happening there now", 10
                )
                items += self._collect(results, "local", seen)
            except Exception:
                pass
        # 25 results per request is the top of Exa's cheapest billing tier
        for topic in self.cfg["topics"][:4]:
            query = f"{topic} — the most significant news right now"
            try:
                results = self._search(query, 25, category="news")
            except urllib.error.HTTPError:
                try:
                    results = self._search(query, 25, category="news", search_type="auto")
                except Exception:
                    continue
            except Exception:
                continue
            items += self._collect(results, "news", seen)
        with self.lock:
            if items:
                self.items = items
                self.generation += 1
                self.fetched_at = time.localtime()
                n_local = sum(1 for i in items if i["kind"] == "local")
                self.status = f"{len(items)} headlines" + (
                    f" · {n_local} local ({', '.join(places)})" if places else ""
                )
                try:
                    CACHE_PATH.parent.mkdir(parents=True, exist_ok=True)
                    CACHE_PATH.write_text(
                        json.dumps({"at": time.time(), "places": places, "items": items})
                    )
                except OSError:
                    pass
            elif not self.items:
                self.status = "news offline — poetic only"


# ---------------------------------------------------------------------------
# rendering: raw ANSI so headlines can be real OSC 8 hyperlinks


def sgr(*codes):
    return "\x1b[" + ";".join(map(str, codes)) + "m"


OMARCHY_THEME = Path.home() / ".config" / "omarchy" / "current" / "theme"


def _hex_rgb(s):
    s = s.lstrip("#")
    return (int(s[0:2], 16), int(s[2:4], 16), int(s[4:6], 16))


def _mix(a, b, f):
    return tuple(round(a[i] + (b[i] - a[i]) * f) for i in range(3))


def _fg(rgb, *pre):
    return sgr(0, *pre, 38, 2, *rgb)


def load_omarchy_colors():
    """Colors of the active Omarchy theme (from its alacritty.toml), or None."""
    try:
        import tomllib
        data = tomllib.loads((OMARCHY_THEME / "alacritty.toml").read_text())
        c = data["colors"]
        bright = c.get("bright", {})
        return {
            "name": OMARCHY_THEME.resolve().name,
            "bg": _hex_rgb(c["primary"]["background"]),
            "fg": _hex_rgb(c["primary"]["foreground"]),
            "green": _hex_rgb(c["normal"]["green"]),
            "bgreen": _hex_rgb(bright.get("green", c["normal"]["green"])),
            "yellow": _hex_rgb(c["normal"]["yellow"]),
            "cyan": _hex_rgb(c["normal"]["cyan"]),
            "bwhite": _hex_rgb(bright.get("white", c["primary"]["foreground"])),
        }
    except Exception:
        return None


def build_theme_palette(t, focus=False):
    """Truecolor palette derived from an Omarchy theme: the rain runs in the
    theme's green, fading toward its background; text sits in theme colors."""
    bg, green, fgc = t["bg"], t["green"], t["fg"]

    def g(f):
        return _fg(_mix(green, bg, f))

    pal = {
        "head": _fg(t["bwhite"], 1),
        "trail": [g(0.0), g(0.15), g(0.35), g(0.55), g(0.7), g(0.8)],
        "residue": [g(0.6), g(0.72), g(0.8), _fg(_mix(fgc, bg, 0.85))],
        "reader": _fg(_mix(fgc, bg, 0.12)),
        "dim": _fg(_mix(fgc, bg, 0.5)),
        "blank": sgr(0),
    }
    if focus:
        pal.update(news=_fg(fgc, 1), local=_fg(t["cyan"], 1),
                   poetic=_fg(t["yellow"]), scramble=_fg(t["bwhite"], 1))
    else:
        pal.update(news=_fg(_mix(green, fgc, 0.45)), local=_fg(_mix(t["cyan"], bg, 0.2)),
                   poetic=_fg(_mix(t["yellow"], bg, 0.45)), scramble=_fg(t["bgreen"], 1))
    return pal


def build_palette(basic=False, focus=False, theme=None):
    if theme and not basic:
        return build_theme_palette(theme, focus)
    """focus=False: messages sit embedded in the code, a shade above the field.
    focus=True: headlines surface — bright white, full contrast."""
    if not basic:
        pal = {
            "head": sgr(0, 1, 38, 5, 48),
            "trail": [sgr(0, 38, 5, 46), sgr(0, 38, 5, 40), sgr(0, 38, 5, 34),
                      sgr(0, 38, 5, 28), sgr(0, 38, 5, 22), sgr(0, 2, 38, 5, 22)],
            "residue": [sgr(0, 38, 5, 22), sgr(0, 2, 38, 5, 28),
                        sgr(0, 2, 38, 5, 22), sgr(0, 2, 38, 5, 235)],
            "reader": sgr(0, 38, 5, 250),
            "dim": sgr(0, 38, 5, 241),
            "blank": sgr(0),
        }
        if focus:
            pal.update(news=sgr(0, 1, 38, 5, 255), local=sgr(0, 1, 38, 5, 87),
                       poetic=sgr(0, 38, 5, 222), scramble=sgr(0, 1, 38, 5, 231))
        else:
            pal.update(news=sgr(0, 38, 5, 120), local=sgr(0, 38, 5, 80),
                       poetic=sgr(0, 38, 5, 137), scramble=sgr(0, 1, 38, 5, 83))
        return pal
    g, gd = sgr(0, 32), sgr(0, 2, 32)
    pal = {
        "head": sgr(0, 1, 32),
        "trail": [sgr(0, 1, 32), g, g, gd, gd, gd],
        "residue": [gd],
        "reader": sgr(0, 37),
        "dim": sgr(0, 2, 37),
        "blank": sgr(0),
    }
    if focus:
        pal.update(news=sgr(0, 1, 37), local=sgr(0, 1, 36),
                   poetic=sgr(0, 33), scramble=sgr(0, 1, 37))
    else:
        pal.update(news=sgr(0, 1, 32), local=sgr(0, 36),
                   poetic=sgr(0, 33), scramble=sgr(0, 1, 32))
    return pal


class Term:
    """Minimal raw-terminal layer: alt screen, cbreak keys, buffered writes."""

    def __init__(self):
        self.fd = sys.stdin.fileno()
        self.buf = []
        self.w, self.h = shutil.get_terminal_size()
        self.saved = None

    def enter(self, mouse=True):
        self.saved = termios.tcgetattr(self.fd)
        tty.setcbreak(self.fd)
        self.out("\x1b[?1049h\x1b[?25l\x1b[?7l\x1b[2J")
        if mouse:
            self.out("\x1b[?1000h\x1b[?1006h")
        self.flush()

    def leave(self):
        self.buf.clear()
        self.out("\x1b[?1006l\x1b[?1000l\x1b[0m\x1b[?7h\x1b[?25h\x1b[?1049l")
        self.flush()
        if self.saved:
            termios.tcsetattr(self.fd, termios.TCSADRAIN, self.saved)

    def resize(self):
        self.w, self.h = shutil.get_terminal_size()

    def out(self, s):
        self.buf.append(s)

    def cell(self, y, x, esc, ch):
        if 0 <= y < self.h and 0 <= x < self.w - 1:
            self.buf.append(f"\x1b[{y + 1};{x + 1}H{esc}{ch}")

    def span(self, y, x, esc, text, url=None):
        if not (0 <= y < self.h) or x >= self.w - 1:
            return
        if x < 0:
            text, x = text[-x:], 0
        text = text[: self.w - 1 - x]
        if not text:
            return
        if url:
            text = f"\x1b]8;;{url}\x1b\\{text}\x1b]8;;\x1b\\"
        self.buf.append(f"\x1b[{y + 1};{x + 1}H{esc}{text}")

    def flush(self):
        if self.buf:
            sys.stdout.write("".join(self.buf))
            sys.stdout.flush()
            self.buf.clear()

    def read(self, timeout):
        try:
            r, _, _ = select.select([self.fd], [], [], max(0.0, timeout))
        except (InterruptedError, OSError):
            return b""
        if not r:
            return b""
        try:
            return os.read(self.fd, 256)
        except OSError:
            return b""


def glyph_at(row, x, salt, glyphs):
    return glyphs[(x * 73856093 ^ row * 19349663 ^ salt * 83492791) % len(glyphs)]


def residue_at(row, x, t, glyphs, pal):
    """(esc, glyph) for the settled code field at a cell; evolves slowly."""
    rh = (x * 2654435761 ^ row * 40503 ^ int(t / 7) * 69069) & 0xFFFFFF
    if rh % 100 < 22:
        return pal["blank"], " "
    return pal["residue"][rh % len(pal["residue"])], glyph_at(row, x, (rh >> 8) & 0xFF, glyphs)


class Noise:
    """A stream sweeping left to right. Writers leave settled code behind;
    erasers carve a moving window of darkness that heals after them."""

    def __init__(self, h, w, eraser=False):
        self.row = random.randrange(max(1, h))
        self.eraser = eraser
        self.head = -random.uniform(0, 12)
        self.speed = random.uniform(25, 60) if eraser else random.uniform(15, 45)
        self.length = random.randint(8, max(10, min(34, w // 3)))
        self.last_head = int(self.head)
        self.last_tail = int(self.head) - self.length

    def update(self, dt, mult):
        self.head += self.speed * mult * dt

    def dead(self, w):
        return self.head - self.length > w

    def draw(self, term, t, pal, glyphs, guard):
        hi = int(self.head)
        tail = hi - self.length + 1
        if self.eraser:
            for x in range(self.last_head + 1, hi + 1):
                if not guard(self.row, x):
                    term.cell(self.row, x, pal["blank"], " ")
            for x in range(self.last_tail, tail):
                if not guard(self.row, x):
                    esc, ch = residue_at(self.row, x, t, glyphs, pal)
                    term.cell(self.row, x, esc, ch)
            self.last_head, self.last_tail = hi, max(self.last_tail, tail)
            return
        for x in range(self.last_tail, tail):
            if not guard(self.row, x):
                esc, ch = residue_at(self.row, x, t, glyphs, pal)
                term.cell(self.row, x, esc, ch)
        self.last_head, self.last_tail = hi, max(self.last_tail, tail)
        for d in range(self.length):
            x = hi - d
            if guard(self.row, x):
                continue
            ch = glyph_at(self.row, x, int(t * 2.5 + (x * 7 + self.row * 13) % 23 / 23), glyphs)
            if d == 0:
                term.cell(self.row, x, pal["head"], ch)
            else:
                band = min(len(pal["trail"]) - 1, int(d / self.length * len(pal["trail"])))
                term.cell(self.row, x, pal["trail"][band], ch)


class Message:
    """A headline or poetic line that decodes out of the field, lingers, dissolves."""

    def __init__(self, text, kind, url, row, width, t, x0=None, delay=0.0):
        self.text, self.kind, self.url, self.row = text, kind, url, row
        self.domain = ""
        if x0 is None:
            x0 = random.randint(1, max(1, width - len(text) - 2))
        self.x0 = max(1, min(x0, max(1, width - len(text) - 2)))
        self.speed = random.uniform(32, 48)
        self.phase, self.phase_start = "reveal", t
        self.head = -delay * self.speed
        self.erase = 0.0
        self.dwell = 4.0 + 0.055 * len(text)
        self.done = False

    def span_range(self):
        return self.x0 - 1, self.x0 + len(self.text) + 1

    def update(self, t, dt, mult):
        if self.phase == "reveal":
            self.head += self.speed * mult * dt
            if self.head >= len(self.text) + SCRAMBLE:
                self.phase, self.phase_start = "dwell", t
        elif self.phase == "dwell":
            if t - self.phase_start >= self.dwell:
                self.phase = "erase"
        else:
            self.erase += self.speed * 1.6 * mult * dt
            if self.erase >= len(self.text) + SCRAMBLE:
                self.done = True

    def draw(self, term, t, pal, glyphs):
        attr = {"news": pal["news"], "local": pal["local"],
                "summary": pal["reader"]}.get(self.kind, pal["poetic"])
        n = len(self.text)
        if self.phase == "reveal":
            locked = min(n, int(self.head - SCRAMBLE))
            if locked > 0:
                term.span(self.row, self.x0 - 1, attr, " " + self.text[:locked], self.url)
            for i in range(max(0, locked), min(n, int(self.head))):
                g = glyph_at(self.row, self.x0 + i, int(t * 12), glyphs)
                term.cell(self.row, self.x0 + i, pal["scramble"], g)
        elif self.phase == "dwell":
            term.span(self.row, self.x0 - 1, attr, " " + self.text + " ", self.url)
        else:
            gone = min(n, int(self.erase - SCRAMBLE))
            for x in range(self.x0 - 1, self.x0 + gone + (2 if gone == n else 0)):
                esc, ch = residue_at(self.row, x, t, glyphs, pal)
                term.cell(self.row, x, esc, ch)
            for i in range(max(0, gone), min(n, int(self.erase))):
                g = glyph_at(self.row, self.x0 + i, int(t * 12), glyphs)
                term.cell(self.row, self.x0 + i, pal["trail"][-1], g)
            if int(self.erase) < n:
                start = max(0, int(self.erase))
                term.span(self.row, self.x0 + start, attr, self.text[start:] + " ", self.url)


WAKE = [
    "Wake up, Neo...",
    "The Matrix has you.",
    "Follow the white rabbit.",
    "Knock, knock, Neo.",
]

HELP = [
    " meanwhile — things happening right now ",
    "",
    "  q        quit              space   pause",
    "  click    decode a story    enter   pick one to decode",
    "  t        edit topics       g       edit places (local intel)",
    "  f        focus mode        o       something true now",
    "  m        toggle news       p       toggle poetic",
    "  + / -    speed             r       refresh headlines",
    "  s        status bar        ?       help",
    "",
    "  in editors: type + enter adds · 1-9 removes · esc closes",
    "",
    "  any other key closes help",
]


class App:
    def __init__(self, term, cfg, feed):
        self.term = term
        self.cfg = cfg
        self.feed = feed
        self.basic = "256" not in os.environ.get("TERM", "") and not os.environ.get("COLORTERM")
        self.focus = bool(cfg.get("focus", False))
        self.theme_colors = load_omarchy_colors() if cfg.get("theme", "auto") == "auto" else None
        self.theme_mtime = self._theme_mtime()
        self.pal = build_palette(self.basic, self.focus, self.theme_colors)
        self.glyphs = GLYPHS_ASCII if cfg["ascii_only"] else GLYPHS_KATA
        self.h, self.w = term.h, term.w
        self.streams = []
        self.messages = []
        self.news_q, self.local_q = [], []
        self.gen = 0
        self.recent = []  # first-25-char keys of recently shown poetic lines
        self.started_at = time.time()
        self.next_msg = time.monotonic() + 1.0
        self.paused = False
        self.show_status = False
        self.show_help = False
        self.editor = None  # {"kind": "topics"|"places", "input": str, "pending": bytes}
        self.picker = None            # {"sel": int} while choosing a headline to decode
        self.reader_pending = None    # (request_token, summary) set by the fetch thread
        self.reader_req = 0           # supersedes stale in-flight fetches
        self.wake = None              # the rain speaking; see WAKE
        self.shown_links = []         # recent on-screen headlines, newest first
        self.panel_rect = None  # (y0, x0, y1, x1) of help/editor/picker overlay
        self.news_on = True
        self.poetic_on = True
        self.toast = ("", 0.0)
        self.resized = False
        self._spans = {}

    # -- omarchy live theming ----------------------------------------------
    def _theme_mtime(self):
        try:
            return os.lstat(OMARCHY_THEME).st_mtime
        except OSError:
            return None

    def check_theme(self, t):
        if self.cfg.get("theme", "auto") != "auto":
            return
        mtime = self._theme_mtime()
        if mtime == self.theme_mtime:
            return
        new = load_omarchy_colors()
        if new is None:
            return  # keep the current palette; retry on the next check
        self.theme_mtime = mtime
        if new == self.theme_colors:
            return
        self.theme_colors = new
        self.pal = build_palette(self.basic, self.focus, self.theme_colors)
        self.term.out("\x1b[2J")
        self.flash(f"theme: {new['name']}", t)

    # -- guard: cells that field/streams must not overwrite ----------------
    def guard(self, row, x):
        if row == self.h - 1 and self.show_status:
            return True
        if self.panel_rect:
            y0, x0, y1, x1 = self.panel_rect
            if y0 <= row <= y1 and x0 <= x <= x1:
                return True
        for lo, hi in self._spans.get(row, ()):
            if lo <= x <= hi:
                return True
        return False

    # -- content selection -------------------------------------------------
    def next_headline(self):
        with self.feed.lock:
            if self.feed.generation != self.gen:
                self.gen = self.feed.generation
                self.local_q = [i for i in self.feed.items if i["kind"] == "local"]
                self.news_q = [i for i in self.feed.items if i["kind"] == "news"]
                random.shuffle(self.local_q)
                random.shuffle(self.news_q)
            elif not self.local_q and not self.news_q and self.feed.items:
                self.local_q = [i for i in self.feed.items if i["kind"] == "local"]
                self.news_q = [i for i in self.feed.items if i["kind"] == "news"]
                random.shuffle(self.local_q)
                random.shuffle(self.news_q)
        if self.local_q and (not self.news_q or random.random() < 0.55):
            return self.local_q.pop()
        return self.news_q.pop() if self.news_q else None

    def spawn_message(self, t, force=None):
        if len(self.messages) >= max(4, self.h // 4):
            return
        kind = force
        if kind is None:
            if not (self.news_on or self.poetic_on):
                return
            if self.news_on and (not self.poetic_on or random.random() > self.cfg["poetic_ratio"]):
                kind = "news"
            else:
                kind = "poetic"
        item = self.next_headline() if kind == "news" else None
        domain = ""
        if item is not None:
            text, url, kind = item["text"], item.get("url"), item.get("kind", "news")
            domain = item.get("domain", "")
            if url:
                self.shown_links = ([{"text": text, "url": url, "domain": domain}]
                                    + [l for l in self.shown_links if l["url"] != url])[:9]
        else:
            if kind == "news" and not (self.poetic_on or force):
                return
            for _ in range(8):
                text = poetic_line(self.started_at)
                if text[:25] not in self.recent:
                    break
            self.recent = (self.recent + [text[:25]])[-12:]
            url, kind = None, "poetic"
        if len(text) > self.w - 6:
            text = text[: self.w - 9] + "…"
        taken = {m.row for m in self.messages}
        candidates = [r for r in range(1, self.h - 2) if r not in taken]
        if not candidates:
            return
        spaced = [r for r in candidates if r - 1 not in taken and r + 1 not in taken]
        row = random.choice(spaced or candidates)
        m = Message(text, kind, url, row, self.w, t)
        m.domain = domain
        self.messages.append(m)

    # -- the rain speaking --------------------------------------------------
    def trigger_wake(self, t):
        self.close_panel()
        self.streams = []
        self.messages = []
        self.wake = {"line": 0, "chars": 0.0, "pause_until": 0.0}

    def tick_wake(self, t, dt):
        w = self.wake
        if t < w["pause_until"]:
            if w.get("advance"):
                w.update(advance=False, chars=0.0)
            return
        if w.get("advance"):
            if w["line"] >= len(WAKE) - 1:
                self.wake = None
                self.term.out("\x1b[2J")
                return
            w.update(advance=False, line=w["line"] + 1, chars=0.0)
            self.term.out("\x1b[2J")
        line = WAKE[w["line"]]
        if w["chars"] < len(line):
            w["chars"] += 11 * dt
            if w["chars"] >= len(line):
                w["pause_until"] = t + (2.8 if w["line"] == len(WAKE) - 1 else 1.7)
                w["advance"] = True

    # -- frame -------------------------------------------------------------
    def tick(self, t, dt):
        if self.wake:
            self.tick_wake(t, dt)
            return
        quiet = not (self.picker or self.editor or self.show_help)
        if quiet and random.random() < dt / 3600:  # about once an hour, the rain speaks
            self.trigger_wake(t)
            return
        mult = self.cfg["speed"]
        target = max(8, int(self.h * self.cfg["density"] * 1.8))
        while len(self.streams) < target and random.random() < 0.4:
            self.streams.append(Noise(self.h, self.w, eraser=random.random() < 0.22))
        for s in self.streams:
            s.update(dt, mult)
        for m in self.messages:
            m.update(t, dt, mult)
        if t >= self.next_msg:
            self.spawn_message(t)
            self.next_msg = t + self.cfg["message_every_seconds"] * random.uniform(0.7, 1.4)

    def draw(self, t):
        term, pal = self.term, self.pal
        if self.wake:
            w = self.wake
            line = WAKE[w["line"]][: int(w["chars"])]
            term.span(2, 3, pal["scramble"], line + "█")
            term.flush()
            return
        self._spans = {}
        for m in self.messages:
            self._spans.setdefault(m.row, []).append(m.span_range())
        if self.reader_pending:
            self.mount_summary(t)
        if self.show_help or self.editor or self.picker:
            rect = self.compute_panel_rect()
        else:
            rect = None
        if rect:
            if self.panel_rect and rect != self.panel_rect:
                self.clear_rect(self.panel_rect)
            self.panel_rect = rect
        if not self.paused:
            for _ in range(max(10, (self.w * self.h) // 140)):
                y, x = random.randrange(self.h), random.randrange(max(1, self.w - 1))
                if not self.guard(y, x):
                    esc, ch = residue_at(y, x, t, self.glyphs, pal)
                    term.cell(y, x, esc, ch)
            for s in self.streams:
                s.draw(term, t, pal, self.glyphs, self.guard)
            self.streams = [s for s in self.streams if not s.dead(self.w)]
            for m in self.messages:
                m.draw(term, t, pal, self.glyphs)
            self.messages = [m for m in self.messages if not m.done]
        if self.show_status:
            with self.feed.lock:
                status, at = self.feed.status, self.feed.fetched_at
            when = time.strftime(" · refreshed %H:%M", at) if at else ""
            places = [p for p in self.cfg.get("places", []) if str(p).strip()]
            line = (f" meanwhile · {status}{when} · topics: {', '.join(self.cfg['topics'])}"
                    + (f" · ⌖ {', '.join(places)}" if places else "") + " · q quit ? help ")
            term.span(self.h - 1, 0, pal["dim"], line[: self.w - 1].ljust(self.w - 1))
        msg, until = self.toast
        if msg and t < until:
            term.span(self.h - 1, max(0, self.w - len(msg) - 3), pal["dim"], f" {msg} ")
        if self.show_help or self.editor or self.picker:
            self.draw_panel()
        term.flush()

    # -- summaries: click a headline and its story decodes into the rain ----
    def open_summary(self, link, t):
        self.flash(f"decoding {link['domain'] or 'story'}…", t)
        self.reader_req += 1
        tok = self.reader_req

        def fetch():
            try:
                body = {"ids": [link["url"]],
                        "summary": {"query": "What happened, concretely? 2-4 short sentences."},
                        "livecrawl": "fallback"}
                req = urllib.request.Request(
                    "https://api.exa.ai/contents",
                    data=json.dumps(body).encode(),
                    headers={"x-api-key": self.feed.api_key or "",
                             "content-type": "application/json"},
                )
                with urllib.request.urlopen(req, timeout=25) as resp:
                    results = json.loads(resp.read()).get("results", [])
                r = results[0] if results else {}
                text = (r.get("summary") or "").strip() or "(no summary could be decoded)"
                self.reader_pending = (tok, {
                    "title": clean_title(r.get("title")) or link["text"],
                    "domain": link["domain"], "summary": text, "url": link["url"],
                })
            except Exception:
                self.reader_pending = (tok, {
                    "title": link["text"], "domain": link["domain"], "url": link["url"],
                    "summary": "(the summary could not be decoded — "
                               "shift-click the headline to open the story)",
                })

        threading.Thread(target=fetch, daemon=True).start()

    def mount_summary(self, t):
        (tok, art), self.reader_pending = self.reader_pending, None
        if tok != self.reader_req:
            return
        width = max(30, min(self.w - 16, 72))
        lines = textwrap.wrap(art["summary"], width=width)[:6]
        block = [(art["title"][:width], "news")] + [(s, "summary") for s in lines]
        k = len(block)
        taken = {m.row for m in self.messages}
        r0 = None
        for _ in range(30):
            cand = random.randint(1, max(1, self.h - 2 - k))
            if all((cand + i) not in taken for i in range(k)):
                r0 = cand
                break
        if r0 is None:
            r0 = 1
            self.messages = [m for m in self.messages if not (r0 <= m.row < r0 + k)]
        x0 = max(2, (self.w - width) // 2 + random.randint(-6, 6))
        for i, (text, kind) in enumerate(block):
            m = Message(text, kind, art["url"] if kind == "news" else None,
                        r0 + i, self.w, t, x0=x0, delay=0.35 * i)
            m.domain = art.get("domain", "")
            m.dwell = 10.0 + 0.03 * len(art["summary"])
            self.messages.append(m)

    def click(self, x, y, t):
        for m in self.messages:
            if m.row == y and m.url and m.x0 <= x < m.x0 + len(m.text):
                self.open_summary({"text": m.text, "url": m.url, "domain": m.domain}, t)
                return

    def panel_lines(self):
        if self.show_help:
            return HELP
        if self.picker:
            sel = self.picker["sel"]
            lines = [" ▚ decode a story ", ""]
            for i, l in enumerate(self.shown_links, 1):
                mark = "▸" if i - 1 == sel else " "
                lines.append(f" {mark} {i}  {l['text'][:70]}")
            lines += ["", "   ↑/↓ + enter · or just click a headline in the rain",
                      "   its story decodes into the stream · esc closes"]
            return lines
        ed = self.editor
        kind = ed["kind"]
        title = (" ◈ topics — what the news feed follows "
                 if kind == "topics" else " ⌖ places — local intel ")
        lines = [title, ""]
        entries = self.cfg[kind]
        for i, v in enumerate(entries[:9], 1):
            lines.append(f"   {i}  {v}")
        if not entries:
            lines.append("   (none yet — type one below)")
        lines += ["", f"   ▸ {ed['input']}█", "",
                  "   type + enter adds · 1-9 removes",
                  "   enter on empty line closes · esc closes"]
        return lines

    def compute_panel_rect(self):
        lines = self.panel_lines()
        bw = min(self.w - 2, max(46, max(len(s) for s in lines) + 4))
        bh = len(lines) + 2
        y0, x0 = max(0, (self.h - bh) // 2), max(0, (self.w - bw) // 2)
        return (y0, x0, min(self.h - 1, y0 + bh - 1), min(self.w - 2, x0 + bw - 1))

    def draw_panel(self):
        y0, x0, y1, x1 = self.panel_rect
        bw = x1 - x0 + 1
        for i in range(y1 - y0 + 1):
            self.term.span(y0 + i, x0, self.pal["blank"], " " * bw)
        if self.editor:
            accent = self.pal["local"] if self.editor["kind"] == "places" else self.pal["poetic"]
        else:
            accent = self.pal["news"]
        for i, s in enumerate(self.panel_lines()):
            attr = accent if i == 0 or s.lstrip().startswith("▸") else self.pal["dim"]
            self.term.span(y0 + 1 + i, x0 + 2, attr, s[: bw - 3])

    def clear_rect(self, rect):
        y0, x0, y1, x1 = rect
        for y in range(y0, y1 + 1):
            self.term.span(y, x0, self.pal["blank"], " " * (x1 - x0 + 1))

    def close_panel(self):
        self.editor = None
        self.show_help = False
        self.picker = None
        self.panel_rect = None
        self.term.out("\x1b[2J")  # let the field rebuild — a small reboot

    def flash(self, msg, t):
        self.toast = (msg, t + 2.5)

    def clear_row(self, row):
        self.term.span(row, 0, self.pal["blank"], " " * (self.w - 1))

    # -- input -------------------------------------------------------------
    @staticmethod
    def _tokens(data):
        """Split raw input into ("key", byte) / ("seq", final) / ("mouse", ...)
        tokens so escape sequences never leak bytes into the key handlers."""
        toks, i, n = [], 0, len(data)
        while i < n:
            b = data[i]
            if b == 27 and i + 1 < n and data[i + 1] in (0x5B, 0x4F):
                j = i + 2
                while j < n and not (0x40 <= data[j] <= 0x7E):
                    j += 1
                final = data[j] if j < n else 0
                params = bytes(data[i + 2:j])
                if final in (0x4D, 0x6D) and params.startswith(b"<"):
                    try:
                        btn, mx, my = (int(v) for v in params[1:].split(b";"))
                        toks.append(("mouse", (btn, mx, my, final == 0x4D)))
                    except ValueError:
                        pass
                else:
                    toks.append(("seq", final))
                i = j + 1
            else:
                toks.append(("key", b))
                i += 1
        return toks

    def handle_bytes(self, data, t):
        for kind, val in self._tokens(data):
            is_seq = kind == "seq"
            b = val if kind == "key" else 0
            if kind == "mouse":
                btn, mx, my, press = val
                if press and btn == 0 and not (self.wake or self.editor or self.show_help):
                    self.click(mx - 1, my - 1, t)
                continue
            if self.wake:
                if b in (ord("q"), 27):
                    return False
                continue  # the rain is speaking; let it finish
            if self.picker:
                links = self.shown_links
                if is_seq:
                    if links and val == 0x41:      # up
                        self.picker["sel"] = (self.picker["sel"] - 1) % len(links)
                    elif links and val == 0x42:    # down
                        self.picker["sel"] = (self.picker["sel"] + 1) % len(links)
                    continue
                if b in (13, 10) and links:
                    self.open_summary(links[self.picker["sel"]], t)
                    self.close_panel()
                elif 0x31 <= b <= 0x39 and int(chr(b)) <= len(links):
                    self.open_summary(links[int(chr(b)) - 1], t)
                    self.close_panel()
                else:
                    self.close_panel()
                continue
            if is_seq:
                continue  # arrows etc. only mean something in the picker
            if self.editor is not None:
                ed = self.editor
                entries = self.cfg[ed["kind"]]
                if b in (13, 10):
                    val = ed["input"].strip()
                    if not val:
                        self.close_panel()
                        continue
                    if val.casefold() in ("neo", "wake up", "follow the white rabbit"):
                        self.close_panel()
                        self.trigger_wake(t)
                        continue
                    if val.casefold() not in (str(x).casefold() for x in entries):
                        entries.append(val)
                        save_config(self.cfg)
                        self.feed.wake.set()
                        cap = 4 if ed["kind"] == "topics" else 3
                        note = f" (only the first {cap} are fetched)" if len(entries) > cap else ""
                        self.flash(f"added {val} — refreshing…{note}", t)
                    ed["input"], ed["pending"] = "", b""
                elif b == 27:
                    self.close_panel()
                    return True  # drop any queued escape-sequence bytes
                elif b in (8, 127):
                    ed["input"], ed["pending"] = ed["input"][:-1], b""
                elif 0x31 <= b <= 0x39 and not ed["input"] and int(chr(b)) <= len(entries):
                    gone = entries.pop(int(chr(b)) - 1)
                    save_config(self.cfg)
                    self.feed.wake.set()
                    self.flash(f"removed {gone} — refreshing…", t)
                elif b >= 32:
                    # accumulate UTF-8 so places like Zürich type cleanly
                    ed["pending"] += bytes([b])
                    try:
                        ed["input"] += ed["pending"].decode("utf-8")
                        ed["pending"] = b""
                    except UnicodeDecodeError:
                        if len(ed["pending"]) >= 4:
                            ed["pending"] = b""
                continue
            if self.show_help:
                if b == ord("q"):
                    return False
                self.close_panel()
                continue
            if b in (ord("q"), 27):
                return False
            if b == ord(" "):
                self.paused = not self.paused
                self.flash("paused — space to resume" if self.paused else "", t)
            elif b == ord("n"):
                self.spawn_message(t, force="news")
            elif b == ord("o"):
                self.spawn_message(t, force="poetic")
            elif b == ord("f"):
                self.focus = not self.focus
                self.cfg["focus"] = self.focus
                self.pal = build_palette(self.basic, self.focus, self.theme_colors)
                save_config(self.cfg)
                self.flash("focus — text surfaced" if self.focus else "embedded — text set back", t)
            elif b == ord("t"):
                self.editor = {"kind": "topics", "input": "", "pending": b""}
            elif b == ord("g"):
                self.editor = {"kind": "places", "input": "", "pending": b""}
            elif b in (13, 10, ord("l")):
                if self.shown_links:
                    self.picker = {"sel": 0}
                else:
                    self.flash("no headlines on screen yet", t)
            elif b == ord("m"):
                self.news_on = not self.news_on
                self.flash(f"news {'on' if self.news_on else 'off'}", t)
            elif b == ord("p"):
                self.poetic_on = not self.poetic_on
                self.flash(f"poetic {'on' if self.poetic_on else 'off'}", t)
            elif b in (ord("+"), ord("=")):
                self.cfg["speed"] = min(4.0, self.cfg["speed"] * 1.25)
                self.flash(f"speed {self.cfg['speed']:.2f}x", t)
            elif b == ord("-"):
                self.cfg["speed"] = max(0.2, self.cfg["speed"] / 1.25)
                self.flash(f"speed {self.cfg['speed']:.2f}x", t)
            elif b == ord("r"):
                self.feed.wake.set()
                self.flash("refreshing headlines…", t)
            elif b == ord("s"):
                self.show_status = not self.show_status
                if not self.show_status:
                    self.clear_row(self.h - 1)
            elif b == ord("?"):
                self.show_help = True
        return True

    # -- main loop ---------------------------------------------------------
    def run(self):
        signal.signal(signal.SIGWINCH, lambda *_: setattr(self, "resized", True))
        self.term.enter(mouse=bool(self.cfg.get("mouse", True)))
        try:
            last = time.monotonic()
            frame = 0
            while True:
                t = time.monotonic()
                dt, last = min(t - last, 0.1), t
                frame += 1
                if frame % 90 == 0:
                    self.check_theme(t)
                if self.resized:
                    self.resized = False
                    self.term.resize()
                    self.h, self.w = self.term.h, self.term.w
                    self.messages = [m for m in self.messages
                                     if m.row < self.h - 1 and m.x0 + len(m.text) < self.w - 1]
                    self.streams = [s for s in self.streams if s.row < self.h]
                    self.panel_rect = None
                    self.term.out("\x1b[2J")
                if not self.paused:
                    self.tick(t, dt)
                self.draw(t)
                data = self.term.read((t + 1 / 30) - time.monotonic())
                if data and not self.handle_bytes(data, t):
                    return
        finally:
            self.term.leave()


def main():
    ap = argparse.ArgumentParser(prog="meanwhile", description=__doc__.split("\n")[0])
    ap.add_argument("--offline", action="store_true", help="poetic lines only, no news fetch")
    ap.add_argument("--ascii", action="store_true", help="ASCII glyphs (no katakana)")
    ap.add_argument("--topics", help="comma-separated topics, overrides config")
    ap.add_argument("--places", help="comma-separated places for local intel, overrides config")
    ap.add_argument("--speed", type=float, help="speed multiplier")
    ap.add_argument("--theme", choices=["auto", "matrix"], help="palette source")
    ap.add_argument("--version", action="version", version=f"meanwhile {VERSION}")
    args = ap.parse_args()

    if not sys.stdout.isatty() or not sys.stdin.isatty():
        sys.exit("meanwhile needs an interactive terminal")

    cfg = load_config()
    if args.topics:
        cfg["topics"] = [s.strip() for s in args.topics.split(",") if s.strip()]
    if args.places is not None:
        cfg["places"] = [s.strip() for s in args.places.split(",") if s.strip()]
    if args.speed:
        cfg["speed"] = args.speed
    if args.theme:
        cfg["theme"] = args.theme
    if args.ascii:
        cfg["ascii_only"] = True

    key = None if args.offline else resolve_api_key(cfg)
    feed = Newsfeed(cfg, key)
    feed.start()
    try:
        App(Term(), cfg, feed).run()
    except KeyboardInterrupt:
        pass


if __name__ == "__main__":
    main()
