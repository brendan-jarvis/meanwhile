#!/usr/bin/env python3
"""meanwhile — horizontal matrix rain of things happening right now.

Noise streams sweep across the terminal like cmatrix turned on its side.
Out of the noise, real things decode into view: live news headlines
(via the Exa API) in white, and true, quietly poetic facts about what is
happening somewhere on Earth right now in amber.

stdlib only. Run: meanwhile
"""

import argparse
import curses
import json
import locale
import os
import random
import threading
import time
import urllib.error
import urllib.request
from datetime import datetime, timedelta, timezone
from pathlib import Path

VERSION = "0.1.0"
CONFIG_PATH = Path.home() / ".config" / "meanwhile" / "config.json"

DEFAULT_CONFIG = {
    "topics": ["world news", "artificial intelligence", "uk"],
    "refresh_minutes": 15,
    "hours_back": 36,
    "poetic_ratio": 0.45,
    "message_every_seconds": 3.0,
    "density": 0.45,
    "speed": 1.0,
    "ascii_only": False,
    "env_files": ["~/dev/tom-os/.env", "~/dev/exa-newsdesk/.env"],
}

GLYPHS_KATA = "ｦｧｨｩｪｫｬｭｮｯｰｱｲｳｴｵｶｷｸｹｺｻｼｽｾｿﾀﾁﾂﾃﾄﾅﾆﾇﾈﾉﾊﾋﾌﾍﾎﾏﾐﾑﾒﾓﾔﾕﾖﾗﾘﾙﾚﾛﾜﾝ0123456789:･=*+<>"
GLYPHS_ASCII = "abcdefghijklmnopqrstuvwxyz0123456789@#$%&*+=<>:~"
SCRAMBLE = 4  # cells of un-decoded shimmer at a message head

POETIC = [
    "right now, about a million people are in the air",
    "the light warming earth at this moment left the sun eight minutes ago",
    "a photon born in the sun's core wandered a hundred thousand years to its surface — then took eight minutes to reach you",
    "about two thousand thunderstorms are rolling across the planet right now",
    "tonight the moon pulls a tide across every shore on earth",
    "the ISS circles the earth every 92 minutes; its crew sees sixteen sunrises a day",
    "somewhere, a baby just took their first breath",
    "somewhere, a library just opened for the morning",
    "every second, the sun turns four million tonnes of itself into light",
    "somewhere, rain that fell a thousand years ago is melting out of a glacier",
    "there are more trees on earth than stars in the milky way",
    "mount everest grows about four millimetres taller every year",
    "the moon drifts 3.8 centimetres farther from earth every year, and still it holds the tides",
    "somewhere in california, a tree older than the pyramids is quietly adding a ring",
    "beneath your feet spins an iron core nearly the size of the moon",
    "the amazon is pouring a fifth of all the world's river water into the atlantic right now",
    "about a hundred tonnes of stardust will settle on the earth today",
    "somewhere, two people who will love each other haven't met yet",
    "in the arctic, greenland sharks born before newton are still swimming",
    "east of the dateline it is yesterday; west of it, tomorrow is already underway",
    "right now, someone is laughing so hard they can't breathe",
    "eight billion hearts are beating at this moment, hardly any of them in step",
    "the wheat in your last meal was sown by someone you will never meet",
    "antarctica is growing a skirt of sea ice the size of itself, as it does every winter",
    "somewhere, a night-shift nurse is checking on a ward of sleeping strangers",
    "satellites are photographing the earth right now; you may be in one of the pictures",
    "the pole star's light arriving tonight left it around the year 1600",
    "on the day side of earth, four billion people are going about their morning",
    "somewhere, a fisherman is hauling nets under stars you have never seen",
    "there is a desert where it has not rained in five hundred years; it is probably sunny there today",
    "deep in a swedish forest, a spruce root system has been alive for nine and a half thousand years",
    "the atoms in your left hand and your right were forged in different stars",
    "a message in a bottle is floating somewhere at sea right now",
    "somewhere, a teacher just watched an idea land",
    "half the oxygen in your next breath came from the ocean",
    "bar-tailed godwits can fly eleven days across the pacific without landing once",
    "somewhere, an orchestra is tuning to the same A it has been for two centuries",
    "every wave that has ever reached a shore was already old when it arrived",
]

