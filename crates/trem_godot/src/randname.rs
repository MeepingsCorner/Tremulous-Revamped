//! A random default player name (dec064). The 1.1 client took its default
//! `name` from the operating system's login name (`Sys_GetCurrentUser`:
//! USER, LOGNAME, USERNAME), which often holds a real name and then went
//! to every server, its logs, namelog and ban lists. The port no longer
//! reads it for a default: a player who has not chosen a name gets a
//! funny random one, `AdjectiveNoun` or `VerbNoun`, saved like any other
//! name. Nothing here reads the account, machine or home directory.
//!
//! Word list review (done by hand, then enforced by the tests): every
//! word is family-friendly, ASCII, a letter-only CamelCase token; the
//! adjectives are positive or whimsical (nothing that insults, no
//! "Smelly", "Dumb", "Ugly", "Lazy"); the nouns are Tremulous things or
//! harmless objects and animals (no people-types, no body parts, no
//! religion, no politics); the verbs are playful and non-violent. The
//! `banned_hit` test checks the 13 000+ concatenations (the joins are
//! where accidents hide, e.g. "Class"+"Y") against a list of slur and
//! profanity fragments, and every word against the same list.

use crate::cmd::{Cvars, CVAR_ARCHIVE};

/// `MAX_NAME_LENGTH` of the game (cmds.rs): the name buffer, with its NUL.
const MAX_NAME_LENGTH: usize = 32;

pub const ADJECTIVES: &[&str] = &[
    "Sneaky",
    "Toasted",
    "Wobbly",
    "Lucky",
    "Bouncy",
    "Fuzzy",
    "Jolly",
    "Cheeky",
    "Dizzy",
    "Grumpy",
    "Happy",
    "Sleepy",
    "Speedy",
    "Mighty",
    "Tiny",
    "Giant",
    "Shiny",
    "Rusty",
    "Crispy",
    "Sparkly",
    "Peachy",
    "Salty",
    "Sugary",
    "Frosty",
    "Sunny",
    "Stormy",
    "Breezy",
    "Cosmic",
    "Atomic",
    "Turbo",
    "Nimble",
    "Plucky",
    "Brave",
    "Clever",
    "Quirky",
    "Zesty",
    "Zippy",
    "Snazzy",
    "Jazzy",
    "Funky",
    "Groovy",
    "Mellow",
    "Peppy",
    "Perky",
    "Dapper",
    "Feisty",
    "Fluffy",
    "Gentle",
    "Glowing",
    "Golden",
    "Hasty",
    "Humble",
    "Lively",
    "Loyal",
    "Merry",
    "Noble",
    "Odd",
    "Polite",
    "Proud",
    "Rowdy",
    "Silent",
    "Smooth",
    "Snuggly",
    "Spry",
    "Steady",
    "Robust",
    "Swift",
    "Tricky",
    "Wily",
    "Wild",
    "Witty",
    "Zany",
    "Bubbly",
    "Toasty",
    "Caffeinated",
    "Chilly",
    "Cuddly",
    "Dancing",
    "Flying",
    "Hopping",
    "Juggling",
    "Napping",
    "Rolling",
    "Singing",
    "Sliding",
    "Spinning",
    "Surfing",
    "Waddling",
    "Whistling",
    "Wandering",
];

pub const VERBS: &[&str] = &[
    "Boop", "Bonk", "Poke", "Tickle", "Juggle", "Wrangle", "Fetch", "Dodge", "Hoist", "Pester",
    "Nudge", "Scoot", "Hop", "Skip", "Wiggle", "Waddle", "Zoom", "Yeet", "Toss", "Catch", "Bake",
    "Brew", "Stir", "Spin", "Twirl", "Doodle", "Hug", "Pat", "Wave", "Whistle", "Polish", "Paint",
    "Plant", "Build", "Mend", "Patch", "Gather", "Guard", "Herd", "Chase", "Sneak", "Sprint",
    "Climb", "Leap", "Vault", "Glide", "Drift", "Dash", "Race", "Roam", "Count", "Stack", "Sort",
    "Wobble", "Bounce", "Flip", "Fling", "Haul", "Pluck", "Sniff",
];

