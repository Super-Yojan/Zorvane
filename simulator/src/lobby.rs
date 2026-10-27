//! Multiplayer seats: who may command which rover, and the match phase.
//!
//! A player joins with a Zenoh query on `<prefix>/match/join` and gets a seat:
//! an id, a secret and its rovers. Every later command from that player carries
//! a Zenoh attachment `{"seat":"s1","seq":7,"mac":"<hex>"}`, where `mac` is
//! HMAC-SHA256, keyed by the secret, of `seat \n seq \n key \n payload`. The
//! secret only travels in the query reply, so a player who sees another
//! player's commands on the bus can neither forge nor replay them. The host
//! signs the same way as seat `host` with `ZORVANE_HOST_KEY`.
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

pub const HOST_SEAT: &str = "host";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// Unclaimed rovers take unsigned commands, as before multiplayer.
    Open,
    /// Every rover command must be signed by the rover's seat or the host.
    Seated,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Phase {
    Lobby,
    Running,
    Paused,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Team {
    Red,
    Blue,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum PlayerKind {
    #[default]
    Human,
    Agent,
}

/// Host actions on `<prefix>/match/control`, signed by the host.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase", tag = "action")]
pub enum MatchAction {
    Start,
    Pause,
    Resume,
    Reset,
    /// Frees every seat. Players must join again.
    Kick,
}

#[derive(Clone, Debug)]
pub struct LobbyConfig {
    pub mode: Mode,
    pub join_code: Option<String>,
    pub host_key: String,
    pub seat_timeout: Duration,
}

impl Default for LobbyConfig {
    fn default() -> Self {
        Self {
            mode: Mode::Open,
            join_code: None,
            host_key: random_hex(16),
            seat_timeout: Duration::from_secs(15),
        }
    }
}

impl LobbyConfig {
    /// `ZORVANE_MATCH` (`open` or `seated`), `ZORVANE_JOIN_CODE`,
    /// `ZORVANE_HOST_KEY` (random when unset) and `ZORVANE_SEAT_TIMEOUT` seconds.
    pub fn from_env() -> Result<Self, String> {
        let mut config = Self::default();
        if let Ok(mode) = std::env::var("ZORVANE_MATCH") {
            config.mode = match mode.trim() {
                "" | "open" => Mode::Open,
                "seated" => Mode::Seated,
                other => return Err(format!("ZORVANE_MATCH must be open or seated, not {other}")),
            };
        }
        config.join_code = std::env::var("ZORVANE_JOIN_CODE")
            .ok()
            .map(|code| code.trim().to_owned())
            .filter(|code| !code.is_empty());
        if let Ok(key) = std::env::var("ZORVANE_HOST_KEY") {
            let key = key.trim();
            if key.len() < 8 {
                return Err("ZORVANE_HOST_KEY must be at least 8 characters".into());
            }
            config.host_key = key.to_owned();
        }
        if let Ok(seconds) = std::env::var("ZORVANE_SEAT_TIMEOUT") {
            let seconds: f64 = seconds
                .trim()
                .parse()
                .map_err(|_| "ZORVANE_SEAT_TIMEOUT must be a number of seconds".to_owned())?;
            if !(1.0..=3600.0).contains(&seconds) {
                return Err("ZORVANE_SEAT_TIMEOUT must be 1 to 3600 seconds".into());
            }
            config.seat_timeout = Duration::from_secs_f64(seconds);
        }
        Ok(config)
    }
}

/// Body of a `match/join` query.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JoinRequest {
    pub name: String,
    #[serde(default)]
    pub team: Option<Team>,
    /// A count, `"all"` for every free rover of the team, or a list of ids.
    #[serde(default)]
    pub rovers: Option<serde_json::Value>,
    #[serde(default)]
    pub kind: PlayerKind,
    #[serde(default)]
    pub code: Option<String>,
}

#[derive(Debug)]
struct Seat {
    name: String,
    kind: PlayerKind,
    team: Team,
    rovers: Vec<u64>,
    secret: String,
    last_seq: u64,
    last_seen: Instant,
}

pub struct Lobby {
    pub config: LobbyConfig,
    pub phase: Phase,
    seats: BTreeMap<String, Seat>,
    host_seq: u64,
    next_seat: u64,
    running_since: Option<Instant>,
    elapsed_before: Duration,
}

impl Default for Lobby {
    fn default() -> Self {
        Self::new(LobbyConfig::default())
    }
}

/// Why a command was refused. Logged once per kind, never sent back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    BadSignature,
    NotYourRover,
    Unseated,
    NotRunning,
    HostOnly,
}