# (city, rough UTC offset in July) — for "the sun is rising over ..." lines
CITIES = [
    ("Apia", 13), ("Auckland", 12), ("Sydney", 10), ("Tokyo", 9),
    ("Shanghai", 8), ("Bangkok", 7), ("Dhaka", 6), ("Delhi", 5.5),
    ("Dubai", 4), ("Nairobi", 3), ("Istanbul", 3), ("Cairo", 2),
    ("Lagos", 1), ("London", 1), ("Reykjavik", 0), ("Praia", -1),
    ("Rio de Janeiro", -3), ("Buenos Aires", -3), ("Santiago", -4),
    ("New York", -4), ("Chicago", -5), ("Denver", -6),
    ("Los Angeles", -7), ("Anchorage", -8), ("Honolulu", -10),
]


def load_config():
    cfg = dict(DEFAULT_CONFIG)
    try:
        cfg.update(json.loads(CONFIG_PATH.read_text()))
    except FileNotFoundError:
        CONFIG_PATH.parent.mkdir(parents=True, exist_ok=True)
        CONFIG_PATH.write_text(json.dumps(DEFAULT_CONFIG, indent=2) + "\n")
    except (json.JSONDecodeError, OSError):
        pass
    return cfg


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


def poetic_line(started_at):
    """One true thing. Mix of a static corpus, live counters, and city clocks."""
    elapsed = time.time() - started_at
    computed = []
    if elapsed > 15:
        computed += [
            f"about {int(elapsed * 4.3):,} people have been born since you opened this window",
            f"lightning has struck the earth about {int(elapsed * 44):,} times since you started watching",
            f"voyager 1 is {int(elapsed * 17):,} km farther from home than when this screen lit up",
            f"the sun has carried you {int(elapsed * 29.8):,} km through space since you began watching",
            f"the earth has turned {elapsed * 360 / 86164:.2f} degrees since this began",
        ]
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
    if computed and random.random() < 0.45:
        return random.choice(computed)
    return random.choice(POETIC)