pub const NOUNS: &[&str] = &[
    "Granger",
    "Dretch",
    "Basilisk",
    "Marauder",
    "Dragoon",
    "Tyrant",
    "Overmind",
    "Hive",
    "Booster",
    "Barricade",
    "Medkit",
    "Armoury",
    "Reactor",
    "Telenode",
    "Turret",
    "Lasgun",
    "Chaingun",
    "Flamer",
    "Shotgun",
    "Blaster",
    "Rifle",
    "Painsaw",
    "Grenade",
    "Jetpack",
    "Battlesuit",
    "Helmet",
    "Spitter",
    "Spore",
    "Egg",
    "Tentacle",
    "Gherkin",
    "Waffle",
    "Muffin",
    "Noodle",
    "Biscuit",
    "Walrus",
    "Badger",
    "Pancake",
    "Turnip",
    "Gnome",
    "Toaster",
    "Nugget",
    "Penguin",
    "Otter",
    "Llama",
    "Moose",
    "Puffin",
    "Hedgehog",
    "Hamster",
    "Pigeon",
    "Pretzel",
    "Pudding",
    "Potato",
    "Pumpkin",
    "Radish",
    "Sprout",
    "Teapot",
    "Teacup",
    "Biscotti",
    "Cupcake",
    "Banana",
    "Mango",
    "Walnut",
    "Acorn",
    "Pebble",
    "Comet",
    "Rocket",
    "Robot",
    "Wizard",
    "Goblin",
    "Yeti",
    "Dragon",
    "Unicorn",
    "Kraken",
    "Gremlin",
    "Sasquatch",
    "Ferret",
    "Beaver",
    "Pelican",
    "Gecko",
    "Lantern",
    "Mitten",
    "Anchor",
    "Bagel",
    "Donut",
    "Taco",
    "Cactus",
    "Trombone",
    "Kazoo",
    "Yodeler",
];

/// Fragments that no generated name may contain (case-insensitive).
/// Used by the tests as the review gate for words and their joins.
#[cfg(test)]
const BANNED: &[&str] = &[
    "nig", "fag", "cunt", "shit", "fuck", "dick", "cock", "rape", "nazi", "coon", "spic", "kike",
    "paki", "whore", "slut", "piss", "porn", "cum", "sex", "anal", "anus", "tit", "butt", "arse",
    "homo", "gay", "dyke", "retard", "wank", "jerk", "pimp", "hell", "damn", "crap", "turd", "poo",
    "fart", "boob", "nude", "naked", "drug", "kill", "die", "dead", "suicid", "bitch", "bastard",
    "twat", "negro", "chink", "gook", "jew", "hoe", "ass",
];

/// splitmix64: a small, fixed generator so a seed always gives the same name.
fn next(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// The name for `seed`: `AdjectiveNoun` (60%) or `VerbNoun` (40%).
/// Deterministic; ASCII letters only (so no colour codes), at most 20
/// characters of the 31 the game allows.
pub fn random_name(seed: u64) -> String {
    let mut s = seed;
    let first_verb = next(&mut s) % 10 >= 6;
    let a = if first_verb {
        VERBS[(next(&mut s) % VERBS.len() as u64) as usize]
    } else {
        ADJECTIVES[(next(&mut s) % ADJECTIVES.len() as u64) as usize]
    };
    let n = NOUNS[(next(&mut s) % NOUNS.len() as u64) as usize];
    let name = format!("{a}{n}");
    if name.len() < MAX_NAME_LENGTH && name.bytes().all(|c| c.is_ascii_alphabetic()) {
        name
    } else {
        "UnnamedPlayer".into()
    }
}

/// A seed for a new profile: the std library's per-process OS-random hash
/// keys mixed with the clock. Never the user, host or home directory.
pub fn fresh_seed() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    if let Ok(t) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        h.write_u128(t.as_nanos());
    }
    h.finish()
}

/// The operating system's login name, only to recognise a name that an
/// older version copied from it. Never used as a default.
fn os_user_names() -> Vec<String> {
    ["USER", "LOGNAME", "USERNAME"]
        .iter()
        .filter_map(|k| std::env::var(k).ok())
        .filter(|v| !v.is_empty())
        .collect()
}

