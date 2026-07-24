use rand::seq::SliceRandom;
use rand::Rng;
use std::collections::HashMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub fn moon_line() -> String {
    // a known new moon
    let ref_ts = 947_182_440i64; // 2000-01-06 18:14 UTC as unix
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let days = (now - ref_ts) as f64 / 86400.0;
    // the mean-synodic model lags the observed phase by about a third of a day
    let phase = ((days + 0.35) % 29.530588) / 29.530588;
    let illum = (1.0 - (2.0 * std::f64::consts::PI * phase).cos()) / 2.0;
    let name = if phase < 0.03 || phase > 0.97 {
        "new — all shadow"
    } else if phase < 0.22 {
        "a waxing crescent"
    } else if phase < 0.28 {
        "at first quarter"
    } else if phase < 0.47 {
        "waxing gibbous"
    } else if phase < 0.53 {
        "full"
    } else if phase < 0.72 {
        "waning gibbous"
    } else if phase < 0.78 {
        "at last quarter"
    } else {
        "a waning crescent"
    };
    format!(
        "tonight the moon is {name}, about {}% lit",
        (illum * 20.0).round() as i32 * 5
    )
}

// (city, rough UTC offset in July)
const CITIES: &[(&str, f64)] = &[
    ("Apia", 13.0),
    ("Auckland", 12.0),
    ("Sydney", 10.0),
    ("Tokyo", 9.0),
    ("Shanghai", 8.0),
    ("Bangkok", 7.0),
    ("Dhaka", 6.0),
    ("Delhi", 5.5),
    ("Dubai", 4.0),
    ("Nairobi", 3.0),
    ("Istanbul", 3.0),
    ("Cairo", 3.0),
    ("Lagos", 1.0),
    ("London", 1.0),
    ("Reykjavik", 0.0),
    ("Praia", -1.0),
    ("Rio de Janeiro", -3.0),
    ("Buenos Aires", -3.0),
    ("Santiago", -4.0),
    ("New York", -4.0),
    ("Chicago", -5.0),
    ("Denver", -6.0),
    ("Los Angeles", -7.0),
    ("Anchorage", -8.0),
    ("Honolulu", -10.0),
];

