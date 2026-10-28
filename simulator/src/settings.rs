//! Command-line options as start menu choices.
//!
//! Every option here is an environment variable, so a run started from the
//! shell and a run set up in the menu behave the same. The environment sets the
//! menu's starting values. Match options apply when Start or Restart builds the
//! field. Startup options (vehicle, network, detectors) are read once when the
//! simulator starts, so changing one relaunches Zorvane with the new values.
//! Free-text options (router addresses, passwords, host key) stay command line.
use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

/// Menu choices that override the environment in this process.
fn overlay() -> &'static RwLock<HashMap<&'static str, String>> {
    static OVERLAY: OnceLock<RwLock<HashMap<&'static str, String>>> = OnceLock::new();
    OVERLAY.get_or_init(Default::default)
}

/// An option's value: the menu's choice, else the environment.
pub fn env(key: &str) -> Option<String> {
    if let Some(value) = overlay().read().unwrap().get(key) {
        return Some(value.clone());
    }
    std::env::var(key).ok()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Applies {
    /// When the field is built (Start, Restart).
    Match,
    /// Once at launch; a change relaunches Zorvane.
    Startup,
}

pub struct MenuOption {
    pub key: &'static str,
    pub label: &'static str,
    pub applies: Applies,
    /// Value and what the menu shows. The first is the default when unset.
    pub choices: &'static [(&'static str, &'static str)],
}

const ON_OFF: &[(&str, &str); 2] = &[("1", "On"), ("0", "Off")];

pub const OPTIONS: &[MenuOption] = &[
    MenuOption {
        key: "TERRA_NEXT_CAMERA",
        label: "Camera",
        applies: Applies::Match,
        choices: &[
            ("follow", "Follow rover 0"),
            ("free", "Free camera"),
            ("overview", "Field overview"),
            ("spectator", "Spectator (no driving)"),
        ],
    },
    MenuOption {
        key: "TERRA_NEXT_LIGHTING",
        label: "Lighting",
        applies: Applies::Match,
        choices: &[
            ("noon", "Noon"),
            ("overcast", "Overcast"),
            ("evening", "Evening"),
        ],
    },
    MenuOption {
        key: "TERRA_NEXT_BALLS",
        label: "Extra balls",
        applies: Applies::Match,
        choices: &[
            ("0", "None"),
            ("50", "50"),
            ("100", "100"),
            ("200", "200"),
            ("400", "400"),
        ],
    },
    MenuOption {
        key: "TERRA_NEXT_SEED",
        label: "Bucket layout",
        applies: Applies::Match,
        choices: &[
            ("42", "Seed 42"),
            ("1", "Seed 1"),
            ("2", "Seed 2"),
            ("3", "Seed 3"),
            ("4", "Seed 4"),
            ("5", "Seed 5"),
        ],
    },
    MenuOption {
        key: "ZORVANE_CUTSCENE",
        label: "Intro cutscene",
        applies: Applies::Match,
        choices: ON_OFF,
    },
    MenuOption {
        key: "ZORVANE_VEHICLE",
        label: "Rover",
        applies: Applies::Startup,
        choices: &[("crab-trap-bot", "CrabTrapBot"), ("terra-ground", "Terra")],
    },
    MenuOption {
        key: "TERRA_ACTUATION_MODE",
        label: "Motor control",
        applies: Applies::Startup,
        choices: &[
            ("velocity_feedback", "Velocity loop"),
            ("direct_effort", "Direct effort"),
        ],
    },
    MenuOption {
        key: "TERRA_ZENOH",
        label: "Dashboard link",
        applies: Applies::Startup,
        choices: &[("1", "On (ARGOS, MCP)"), ("0", "Off (keyboard)")],
    },
    MenuOption {
        key: "TERRA_ZENOH_LISTEN",
        label: "Who can connect",
        applies: Applies::Startup,
        choices: &[
            ("tcp/127.0.0.1:7447", "This computer"),
            ("tcp/0.0.0.0:7447", "Other computers too"),
        ],
    },
    MenuOption {
        key: "ZORVANE_MATCH",
        label: "Multiplayer",
        applies: Applies::Startup,
        choices: &[("open", "Open"), ("seated", "Seated")],
    },
    MenuOption {
        key: "ZORVANE_JOIN_CODE",
        label: "Join code",
        applies: Applies::Startup,
        // The second choice is filled with a fresh code, see `Choices::new`.
        choices: &[("", "None"), (GENERATED, "New code")],
    },
    MenuOption {
        key: "ZORVANE_SEAT_TIMEOUT",
        label: "Seat timeout",
        applies: Applies::Startup,
        choices: &[("15", "15 s"), ("30", "30 s"), ("60", "60 s")],
    },
    MenuOption {
        key: "ZORVANE_FPV",
        label: "FPV video",
        applies: Applies::Startup,
        choices: ON_OFF,
    },
    MenuOption {
        key: "ZORVANE_OBJECTS",
        label: "Bucket and ball detector",
        applies: Applies::Startup,
        choices: ON_OFF,
    },
];

/// Placeholder value for a join code made up when the menu opens.
const GENERATED: &str = "<generated>";

fn join_code() -> String {
    // No 0/O or 1/I, so the code reads aloud cleanly.
    const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    (0..6)
        .map(|_| ALPHABET[fastrand::usize(..ALPHABET.len())] as char)
        .collect()
}

/// One option's choices, with any value from the environment the list lacks.
#[derive(Clone, Debug)]
pub struct Choices {
    pub key: &'static str,
    pub label: &'static str,
    pub applies: Applies,
    pub values: Vec<(String, String)>,
    pub selected: usize,
    /// The choice this process started with.
    pub launched: usize,
}

impl Choices {
    fn new(option: &MenuOption, from_env: Option<String>) -> Self {
        let mut values: Vec<(String, String)> = option
            .choices
            .iter()
            .map(|(value, shown)| {
                if *value == GENERATED {
                    let code = join_code();
                    (code.clone(), code)
                } else {
                    ((*value).to_owned(), (*shown).to_owned())
                }
            })
            .collect();
        // ZORVANE_SPECTATOR=1 is the camera's spectator choice.
        let from_env = if option.key == "TERRA_NEXT_CAMERA"
            && matches!(
                std::env::var("ZORVANE_SPECTATOR").as_deref(),
                Ok("1") | Ok("true")
            ) {
            Some("spectator".to_owned())
        } else {
            from_env
        };
        let selected = match from_env.map(|value| value.trim().to_owned()) {
            None => 0,
            Some(value) => match values.iter().position(|(known, _)| *known == value) {
                Some(index) => index,
                None => {
                    values.push((value.clone(), format!("{value} (set at launch)")));
                    values.len() - 1
                }
            },
        };
        Self {
            key: option.key,
            label: option.label,
            applies: option.applies,
            values,
            selected,
            launched: selected,
        }
    }

    pub fn value(&self) -> &str {
        &self.values[self.selected].0
    }

    pub fn shown(&self) -> &str {
        &self.values[self.selected].1
    }

    pub fn cycle(&mut self) {
        self.selected = (self.selected + 1) % self.values.len();
    }

    pub fn changed(&self) -> bool {
        self.selected != self.launched
    }
}

/// Every menu option with its current choice.
#[derive(bevy::prelude::Resource, Clone, Debug)]
pub struct MenuSettings {
    pub options: Vec<Choices>,
}

impl MenuSettings {
    pub fn from_env() -> Self {
        Self {
            options: OPTIONS
                .iter()
                .map(|option| Choices::new(option, std::env::var(option.key).ok()))
                .collect(),
        }
    }

    pub fn get(&self, key: &str) -> Option<&Choices> {
        self.options.iter().find(|option| option.key == key)
    }

    /// A startup option differs from this run's, so Start must relaunch.
    pub fn needs_relaunch(&self) -> bool {
        self.options
            .iter()
            .any(|option| option.applies == Applies::Startup && option.changed())
    }

    /// Makes the match choices visible to `env` for the next field build.
    pub fn apply_match(&self) {
        let mut overlay = overlay().write().unwrap();
        for option in self.options.iter().filter(|o| o.applies == Applies::Match) {
            overlay.insert(option.key, option.value().to_owned());
        }
    }

    /// Every choice as environment variables, for a relaunch.
    pub fn env_vars(&self) -> Vec<(&'static str, String)> {
        self.options
            .iter()
            .map(|option| (option.key, option.value().to_owned()))
            .collect()
    }
}

/// Set on a relaunched Zorvane: skip the start menu and play at once.
pub const AUTOSTART: &str = "ZORVANE_AUTOSTART";

/// Starts a new Zorvane with these choices. The caller then exits this one.
pub fn relaunch(settings: &MenuSettings, rover_count: usize) -> std::io::Result<()> {
    let exe = std::env::current_exe()?;
    let mut command = std::process::Command::new(exe);
    command
        .args(std::env::args_os().skip(1))
        .envs(settings.env_vars())
        .env("TERRA_ROVER_COUNT", rover_count.to_string())
        .env(AUTOSTART, "1")
        .env_remove("ZORVANE_SPECTATOR");
    command.spawn().map(|_| ())
}

/// A relaunched Zorvane waits for the old one to free its Zenoh port.
pub fn wait_for_listen_port() {
    if std::env::var(AUTOSTART).as_deref() != Ok("1") {
        return;
    }
    let listen =
        std::env::var("TERRA_ZENOH_LISTEN").unwrap_or_else(|_| "tcp/127.0.0.1:7447".into());
    let Some(address) = listen.strip_prefix("tcp/") else {
        return;
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while std::net::TcpListener::bind(address).is_err() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_environment_values_are_kept_as_a_choice() {
        let option = &OPTIONS[2];
        let choices = Choices::new(option, Some("75".into()));
        assert_eq!(choices.value(), "75");
        assert!(!choices.changed());
        let choices = Choices::new(option, None);
        assert_eq!(choices.value(), "0");
    }

    #[test]
    fn startup_change_needs_relaunch_and_match_change_does_not() {
        let mut settings = MenuSettings {
            options: OPTIONS.iter().map(|o| Choices::new(o, None)).collect(),
        };
        assert!(!settings.needs_relaunch());
        let lighting = settings
            .options
            .iter()
            .position(|o| o.key == "TERRA_NEXT_LIGHTING")
            .unwrap();
        settings.options[lighting].cycle();
        assert!(!settings.needs_relaunch());
        settings.apply_match();
        assert_eq!(env("TERRA_NEXT_LIGHTING").as_deref(), Some("overcast"));
        let vehicle = settings
            .options
            .iter()
            .position(|o| o.key == "ZORVANE_VEHICLE")
            .unwrap();
        settings.options[vehicle].cycle();
        assert!(settings.needs_relaunch());
        settings.options[vehicle].cycle();
        assert!(!settings.needs_relaunch());
    }

    #[test]
    fn generated_join_code_is_six_readable_characters() {
        let option = OPTIONS
            .iter()
            .find(|o| o.key == "ZORVANE_JOIN_CODE")
            .unwrap();
        let mut choices = Choices::new(option, None);
        assert_eq!(choices.value(), "");
        choices.cycle();
        assert_eq!(choices.value().len(), 6);
        assert!(!choices.value().contains(['0', 'O', '1', 'I']));
    }
}