impl Lobby {
    pub fn new(config: LobbyConfig) -> Self {
        let phase = match config.mode {
            Mode::Open => Phase::Running,
            Mode::Seated => Phase::Lobby,
        };
        Self {
            config,
            phase,
            seats: BTreeMap::new(),
            host_seq: 0,
            next_seat: 1,
            running_since: (phase == Phase::Running).then(Instant::now),
            elapsed_before: Duration::ZERO,
        }
    }

    pub fn owner(&self, rover: u64) -> Option<&str> {
        self.seats
            .iter()
            .find(|(_, seat)| seat.rovers.contains(&rover))
            .map(|(id, _)| id.as_str())
    }

    /// Seats a player. `fleet` is every spawned rover with its alliance.
    pub fn join(
        &mut self,
        request: &[u8],
        fleet: &[(u64, Team)],
        now: Instant,
    ) -> Result<serde_json::Value, String> {
        let request: JoinRequest = serde_json::from_slice(request)
            .map_err(|error| format!("bad join request: {error}"))?;
        if let Some(code) = &self.config.join_code
            && request.code.as_deref() != Some(code.as_str())
        {
            return Err("wrong join code".into());
        }
        let name = request.name.trim();
        if name.is_empty()
            || name.len() > 32
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b" _.-".contains(&b))
        {
            return Err("name must be 1 to 32 letters, digits, spaces, _ . or -".into());
        }
        let free: Vec<(u64, Team)> = fleet
            .iter()
            .copied()
            .filter(|(id, team)| {
                self.owner(*id).is_none() && request.team.is_none_or(|wanted| wanted == *team)
            })
            .collect();
        let rovers: Vec<u64> = match &request.rovers {
            None => free.iter().take(1).map(|(id, _)| *id).collect(),
            Some(serde_json::Value::String(all)) if all == "all" => {
                let team = request.team.or(free.first().map(|(_, team)| *team));
                free.iter()
                    .filter(|(_, t)| Some(*t) == team)
                    .map(|(id, _)| *id)
                    .collect()
            }
            Some(serde_json::Value::Number(count)) => {
                let count = count
                    .as_u64()
                    .filter(|count| (1..=32).contains(count))
                    .ok_or("rovers must be 1 to 32")? as usize;
                let team = request.team.or(free.first().map(|(_, team)| *team));
                let picked: Vec<u64> = free
                    .iter()
                    .filter(|(_, t)| Some(*t) == team)
                    .take(count)
                    .map(|(id, _)| *id)
                    .collect();
                if picked.len() < count {
                    return Err(format!(
                        "only {} free rover(s) on that team; the host can raise TERRA_ROVER_COUNT",
                        picked.len()
                    ));
                }
                picked
            }
            Some(serde_json::Value::Array(ids)) => {
                let mut picked = Vec::new();
                for id in ids {
                    let id = id.as_u64().ok_or("rover ids must be integers")?;
                    if !free.iter().any(|(free_id, _)| *free_id == id) {
                        return Err(format!(
                            "rover {id} is taken, missing, or on the other team"
                        ));
                    }
                    if !picked.contains(&id) {
                        picked.push(id);
                    }
                }
                picked
            }
            Some(_) => return Err("rovers must be a count, \"all\", or a list of ids".into()),
        };
        let Some(team) = rovers
            .first()
            .and_then(|id| fleet.iter().find(|(rover, _)| rover == id))
            .map(|(_, team)| *team)
        else {
            return Err("no free rover; the host can raise TERRA_ROVER_COUNT".into());
        };
        if rovers
            .iter()
            .any(|id| fleet.iter().any(|(rover, t)| rover == id && *t != team))
        {
            return Err("one seat drives rovers of one team".into());
        }
        let seat = format!("s{}", self.next_seat);
        self.next_seat += 1;
        let secret = random_hex(32);
        self.seats.insert(
            seat.clone(),
            Seat {
                name: name.to_owned(),
                kind: request.kind,
                team,
                rovers: rovers.clone(),
                secret: secret.clone(),
                last_seq: 0,
                last_seen: now,
            },
        );
        Ok(serde_json::json!({
            "ok": true,
            "seat": seat,
            "secret": secret,
            "team": team,
            "rovers": rovers,
            "mode": self.config.mode,
            "phase": self.phase,
            "seat_timeout": self.config.seat_timeout.as_secs_f64(),
        }))
    }

    /// Checks an attachment. `Ok(None)` is an unsigned sample, `Ok(Some(seat))`
    /// a valid signature with a fresh sequence number (which it records).
    pub fn verify(
        &mut self,
        key: &str,
        payload: &[u8],
        attachment: Option<&[u8]>,
        now: Instant,
    ) -> Result<Option<String>, Refusal> {
        let Some(attachment) = attachment else {
            return Ok(None);
        };
        #[derive(Deserialize)]
        struct Signature {
            seat: String,
            seq: u64,
            mac: String,
        }
        let signature: Signature =
            serde_json::from_slice(attachment).map_err(|_| Refusal::BadSignature)?;
        let mac = decode_hex(&signature.mac).ok_or(Refusal::BadSignature)?;
        let (secret, last_seq) = if signature.seat == HOST_SEAT {
            (self.config.host_key.as_str(), self.host_seq)
        } else {
            let seat = self
                .seats
                .get(&signature.seat)
                .ok_or(Refusal::BadSignature)?;
            (seat.secret.as_str(), seat.last_seq)
        };
        if signature.seq <= last_seq {
            return Err(Refusal::BadSignature);
        }
        mac_for(secret, &signature.seat, signature.seq, key, payload)
            .verify_slice(&mac)
            .map_err(|_| Refusal::BadSignature)?;
        if signature.seat == HOST_SEAT {
            self.host_seq = signature.seq;
        } else if let Some(seat) = self.seats.get_mut(&signature.seat) {
            seat.last_seq = signature.seq;
            seat.last_seen = now;
        }
        Ok(Some(signature.seat))
    }

    /// May `signer` send this command to `rover`? `stop` is an emergency stop,
    /// which the owner may send in any phase.
    pub fn authorize(&self, rover: u64, signer: Option<&str>, stop: bool) -> Result<(), Refusal> {
        if signer == Some(HOST_SEAT) {
            return Ok(());
        }
        match (self.owner(rover), signer) {
            (Some(owner), Some(signer)) if owner == signer => {}
            (Some(_), _) => return Err(Refusal::NotYourRover),
            (None, None) if self.config.mode == Mode::Open => return Ok(()),
            (None, _) => return Err(Refusal::Unseated),
        }
        if self.config.mode == Mode::Seated && self.phase != Phase::Running && !stop {
            return Err(Refusal::NotRunning);
        }
        Ok(())
    }

    /// Fleet size, open to everyone only in open mode.
    pub fn authorize_host(&self, signer: Option<&str>) -> Result<(), Refusal> {
        if signer == Some(HOST_SEAT) || (self.config.mode == Mode::Open && signer.is_none()) {
            Ok(())
        } else {
            Err(Refusal::HostOnly)
        }
    }

    /// Frees a seat. Returns its rovers.
    pub fn leave(&mut self, seat: &str) -> Vec<u64> {
        self.seats
            .remove(seat)
            .map(|seat| seat.rovers)
            .unwrap_or_default()
    }

    /// Frees seats with no signed sample for the timeout. Returns their rovers.
    pub fn expire(&mut self, now: Instant) -> Vec<u64> {
        let timeout = self.config.seat_timeout;
        let stale: Vec<String> = self
            .seats
            .iter()
            .filter(|(_, seat)| now.saturating_duration_since(seat.last_seen) > timeout)
            .map(|(id, _)| id.clone())
            .collect();
        stale.iter().flat_map(|seat| self.leave(seat)).collect()
    }

    /// Applies a host action. Returns the rovers to stop.
    pub fn apply(&mut self, action: MatchAction, now: Instant) -> Vec<u64> {
        match action {
            MatchAction::Start => {
                self.elapsed_before = Duration::ZERO;
                self.running_since = Some(now);
                self.phase = Phase::Running;
            }
            MatchAction::Pause if self.phase == Phase::Running => {
                self.elapsed_before += self
                    .running_since
                    .take()
                    .map_or(Duration::ZERO, |t| now - t);
                self.phase = Phase::Paused;
            }
            MatchAction::Resume if self.phase == Phase::Paused => {
                self.running_since = Some(now);
                self.phase = Phase::Running;
            }
            MatchAction::Reset => {
                self.elapsed_before = Duration::ZERO;
                self.running_since = None;
                self.phase = match self.config.mode {
                    Mode::Open => Phase::Running,
                    Mode::Seated => Phase::Lobby,
                };
                if self.phase == Phase::Running {
                    self.running_since = Some(now);
                }
            }
            MatchAction::Kick => {
                return std::mem::take(&mut self.seats)
                    .into_values()
                    .flat_map(|seat| seat.rovers)
                    .collect();
            }
            MatchAction::Pause | MatchAction::Resume => {}
        }
        Vec::new()
    }

    pub fn elapsed(&self, now: Instant) -> Duration {
        self.elapsed_before + self.running_since.map_or(Duration::ZERO, |t| now - t)
    }

    /// `<prefix>/match/state`. Never carries secrets.
    pub fn state_json(&self, fleet: &[(u64, Team)], now: Instant) -> serde_json::Value {
        let seats: Vec<_> = self
            .seats
            .iter()
            .map(|(id, seat)| {
                serde_json::json!({
                    "seat": id,
                    "name": seat.name,
                    "kind": seat.kind,
                    "team": seat.team,
                    "rovers": seat.rovers,
                    "idle": now.saturating_duration_since(seat.last_seen).as_secs_f64(),
                })
            })
            .collect();
        let free: Vec<_> = fleet
            .iter()
            .filter(|(id, _)| self.owner(*id).is_none())
            .map(|(id, team)| serde_json::json!({"rover": id, "team": team}))
            .collect();
        serde_json::json!({
            "schema_version": 1,
            "mode": self.config.mode,
            "phase": self.phase,
            "elapsed": self.elapsed(now).as_secs_f64(),
            "join_code_required": self.config.join_code.is_some(),
            "seats": seats,
            "free": free,
        })
    }
}