fn poetic_corpus() -> HashMap<&'static str, Vec<&'static str>> {
    let mut m = HashMap::new();
    m.insert(
        "sky",
        vec![
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
    );
    m.insert(
        "sea",
        vec![
            "half the oxygen in your next breath came from the ocean",
            "the amazon is pouring a fifth of all the world's river water into the atlantic right now",
            "a message in a bottle is floating somewhere at sea right now",
            "every wave that has ever reached a shore was already old when it arrived",
            "the tide is coming in somewhere, always",
            "rain is falling on the open ocean right now, unwatched, in the middle of everything",
            "on some abyssal plain, a fallen whale is feeding a community that will eat for decades",
            "tonight the moon pulls a tide across every shore on earth",
        ],
    );
    m.insert(
        "deep time",
        vec![
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
    );
    m.insert(
        "the body",
        vec![
            "the atoms in your left hand and your right were forged in different stars",
            "your body made about two million red blood cells in the last second",
            "you will blink about twenty thousand times today and remember none of them",
            "the iron in your blood was made in the death of a star",
            "the surface of the earth is carrying you at hundreds of miles an hour, and you cannot feel it",
        ],
    );
    m.insert(
        "creatures",
        vec![
            "bar-tailed godwits can fly eleven days across the pacific without landing once",
            "somewhere over the southern ocean, an albatross is asleep on the wing",
            "somewhere it is daytime, and a hummingbird's heart is doing twenty beats a second",
            "right now, some four hundred million cats are asleep",
            "an arctic tern sees more daylight than any creature on earth; one is in the light right now",
        ],
    );
    m.insert(
        "people",
        vec![
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
    );
    m
}

fn seasonal_for_month(month: u32) -> Option<&'static [&'static str]> {
    match month {
        12 | 1 | 2 => Some(&[
            "it is high summer in antarctica; the sun circles the sky without setting",
            "in the far north the sun barely clears the horizon, and the snow holds the light all day",
        ]),
        3 | 4 | 5 => Some(&[
            "the cherry-blossom front is moving north through japan about now",
            "across the north, billions of birds are flying home for the spring",
        ]),
        6 | 7 | 8 => Some(&[
            "above the arctic circle, the sun has not set for weeks",
            "it is midwinter in patagonia; snow is settling on the andes",
            "humpback whales are gathering in the warm seas off madagascar to breed",
        ]),
        9 | 10 | 11 => Some(&[
            "monarch butterflies are streaming south across america toward mexico",
            "across the north, the forests are turning gold a little further south each day",
        ]),
        _ => None,
    }
}

fn utc_now_parts() -> (u32, f64) {
    // month (1-12) and fractional UTC hour
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // crude civil calendar from unix days (good enough for month / hour)
    let days = (secs / 86400) as i64;
    let day_secs = secs % 86400;
    let hour = day_secs as f64 / 3600.0;

    // civil_from_days (Howard Hinnant)
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (m as u32, hour)
}

/// One true thing. Static corpus, live counters, city clocks, sky state.
pub fn poetic_line(_started_at: SystemTime, elapsed: Duration) -> String {
    let mut rng = rand::thread_rng();
    let elapsed_s = elapsed.as_secs_f64();
    let mut computed = vec![moon_line()];

    if elapsed_s > 15.0 {
        computed.push(format!(
            "about {} people have been born since you opened this window",
            format_int((elapsed_s * 4.3) as i64)
        ));
        computed.push(format!(
            "lightning has struck the earth about {} times since you started watching",
            format_int((elapsed_s * 44.0) as i64)
        ));
        computed.push(format!(
            "voyager 1 is {} km farther from home than when this screen lit up",
            format_int((elapsed_s * 17.0) as i64)
        ));
        computed.push(format!(
            "the sun has carried you {} km through space since you began watching",
            format_int((elapsed_s * 29.8) as i64)
        ));
        computed.push(format!(
            "the earth has turned {:.2} degrees since this began",
            elapsed_s * 360.0 / 86164.0
        ));
        computed.push(format!(
            "your heart has beaten about {} times while you watched",
            format_int((elapsed_s * 1.15) as i64)
        ));
        computed.push(format!(
            "you have blinked about {} times since this began",
            format_int((elapsed_s * 0.28) as i64)
        ));
        computed.push(format!(
            "your body has made about {} million red blood cells while you watched",
            format_int((elapsed_s * 2.4) as i64)
        ));
    }

    let (month, utc_h) = utc_now_parts();
    if let Some(lines) = seasonal_for_month(month) {
        if let Some(line) = lines.choose(&mut rng) {
            computed.push((*line).to_string());
        }
    }

    let mut cities: Vec<_> = CITIES.to_vec();
    cities.shuffle(&mut rng);
    for (city, off) in cities {
        let local = (utc_h + off).rem_euclid(24.0);
        if (5.0..7.0).contains(&local) {
            computed.push(format!("the sun is climbing over {city} about now"));
            break;
        }
        if (17.0..19.0).contains(&local) {
            computed.push(format!("dusk is settling over {city} about now"));
            break;
        }
        if (0.0..4.0).contains(&local) {
            computed.push(format!("it is deep night in {city}, and the city is mostly dreaming"));
            break;
        }
    }

    if rng.gen_bool(0.5) && !computed.is_empty() {
        return computed.choose(&mut rng).unwrap().clone();
    }

    let corpus = poetic_corpus();
    let categories: Vec<_> = corpus.values().collect();
    let cat = categories.choose(&mut rng).unwrap();
    (*cat.choose(&mut rng).unwrap()).to_string()
}

fn format_int(n: i64) -> String {
    let s = n.abs().to_string();
    let mut out = String::new();
    for (i, c) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    let mut rev: String = out.chars().rev().collect();
    if n < 0 {
        rev.insert(0, '-');
    }
    rev
}

#[allow(dead_code)]
pub fn started_elapsed(started_at: SystemTime) -> Duration {
    SystemTime::now()
        .duration_since(started_at)
        .unwrap_or_default()
}