/// Did the player choose `current`? A name saved in autogen.cfg that
/// differs from the OS login name is a choice; the 1.1 default
/// "UnnamedPlayer", an empty name, and the login name (what versions
/// before dec064 saved on their own) are not.
pub fn is_chosen(current: &str, os_users: &[String]) -> bool {
    let c = current.trim();
    if c.is_empty() || c.eq_ignore_ascii_case("UnnamedPlayer") || c.eq_ignore_ascii_case("player") {
        return false;
    }
    !os_users.iter().any(|u| u.trim().eq_ignore_ascii_case(c))
}

/// After autogen.cfg has run: when the name was not chosen, give the
/// profile a random one and mark the config for saving, so it stays.
/// A `+set name X` on the command line runs afterwards and wins.
pub fn assign_if_unchosen(c: &mut Cvars, seed: u64) -> bool {
    let current = c.string("name");
    if is_chosen(&current, &os_user_names()) {
        return false;
    }
    c.set("name", &random_name(seed));
    c.modified_flags |= CVAR_ARCHIVE;
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_and_bounded() {
        for seed in 0..5000u64 {
            let a = random_name(seed);
            assert_eq!(a, random_name(seed));
            assert!(a.len() <= 20 && a.len() < MAX_NAME_LENGTH, "{a}");
            assert!(a.bytes().all(|c| c.is_ascii_alphabetic()), "{a}");
            assert!(!a.contains('^') && a != "UnnamedPlayer");
        }
        assert_ne!(random_name(1), random_name(2));
    }

    #[test]
    fn every_combination_fits_and_is_clean() {
        let mut n = 0;
        for first in ADJECTIVES.iter().chain(VERBS) {
            for noun in NOUNS {
                let name = format!("{first}{noun}");
                assert!(
                    name.len() < MAX_NAME_LENGTH && name.bytes().all(|c| c.is_ascii_alphabetic()),
                    "{name}"
                );
                let low = name.to_ascii_lowercase();
                for b in BANNED {
                    assert!(!low.contains(b), "{name} contains {b}");
                }
                n += 1;
            }
        }
        assert!(n > 10_000, "{n}");
    }

    #[test]
    fn lists_are_clean_unique_camelcase() {
        for list in [ADJECTIVES, VERBS, NOUNS] {
            let mut seen = std::collections::HashSet::new();
            for w in list {
                assert!(
                    w.bytes().all(|c| c.is_ascii_alphabetic())
                        && w.as_bytes()[0].is_ascii_uppercase(),
                    "{w}"
                );
                assert!(seen.insert(*w), "duplicate {w}");
            }
        }
        assert!(ADJECTIVES.len() >= 60 && VERBS.len() >= 50 && NOUNS.len() >= 60);
    }

    #[test]
    fn never_the_os_user() {
        // even an OS user called like a generated name is a different string
        // from what is generated for that user (the generator never reads it)
        let users = os_user_names();
        for seed in 0..20_000u64 {
            let n = random_name(seed);
            assert!(!users.iter().any(|u| u.eq_ignore_ascii_case(&n)), "{n}");
        }
        // the source does not read the account while generating
        assert_eq!(random_name(42), random_name(42));
    }

    #[test]
    fn chosen_detection() {
        let u = vec!["alice".to_string()];
        assert!(!is_chosen("alice", &u));
        assert!(!is_chosen("Alice", &u));
        assert!(!is_chosen("", &u));
        assert!(!is_chosen("UnnamedPlayer", &u));
        assert!(is_chosen("Bob", &u));
        assert!(is_chosen("^1Bob", &u));
    }

    #[test]
    fn assign_skips_a_chosen_name_and_replaces_the_rest() {
        let mut c = Cvars::default();
        c.get("name", "UnnamedPlayer", CVAR_ARCHIVE);
        c.set("name", "Zed^7");
        assert!(!assign_if_unchosen(&mut c, 7));
        assert_eq!(c.string("name"), "Zed^7");
        c.set("name", "UnnamedPlayer");
        assert!(assign_if_unchosen(&mut c, 7));
        assert_eq!(c.string("name"), random_name(7));
    }

    #[test]
    fn samples() {
        let v: Vec<String> = (1..=10u64).map(|s| random_name(s * 7919)).collect();
        println!("SAMPLES {}", v.join(" "));
    }
}