fn mac_for(secret: &str, seat: &str, seq: u64, key: &str, payload: &[u8]) -> Hmac<Sha256> {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC takes any key");
    mac.update(format!("{seat}\n{seq}\n{key}\n").as_bytes());
    mac.update(payload);
    mac
}

/// The attachment a client sends. Used by tests; clients do the same in Python.
#[cfg(test)]
pub fn sign(secret: &str, seat: &str, seq: u64, key: &str, payload: &[u8]) -> Vec<u8> {
    let mac = mac_for(secret, seat, seq, key, payload)
        .finalize()
        .into_bytes();
    let mac: String = mac.iter().map(|b| format!("{b:02x}")).collect();
    serde_json::to_vec(&serde_json::json!({"seat": seat, "seq": seq, "mac": mac})).unwrap()
}

fn random_hex(bytes: usize) -> String {
    let mut buffer = vec![0u8; bytes];
    getrandom::fill(&mut buffer).expect("operating system random source");
    buffer.iter().map(|b| format!("{b:02x}")).collect()
}

fn decode_hex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) || text.len() > 128 {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "terra/rover/0/teleop";

    fn seated() -> Lobby {
        Lobby::new(LobbyConfig {
            mode: Mode::Seated,
            join_code: Some("field3".into()),
            host_key: "host-key-123".into(),
            seat_timeout: Duration::from_secs(10),
        })
    }

    fn fleet() -> Vec<(u64, Team)> {
        vec![
            (0, Team::Red),
            (1, Team::Blue),
            (2, Team::Red),
            (3, Team::Blue),
        ]
    }

    fn join(lobby: &mut Lobby, body: serde_json::Value) -> Result<serde_json::Value, String> {
        lobby.join(
            &serde_json::to_vec(&body).unwrap(),
            &fleet(),
            Instant::now(),
        )
    }

    #[test]
    fn signature_matches_the_python_player() {
        // tools/test_zorvane_player.py checks the same vector.
        let attachment = sign("secret", "s1", 7, KEY, b"{}");
        let value: serde_json::Value = serde_json::from_slice(&attachment).unwrap();
        assert_eq!(
            value["mac"],
            "62ef79d1ee32fcbc12371df6cd1a1f158a63f1ba02d7455df6c9acf719f8ba85"
        );
    }

    #[test]
    fn join_assigns_free_rovers_on_the_asked_team() {
        let mut lobby = seated();
        let red = join(
            &mut lobby,
            serde_json::json!({"name":"ana","team":"red","code":"field3"}),
        )
        .unwrap();
        assert_eq!(red["rovers"], serde_json::json!([0]));
        let team = join(
            &mut lobby,
            serde_json::json!({"name":"bot","team":"red","rovers":"all","kind":"agent","code":"field3"}),
        )
        .unwrap();
        assert_eq!(team["rovers"], serde_json::json!([2]));
        assert!(
            join(
                &mut lobby,
                serde_json::json!({"name":"late","team":"red","code":"field3"})
            )
            .is_err()
        );
        let blue = join(
            &mut lobby,
            serde_json::json!({"name":"ben","rovers":[3],"code":"field3"}),
        )
        .unwrap();
        assert_eq!(blue["team"], "blue");
        assert!(
            join(
                &mut lobby,
                serde_json::json!({"name":"x","rovers":[3],"code":"field3"})
            )
            .is_err()
        );
        assert!(
            join(
                &mut lobby,
                serde_json::json!({"name":"x","rovers":[1,2],"code":"field3"})
            )
            .is_err()
        );
        assert!(
            join(&mut lobby, serde_json::json!({"name":"x"})).is_err(),
            "join code"
        );
        let state = lobby.state_json(&fleet(), Instant::now()).to_string();
        assert!(
            !state.contains(red["secret"].as_str().unwrap()),
            "state leaks a secret"
        );
    }

    #[test]
    fn only_the_seat_drives_its_rover_and_replays_fail() {
        let mut lobby = seated();
        let now = Instant::now();
        let red = join(
            &mut lobby,
            serde_json::json!({"name":"ana","team":"red","code":"field3"}),
        )
        .unwrap();
        let (seat, secret) = (
            red["seat"].as_str().unwrap(),
            red["secret"].as_str().unwrap(),
        );
        lobby.apply(MatchAction::Start, now);
        let body = br#"{"linear":0.5,"angular":0}"#;
        let signed = sign(secret, seat, 1, KEY, body);
        let signer = lobby.verify(KEY, body, Some(&signed), now).unwrap();
        assert_eq!(signer.as_deref(), Some(seat));
        assert_eq!(lobby.authorize(0, signer.as_deref(), false), Ok(()));
        assert_eq!(
            lobby.verify(KEY, body, Some(&signed), now),
            Err(Refusal::BadSignature),
            "replay"
        );
        let other_key = "terra/rover/1/teleop";
        let moved = sign(secret, seat, 2, KEY, body);
        assert_eq!(
            lobby.verify(other_key, body, Some(&moved), now),
            Err(Refusal::BadSignature)
        );
        let forged = sign("wrong", seat, 3, KEY, body);
        assert_eq!(
            lobby.verify(KEY, body, Some(&forged), now),
            Err(Refusal::BadSignature)
        );
        assert_eq!(
            lobby.authorize(1, Some(seat), false),
            Err(Refusal::Unseated)
        );
        assert_eq!(lobby.authorize(0, None, false), Err(Refusal::NotYourRover));
        assert_eq!(lobby.authorize(1, None, false), Err(Refusal::Unseated));
        let host = sign("host-key-123", HOST_SEAT, 1, KEY, body);
        let host = lobby.verify(KEY, body, Some(&host), now).unwrap();
        assert_eq!(lobby.authorize(0, host.as_deref(), false), Ok(()));
        assert_eq!(lobby.authorize_host(Some(seat)), Err(Refusal::HostOnly));
    }

    #[test]
    fn phases_gate_driving_but_not_stops() {
        let mut lobby = seated();
        let now = Instant::now();
        let red = join(
            &mut lobby,
            serde_json::json!({"name":"ana","code":"field3"}),
        )
        .unwrap();
        let seat = red["seat"].as_str().unwrap();
        assert_eq!(
            lobby.authorize(0, Some(seat), false),
            Err(Refusal::NotRunning)
        );
        assert_eq!(lobby.authorize(0, Some(seat), true), Ok(()));
        lobby.apply(MatchAction::Start, now);
        assert_eq!(lobby.authorize(0, Some(seat), false), Ok(()));
        lobby.apply(MatchAction::Pause, now + Duration::from_secs(5));
        assert_eq!(
            lobby.authorize(0, Some(seat), false),
            Err(Refusal::NotRunning)
        );
        assert_eq!(
            lobby.elapsed(now + Duration::from_secs(9)),
            Duration::from_secs(5)
        );
        lobby.apply(MatchAction::Resume, now + Duration::from_secs(9));
        assert_eq!(
            lobby.elapsed(now + Duration::from_secs(10)),
            Duration::from_secs(6)
        );
        lobby.apply(MatchAction::Reset, now + Duration::from_secs(10));
        assert_eq!(lobby.phase, Phase::Lobby);
        assert_eq!(lobby.owner(0), Some(seat), "a reset keeps seats");
        assert_eq!(lobby.apply(MatchAction::Kick, now), vec![0]);
        assert_eq!(lobby.owner(0), None);
    }

    #[test]
    fn open_mode_keeps_unclaimed_rovers_open_and_seats_expire() {
        let mut lobby = Lobby::new(LobbyConfig {
            seat_timeout: Duration::from_secs(10),
            ..LobbyConfig::default()
        });
        let now = Instant::now();
        assert_eq!(lobby.phase, Phase::Running);
        assert_eq!(lobby.authorize(1, None, false), Ok(()));
        assert_eq!(lobby.authorize_host(None), Ok(()));
        join(&mut lobby, serde_json::json!({"name":"ana","team":"blue"})).unwrap();
        assert_eq!(lobby.authorize(1, None, false), Err(Refusal::NotYourRover));
        assert!(lobby.expire(now + Duration::from_secs(5)).is_empty());
        assert_eq!(lobby.expire(now + Duration::from_secs(11)), vec![1]);
        assert_eq!(lobby.authorize(1, None, false), Ok(()));
    }
}