class Newsfeed(threading.Thread):
    """Background Exa fetcher. Never raises into the UI; degrades to poetic-only."""

    def __init__(self, cfg, api_key):
        super().__init__(daemon=True)
        self.cfg = cfg
        self.api_key = api_key
        self.lock = threading.Lock()
        self.wake = threading.Event()
        self.headlines = []
        self.status = "connecting" if api_key else "no api key — poetic only"
        self.fetched_at = None

    def run(self):
        while True:
            if self.api_key:
                self.fetch()
            self.wake.wait(max(2, self.cfg["refresh_minutes"]) * 60)
            self.wake.clear()

    def _search(self, topic, search_type):
        since = datetime.now(timezone.utc) - timedelta(hours=self.cfg["hours_back"])
        body = {
            "query": f"{topic} — the most significant news right now",
            "type": search_type,
            "category": "news",
            "numResults": max(4, 24 // max(1, len(self.cfg["topics"]))),
            "startPublishedDate": since.strftime("%Y-%m-%dT%H:%M:%S.000Z"),
        }
        req = urllib.request.Request(
            "https://api.exa.ai/search",
            data=json.dumps(body).encode(),
            headers={"x-api-key": self.api_key, "content-type": "application/json"},
        )
        with urllib.request.urlopen(req, timeout=20) as resp:
            return json.loads(resp.read()).get("results", [])

    def fetch(self):
        items, seen = [], set()
        for topic in self.cfg["topics"][:4]:
            try:
                results = self._search(topic, "fast")
            except urllib.error.HTTPError:
                try:
                    results = self._search(topic, "auto")
                except Exception:
                    continue
            except Exception:
                continue
            for r in results:
                title = " ".join((r.get("title") or "").split())
                if not title or title.casefold() in seen:
                    continue
                seen.add(title.casefold())
                domain = ""
                url = r.get("url") or ""
                if "://" in url:
                    domain = url.split("://", 1)[1].split("/", 1)[0].removeprefix("www.")
                items.append(f"{title}  ·  {domain}" if domain else title)
        with self.lock:
            if items:
                random.shuffle(items)
                self.headlines = items
                self.fetched_at = time.localtime()
                self.status = f"{len(items)} headlines"
            elif not self.headlines:
                self.status = "news offline — poetic only"


class Noise:
    """A shimmering stream of glyphs sweeping left to right."""

    def __init__(self, row, width):
        self.row = row
        self.head = -random.uniform(0, 10)
        self.speed = random.uniform(15, 45)
        self.length = random.randint(6, max(8, min(30, width // 3)))

    def update(self, dt, mult):
        self.head += self.speed * mult * dt

    def dead(self, width):
        return self.head - self.length > width

    def draw(self, put, t, pal, glyphs):
        for d in range(self.length):
            x = int(self.head) - d
            frac = d / self.length
            k = int(t * 2.5 + (x * 7 + self.row * 13) % 23 / 23)
            ch = glyphs[(x * 73856093 ^ self.row * 19349663 ^ k * 83492791) % len(glyphs)]
            band = 0 if d == 0 else min(len(pal["noise"]) - 1, 1 + int(frac * (len(pal["noise"]) - 1)))
            put(self.row, x, ch, pal["noise"][band])


class Message:
    """A headline or poetic line that decodes out of the noise, lingers, dissolves."""

    def __init__(self, text, kind, row, width, t):
        self.text = text
        self.kind = kind  # "news" | "poetic"
        self.row = row
        self.x0 = random.randint(1, max(1, width - len(text) - 2))
        self.speed = random.uniform(30, 45)
        self.phase, self.phase_start = "reveal", t
        self.head = 0.0
        self.erase = 0.0
        self.dwell = 3.5 + 0.05 * len(text)
        self.done = False

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

    def draw(self, put, t, pal, glyphs):
        text_attr = pal["news"] if self.kind == "news" else pal["poetic"]
        for i, ch in enumerate(self.text):
            x = self.x0 + i
            if self.phase == "reveal":
                if i < self.head - SCRAMBLE:
                    put(self.row, x, ch, text_attr)
                elif i < self.head:
                    g = glyphs[(x * 73856093 ^ int(t * 12) * 83492791) % len(glyphs)]
                    put(self.row, x, g, pal["scramble"])
            elif self.phase == "dwell":
                put(self.row, x, ch, text_attr)
            else:
                if i < self.erase - SCRAMBLE:
                    continue
                elif i < self.erase:
                    g = glyphs[(x * 73856093 ^ int(t * 12) * 83492791) % len(glyphs)]
                    put(self.row, x, g, pal["fade"])
                else:
                    put(self.row, x, ch, text_attr)


def build_palette():
    curses.start_color()
    curses.use_default_colors()
    pairs = {}

    def mk(fg):
        if fg not in pairs:
            pairs[fg] = len(pairs) + 1
            curses.init_pair(pairs[fg], fg, -1)
        return curses.color_pair(pairs[fg])

    italic = getattr(curses, "A_ITALIC", 0)
    if curses.COLORS >= 256:
        return {
            "noise": [mk(48) | curses.A_BOLD, mk(46), mk(40), mk(34), mk(28), mk(22), mk(22) | curses.A_DIM],
            "news": mk(255) | curses.A_BOLD,
            "poetic": mk(222) | italic,
            "scramble": mk(231) | curses.A_BOLD,
            "fade": mk(22) | curses.A_DIM,
            "dim": mk(241),
        }
    g, w, y = curses.COLOR_GREEN, curses.COLOR_WHITE, curses.COLOR_YELLOW
    return {
        "noise": [mk(g) | curses.A_BOLD, mk(g) | curses.A_BOLD, mk(g), mk(g), mk(g) | curses.A_DIM, mk(g) | curses.A_DIM],
        "news": mk(w) | curses.A_BOLD,
        "poetic": mk(y) | italic,
        "scramble": mk(w) | curses.A_BOLD,
        "fade": mk(g) | curses.A_DIM,
        "dim": mk(w) | curses.A_DIM,
    }


class App:
    def __init__(self, scr, cfg, feed):
        self.scr = scr
        self.cfg = cfg
        self.feed = feed
        self.pal = build_palette()
        self.glyphs = GLYPHS_ASCII if cfg["ascii_only"] else GLYPHS_KATA
        self.h, self.w = scr.getmaxyx()
        self.noise = {}      # row -> Noise
        self.messages = []   # list[Message]
        self.queue = []      # unshown headlines
        self.started_at = time.time()
        self.next_msg = time.monotonic() + 1.5
        self.recent = []  # first-25-chars keys of recently shown lines
        self.paused = False
        self.show_status = False
        self.show_help = False
        self.news_on = True
        self.poetic_on = True
        self.toast = ("", 0.0)

    def put(self, y, x, ch, attr):
        if 0 <= y < self.h and 0 <= x < self.w - 1:
            try:
                self.scr.addstr(y, x, ch, attr)
            except curses.error:
                pass

    # -- content selection -------------------------------------------------
    def next_headline(self):
        if not self.queue:
            with self.feed.lock:
                self.queue = list(self.feed.headlines)
            random.shuffle(self.queue)
        return self.queue.pop() if self.queue else None

    def spawn_message(self, t, force=None):
        kind = force
        if kind is None:
            if not (self.news_on or self.poetic_on):
                return
            if self.news_on and (not self.poetic_on or random.random() > self.cfg["poetic_ratio"]):
                kind = "news"
            else:
                kind = "poetic"
        text = self.next_headline() if kind == "news" else None
        if text is None:
            if kind == "news" and not (self.poetic_on or force):
                return
            for _ in range(8):
                text = poetic_line(self.started_at)
                if text[:25] not in self.recent:
                    break
            kind = "poetic"
        self.recent = (self.recent + [text[:25]])[-12:]
        if len(text) > self.w - 6:
            text = text[: self.w - 9] + "…"
        taken = {m.row for m in self.messages}
        candidates = [r for r in range(1, self.h - 1) if r not in taken]
        if not candidates:
            return
        spaced = [r for r in candidates if r - 1 not in taken and r + 1 not in taken]
        row = random.choice(spaced or candidates)
        self.noise.pop(row, None)
        self.messages.append(Message(text, kind, row, self.w, t))

    # -- frame -------------------------------------------------------------
    def tick(self, t, dt):
        mult = self.cfg["speed"]
        target = int(self.h * self.cfg["density"])
        if len(self.noise) < target:
            free = [r for r in range(self.h) if r not in self.noise and r not in {m.row for m in self.messages}]
            if free and random.random() < min(1.0, (target - len(self.noise)) * 0.15):
                r = random.choice(free)
                self.noise[r] = Noise(r, self.w)
        for r in [r for r, n in self.noise.items() if (n.update(dt, mult) or n.dead(self.w))]:
            del self.noise[r]
        for m in self.messages:
            m.update(t, dt, mult)
        self.messages = [m for m in self.messages if not m.done]
        if t >= self.next_msg:
            self.spawn_message(t)
            self.next_msg = t + self.cfg["message_every_seconds"] * random.uniform(0.7, 1.4)

    def draw(self, t):
        self.scr.erase()
        for n in self.noise.values():
            n.draw(self.put, t, self.pal, self.glyphs)
        for m in self.messages:
            m.draw(self.put, t, self.pal, self.glyphs)
        if self.show_status:
            with self.feed.lock:
                status, at = self.feed.status, self.feed.fetched_at
            when = time.strftime(" · refreshed %H:%M", at) if at else ""
            topics = ", ".join(self.cfg["topics"])
            line = f" meanwhile · {status}{when} · topics: {topics} · q quit ? help "
            self.put(self.h - 1, 0, line[: self.w - 2], self.pal["dim"])
        msg, until = self.toast
        if msg and t < until:
            self.put(self.h - 1, max(0, self.w - len(msg) - 2), msg, self.pal["dim"])
        if self.show_help:
            self.draw_help()
        self.scr.refresh()

    def draw_help(self):
        lines = [
            " meanwhile — things happening right now ",
            "",
            "  q       quit            space   pause",
            "  n       a headline now  o       something true now",
            "  m       toggle news     p       toggle poetic",
            "  + / -   speed           r       refresh headlines",
            "  s       status bar      ?       close help",
        ]
        bw = max(len(s) for s in lines) + 4
        bh = len(lines) + 2
        y0, x0 = max(0, (self.h - bh) // 2), max(0, (self.w - bw) // 2)
        for i in range(bh):
            self.put(y0 + i, x0, " " * bw, curses.A_NORMAL)
        for i, s in enumerate(lines):
            self.put(y0 + 1 + i, x0 + 2, s, self.pal["news"] if i == 0 else self.pal["dim"])

    def flash(self, msg, t):
        self.toast = (msg, t + 2.0)

    def handle_key(self, ch, t):
        if ch in (ord("q"), ord("Q"), 27):
            return False
        if self.show_help:
            self.show_help = False
            return True
        if ch == ord(" "):
            self.paused = not self.paused
            self.flash("paused" if self.paused else "", t)
        elif ch == ord("n"):
            self.spawn_message(t, force="news")
        elif ch == ord("o"):
            self.spawn_message(t, force="poetic")
        elif ch == ord("m"):
            self.news_on = not self.news_on
            self.flash(f"news {'on' if self.news_on else 'off'}", t)
        elif ch == ord("p"):
            self.poetic_on = not self.poetic_on
            self.flash(f"poetic {'on' if self.poetic_on else 'off'}", t)
        elif ch in (ord("+"), ord("=")):
            self.cfg["speed"] = min(4.0, self.cfg["speed"] * 1.25)
            self.flash(f"speed {self.cfg['speed']:.2f}x", t)
        elif ch == ord("-"):
            self.cfg["speed"] = max(0.2, self.cfg["speed"] / 1.25)
            self.flash(f"speed {self.cfg['speed']:.2f}x", t)
        elif ch == ord("r"):
            self.feed.wake.set()
            self.flash("refreshing headlines…", t)
        elif ch == ord("s"):
            self.show_status = not self.show_status
        elif ch == ord("?"):
            self.show_help = True
        elif ch == curses.KEY_RESIZE:
            self.h, self.w = self.scr.getmaxyx()
            self.noise = {r: n for r, n in self.noise.items() if r < self.h}
            self.messages = [m for m in self.messages if m.row < self.h - 1 and m.x0 + len(m.text) < self.w]
        return True

    def run(self):
        curses.curs_set(0)
        self.scr.nodelay(True)
        self.scr.keypad(True)
        last = time.monotonic()
        while True:
            t = time.monotonic()
            dt, last = min(t - last, 0.1), t
            ch = self.scr.getch()
            if ch != -1 and not self.handle_key(ch, t):
                return
            if not self.paused:
                self.tick(t, dt)
            self.draw(t)
            time.sleep(max(0.0, 1 / 30 - (time.monotonic() - t)))


def main():
    ap = argparse.ArgumentParser(prog="meanwhile", description=__doc__.split("\n")[0])
    ap.add_argument("--offline", action="store_true", help="poetic lines only, no news fetch")
    ap.add_argument("--ascii", action="store_true", help="ASCII glyphs (no katakana)")
    ap.add_argument("--topics", help="comma-separated topics, overrides config")
    ap.add_argument("--speed", type=float, help="speed multiplier")
    ap.add_argument("--version", action="version", version=f"meanwhile {VERSION}")
    args = ap.parse_args()

    locale.setlocale(locale.LC_ALL, "")
    cfg = load_config()
    if args.topics:
        cfg["topics"] = [s.strip() for s in args.topics.split(",") if s.strip()]
    if args.speed:
        cfg["speed"] = args.speed
    if args.ascii:
        cfg["ascii_only"] = True

    key = None if args.offline else resolve_api_key(cfg)
    feed = Newsfeed(cfg, key)
    feed.start()
    try:
        curses.wrapper(lambda scr: App(scr, cfg, feed).run())
    except KeyboardInterrupt:
        pass


if __name__ == "__main__":
    main()
