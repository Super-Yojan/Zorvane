//! Online play from the start menu: sign in to a lobby server, list its rooms,
//! and join one. Hosting a room is `Bridge::host_room` in bridge.rs.
//!
//! Joining runs the player relay in-process (the Rust twin of
//! `tools/zorvane_player.py`): it takes a seat in the room, opens
//! `tcp/127.0.0.1:7447` under `terra/rover` for ARGOS, teleop or the MCP
//! server, signs every command for the seat's rovers, and pulls only the state
//! of the player's own team across the network.
use crate::lobby::{ROOMS, Team};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use zenoh::Wait;

/// The lab's ARGOS router on GCP, which doubles as the Zorvane lobby.
pub const DEFAULT_SERVER: &str = "tls/argos.34.135.204.210.sslip.io:7448";
pub const DEFAULT_USER: &str = "zorvane";
/// Where ARGOS and the other clients find a joined room on this machine.
pub const LOCAL_ENDPOINT: &str = "tcp/127.0.0.1:7447";

/// Sign-in details, remembered between runs in the per-user config folder.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct OnlineSettings {
    pub server: String,
    pub user: String,
    pub password: String,
    pub name: String,
}

impl Default for OnlineSettings {
    fn default() -> Self {
        Self {
            server: DEFAULT_SERVER.into(),
            user: DEFAULT_USER.into(),
            password: String::new(),
            name: std::env::var("USER")
                .or_else(|_| std::env::var("USERNAME"))
                .ok()
                .filter(|name| valid_name(name))
                .unwrap_or_else(|| "player".into()),
        }
    }
}

impl OnlineSettings {
    /// Saved settings, then `ZORVANE_SERVER`, `ZORVANE_SERVER_USER` and
    /// `ZORVANE_SERVER_PASSWORD` on top.
    pub fn load() -> Self {
        let mut settings: Self = settings_path()
            .and_then(|path| std::fs::read(path).ok())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        let env = |key: &str| {
            std::env::var(key)
                .ok()
                .filter(|value| !value.trim().is_empty())
        };
        if let Some(server) = env("ZORVANE_SERVER") {
            settings.server = server;
        }
        if let Some(user) = env("ZORVANE_SERVER_USER") {
            settings.user = user;
        }
        if let Some(password) = env("ZORVANE_SERVER_PASSWORD") {
            settings.password = password;
        }
        settings
    }

    /// Writes the settings, readable only by this user where the OS allows.
    pub fn save(&self) {
        let Some(path) = settings_path() else { return };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if std::fs::write(&path, serde_json::to_vec_pretty(self).unwrap()).is_ok() {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
            }
        }
    }

    fn zenoh_config(&self) -> zenoh::Result<zenoh::Config> {
        let mut config = zenoh::Config::default();
        config.insert_json5("mode", "\"client\"")?;
        config.insert_json5(
            "connect/endpoints",
            &serde_json::to_string(&[self.server.trim()]).unwrap(),
        )?;
        config.insert_json5("scouting/multicast/enabled", "false")?;
        config.insert_json5("connect/exit_on_failure", "true")?;
        if !self.user.trim().is_empty() {
            config.insert_json5(
                "transport/auth/usrpwd/user",
                &serde_json::to_string(self.user.trim()).unwrap(),
            )?;
            config.insert_json5(
                "transport/auth/usrpwd/password",
                &serde_json::to_string(&self.password).unwrap(),
            )?;
        }
        Ok(config)
    }

    pub fn validate(&self) -> Result<(), String> {
        let server = self.server.trim();
        if !(server.starts_with("tcp/") || server.starts_with("tls/"))
            || server.parse::<zenoh::config::EndPoint>().is_err()
        {
            return Err("Server must look like tls/host:7448 or tcp/host:7447".into());
        }
        if !valid_name(&self.name) {
            return Err("Name: 1 to 32 letters, digits, spaces, _ . or -".into());
        }
        Ok(())
    }
}

pub fn valid_name(name: &str) -> bool {
    let name = name.trim();
    !name.is_empty()
        && name.len() <= 32
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b" _.-".contains(&b))
}

fn settings_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    if cfg!(target_os = "macos") {
        return home.map(|home| home.join("Library/Application Support/Zorvane/online.json"));
    }
    if cfg!(windows) {
        return std::env::var_os("APPDATA")
            .map(|dir| PathBuf::from(dir).join("Zorvane\\online.json"));
    }
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| home.map(|home| home.join(".config")))
        .map(|dir| dir.join("zorvane/online.json"))
}

/// One entry of the room list, as the host answers `zorvane/rooms/<code>`.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct RoomInfo {
    pub code: String,
    pub name: String,
    #[serde(default)]
    pub host: String,
    pub prefix: String,
    #[serde(default)]
    pub password: bool,
    #[serde(default)]
    pub phase: String,
    #[serde(default)]
    pub rovers: usize,
    #[serde(default)]
    pub free: FreeRovers,
    #[serde(default)]
    pub seats: Vec<SeatInfo>,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct FreeRovers {
    pub red: usize,
    pub blue: usize,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct SeatInfo {
    pub name: String,
    #[serde(default)]
    pub kind: String,
    pub team: Team,
}

impl RoomInfo {
    pub fn free(&self, team: Team) -> usize {
        match team {
            Team::Red => self.free.red,
            Team::Blue => self.free.blue,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum LobbyStatus {
    Connecting,
    Connected,
    Failed(String),
}

#[derive(Default)]
struct LobbyShared {
    status: Option<LobbyStatus>,
    rooms: Vec<RoomInfo>,
}

/// A signed-in lobby: lists the rooms every two seconds until dropped.
pub struct LobbyClient {
    shared: Arc<Mutex<LobbyShared>>,
    stop: Arc<AtomicBool>,
}

impl LobbyClient {
    pub fn connect(settings: OnlineSettings) -> Self {
        let shared = Arc::new(Mutex::new(LobbyShared {
            status: Some(LobbyStatus::Connecting),
            rooms: Vec::new(),
        }));
        let stop = Arc::new(AtomicBool::new(false));
        let (thread_shared, thread_stop) = (shared.clone(), stop.clone());
        std::thread::Builder::new()
            .name("zorvane-lobby".into())
            .spawn(move || {
                let session = match settings.zenoh_config().and_then(|c| zenoh::open(c).wait()) {
                    Ok(session) => session,
                    Err(error) => {
                        let message = describe_connect_error(&error.to_string(), &settings);
                        thread_shared.lock().unwrap().status = Some(LobbyStatus::Failed(message));
                        return;
                    }
                };
                thread_shared.lock().unwrap().status = Some(LobbyStatus::Connected);
                while !thread_stop.load(Ordering::Acquire) {
                    let rooms = list_rooms(&session);
                    thread_shared.lock().unwrap().rooms = rooms;
                    for _ in 0..20 {
                        if thread_stop.load(Ordering::Acquire) {
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(100));
                    }
                }
                let _ = session.close().wait();
            })
            .expect("could not start lobby thread");
        Self { shared, stop }
    }

    pub fn status(&self) -> LobbyStatus {
        self.shared
            .lock()
            .unwrap()
            .status
            .clone()
            .unwrap_or(LobbyStatus::Connecting)
    }

    pub fn rooms(&self) -> Vec<RoomInfo> {
        self.shared.lock().unwrap().rooms.clone()
    }
}

impl Drop for LobbyClient {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}

fn list_rooms(session: &zenoh::Session) -> Vec<RoomInfo> {
    let Ok(replies) = session
        .get(format!("{ROOMS}/*"))
        // Every room answers; the default target may stop at one.
        .target(zenoh::query::QueryTarget::All)
        .consolidation(zenoh::query::ConsolidationMode::None)
        .timeout(Duration::from_secs(2))
        .wait()
    else {
        return Vec::new();
    };
    let mut rooms = BTreeMap::new();
    while let Ok(reply) = replies.recv() {
        if let Ok(sample) = reply.result()
            && let Ok(room) = serde_json::from_slice::<RoomInfo>(&sample.payload().to_bytes())
            && room.prefix == crate::lobby::room_prefix(&room.code)
        {
            rooms.insert(room.code.clone(), room);
        }
    }
    let mut rooms: Vec<_> = rooms.into_values().collect();
    rooms.sort_by_cached_key(|room| room.name.to_lowercase());
    rooms
}

fn describe_connect_error(error: &str, settings: &OnlineSettings) -> String {
    let error = if settings.password.is_empty() {
        error.to_owned()
    } else {
        error.replace(&settings.password, "***")
    };
    if error.to_lowercase().contains("auth") {
        format!("Sign-in refused: check user and password ({error})")
    } else {
        format!("Could not reach {}: {error}", settings.server.trim())
    }
}

/// Rover commands a player sends. Same list as `is_command_kind` in bridge.rs.
fn is_command(kind: &str) -> bool {
    crate::bridge::is_command_kind(kind)
}
/// FPV signaling to a rover. The video goes peer to peer over WebRTC.
const FPV_COMMANDS: [&str; 3] = ["webrtc/request", "webrtc/answer", "webrtc/ice/dashboard"];
/// Per-rover state pulled for the player's team. `pose` and `next/status` come
/// for every rover through one wildcard subscription each.
const TEAM_STATE: [&str; 17] = [
    "localization",
    "navigation/outdoor",
    "autonomy/status",
    "hardware/status",
    "goal/status",
    "map/occupancy",
    "map/occupancy/heartbeat",
    "goal/proposal",
    "exploration/status",
    "search/status",
    "search/report",
    "mission/status",
    "experiment/status",
    "objects",
    "webrtc/offer",
    "webrtc/ice/phone",
    "webrtc/state",
];
const OWN_FRAMES: [&str; 2] = ["camera/rgb", "camera/depth"];
const EVERY_ROVER: [&str; 2] = ["pose", "next/status"];

/// What to join and as whom.
#[derive(Clone, Debug)]
pub struct JoinSpec {
    pub settings: OnlineSettings,
    pub room: RoomInfo,
    pub room_password: Option<String>,
    pub team: Team,
}

#[derive(Clone, Debug, PartialEq)]
pub enum RelayStatus {
    Joining,
    Playing(Seated),
    Failed(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Seated {
    pub seat: String,
    pub team: Team,
    pub rovers: Vec<u64>,
    pub phase: String,
    pub players: usize,
    pub rtt_ms: Option<f64>,
}

/// A running in-app relay. Dropping it leaves the room.
pub struct Relay {
    status: Arc<Mutex<RelayStatus>>,
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl Relay {
    /// The caller frees `LOCAL_ENDPOINT` first (`Bridge::suspend`).
    pub fn start(spec: JoinSpec) -> Self {
        let status = Arc::new(Mutex::new(RelayStatus::Joining));
        let stop = Arc::new(AtomicBool::new(false));
        let (thread_status, thread_stop) = (status.clone(), stop.clone());
        let handle = std::thread::Builder::new()
            .name("zorvane-relay".into())
            .spawn(move || {
                if let Err(error) = run_relay(&spec, &thread_status, &thread_stop) {
                    *thread_status.lock().unwrap() = RelayStatus::Failed(error);
                }
            })
            .expect("could not start relay thread");
        Self {
            status,
            stop,
            handle: Some(handle),
        }
    }

    pub fn status(&self) -> RelayStatus {
        self.status.lock().unwrap().clone()
    }
}

impl Drop for Relay {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

struct Seat {
    id: String,
    secret: String,
    team: Team,
    rovers: BTreeSet<u64>,
}

/// State the Zenoh callbacks share with the relay loop.
struct RelayContext {
    remote: zenoh::Session,
    local: zenoh::Session,
    remote_prefix: String,
    local_prefix: String,
    seat: Mutex<Seat>,
    seq: Mutex<u64>,
    match_state: Mutex<serde_json::Value>,
    missing: AtomicU32,
}

impl RelayContext {
    fn signed_put(&self, key: &str, payload: &[u8]) {
        let (seat, secret) = {
            let seat = self.seat.lock().unwrap();
            (seat.id.clone(), seat.secret.clone())
        };
        // Sign and send under one lock so sequence numbers leave in order.
        let mut seq = self.seq.lock().unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_micros() as u64);
        *seq = (*seq + 1).max(now);
        let attachment = crate::lobby::sign(&secret, &seat, *seq, key, payload);
        let _ = self
            .remote
            .put(key, payload.to_vec())
            .attachment(attachment)
            .wait();
    }

    /// Local client to host: own rovers' commands, signed.
    fn forward_local(&self, key: &str, payload: &[u8]) {
        let Some(rest) = key.strip_prefix(&format!("{}/", self.local_prefix)) else {
            return;
        };
        let Some((rover, kind)) = rest
            .split_once('/')
            .and_then(|(id, kind)| Some((id.parse::<u64>().ok()?, kind)))
        else {
            return;
        };
        let remote_key = format!("{}/{rest}", self.remote_prefix);
        let mine = self.seat.lock().unwrap().rovers.contains(&rover);
        if mine && (is_command(kind) || FPV_COMMANDS.contains(&kind)) {
            self.signed_put(&remote_key, payload);
        }
    }

    /// Host to local clients, with the prefix swapped.
    fn forward(&self, key: &str, payload: Vec<u8>) {
        if let Some(rest) = key.strip_prefix(&format!("{}/", self.remote_prefix)) {
            let _ = self
                .local
                .put(format!("{}/{rest}", self.local_prefix), payload)
                .wait();
        }
    }
}

fn join_request(spec: &JoinSpec, rovers: Option<Vec<u64>>) -> serde_json::Value {
    serde_json::json!({
        "name": spec.settings.name.trim(),
        "team": spec.team,
        "rovers": rovers.map_or(serde_json::json!("all"), |ids| serde_json::json!(ids)),
        "kind": "human",
        "code": spec.room_password,
    })
}

fn query_json(
    session: &zenoh::Session,
    key: &str,
    body: Option<&serde_json::Value>,
    timeout: Duration,
) -> Option<serde_json::Value> {
    let mut get = session.get(key).timeout(timeout);
    if let Some(body) = body {
        get = get.payload(serde_json::to_vec(body).unwrap());
    }
    let replies = get.wait().ok()?;
    while let Ok(reply) = replies.recv() {
        if let Ok(sample) = reply.result() {
            return serde_json::from_slice(&sample.payload().to_bytes()).ok();
        }
    }
    None
}

fn take_seat(
    remote: &zenoh::Session,
    spec: &JoinSpec,
    rovers: Option<Vec<u64>>,
) -> Result<Seat, String> {
    let reply = query_json(
        remote,
        &format!("{}/match/join", spec.room.prefix),
        Some(&join_request(spec, rovers)),
        Duration::from_secs(5),
    )
    .ok_or("The room did not answer. The host may have left.")?;
    if reply["ok"] != true {
        let error = reply["error"].as_str().unwrap_or("refused");
        return Err(if error == "wrong join code" {
            "Wrong room password".into()
        } else {
            format!("Join refused: {error}")
        });
    }
    Ok(Seat {
        id: reply["seat"].as_str().unwrap_or_default().to_owned(),
        secret: reply["secret"].as_str().unwrap_or_default().to_owned(),
        team: serde_json::from_value(reply["team"].clone()).unwrap_or(spec.team),
        rovers: reply["rovers"]
            .as_array()
            .map(|ids| ids.iter().filter_map(|id| id.as_u64()).collect())
            .unwrap_or_default(),
    })
}

fn open_local() -> Result<zenoh::Session, String> {
    let mut config = zenoh::Config::default();
    let set = |config: &mut zenoh::Config, key: &str, value: &str| {
        config
            .insert_json5(key, value)
            .map_err(|error| error.to_string())
    };
    set(&mut config, "mode", "\"peer\"")?;
    set(
        &mut config,
        "listen/endpoints",
        &serde_json::to_string(&[LOCAL_ENDPOINT]).unwrap(),
    )?;
    set(&mut config, "scouting/multicast/enabled", "false")?;
    // The simulator's own listener may take a moment to let go of the port.
    let mut last = String::new();
    for _ in 0..20 {
        match zenoh::open(config.clone()).wait() {
            Ok(session) => return Ok(session),
            Err(error) => last = error.to_string(),
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    Err(format!("Could not open {LOCAL_ENDPOINT} for ARGOS: {last}"))
}

fn run_relay(
    spec: &JoinSpec,
    status: &Mutex<RelayStatus>,
    stop: &AtomicBool,
) -> Result<(), String> {
    let remote = spec
        .settings
        .zenoh_config()
        .and_then(|config| zenoh::open(config).wait())
        .map_err(|error| describe_connect_error(&error.to_string(), &spec.settings))?;
    let seat = take_seat(&remote, spec, None)?;
    let local = open_local()?;
    let context = Arc::new(RelayContext {
        remote: remote.clone(),
        local: local.clone(),
        remote_prefix: spec.room.prefix.clone(),
        local_prefix: "terra/rover".into(),
        seat: Mutex::new(seat),
        seq: Mutex::new(0),
        match_state: Mutex::new(serde_json::Value::Null),
        missing: AtomicU32::new(0),
    });
    let rp = spec.room.prefix.clone();
    let mut subscribers = Vec::new();
    {
        let context = context.clone();
        subscribers.push(
            local
                .declare_subscriber("terra/rover/**")
                .callback(move |sample| {
                    context.forward_local(sample.key_expr().as_str(), &sample.payload().to_bytes())
                })
                .wait()
                .map_err(|error| error.to_string())?,
        );
    }
    let mut forward_from = |key: String| -> Result<(), String> {
        let context = context.clone();
        subscribers.push(
            remote
                .declare_subscriber(key)
                .callback(move |sample| {
                    if sample.key_expr().as_str().ends_with("/match/state") {
                        let state: serde_json::Value =
                            serde_json::from_slice(&sample.payload().to_bytes())
                                .unwrap_or_default();
                        let seat = context.seat.lock().unwrap().id.clone();
                        let seated = state["seats"]
                            .as_array()
                            .is_some_and(|seats| seats.iter().any(|s| s["seat"] == seat.as_str()));
                        if seated {
                            context.missing.store(0, Ordering::Release);
                        } else {
                            context.missing.fetch_add(1, Ordering::AcqRel);
                        }
                        *context.match_state.lock().unwrap() = state;
                    }
                    context.forward(
                        sample.key_expr().as_str(),
                        sample.payload().to_bytes().into_owned(),
                    )
                })
                .wait()
                .map_err(|error| error.to_string())?,
        );
        Ok(())
    };
    for key in ["match/state", "next/score", "fleet/state"] {
        forward_from(format!("{rp}/{key}"))?;
    }
    for kind in EVERY_ROVER {
        forward_from(format!("{rp}/*/{kind}"))?;
    }

    // Team rovers' full state, re-declared when the room's rovers change.
    let mut in_view: BTreeMap<u64, Vec<zenoh::pubsub::Subscriber<()>>> = BTreeMap::new();
    let mut last_heartbeat = Instant::now() - Duration::from_secs(10);
    let mut last_ping = Instant::now() - Duration::from_secs(10);
    let mut rtt_ms = None;
    while !stop.load(Ordering::Acquire) {
        let now = Instant::now();
        if now - last_heartbeat >= Duration::from_secs(2) {
            last_heartbeat = now;
            context.signed_put(&format!("{rp}/match/heartbeat"), b"{}");
        }
        if context.missing.load(Ordering::Acquire) >= 3 {
            // The host restarted or timed the seat out: ask for the same rovers.
            context.missing.store(0, Ordering::Release);
            let rovers: Vec<u64> = context
                .seat
                .lock()
                .unwrap()
                .rovers
                .iter()
                .copied()
                .collect();
            let seat = take_seat(&remote, spec, (!rovers.is_empty()).then_some(rovers))
                .or_else(|_| take_seat(&remote, spec, None))
                .map_err(|error| format!("Lost the seat: {error}"))?;
            *context.seat.lock().unwrap() = seat;
        }
        let state = context.match_state.lock().unwrap().clone();
        let (team, own) = {
            let seat = context.seat.lock().unwrap();
            (seat.team, seat.rovers.clone())
        };
        let mut wanted: BTreeSet<u64> = own.clone();
        let team_name = serde_json::to_value(team).unwrap();
        for seat in state["seats"].as_array().into_iter().flatten() {
            if seat["team"] == team_name {
                wanted.extend(
                    seat["rovers"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|id| id.as_u64()),
                );
            }
        }
        for free in state["free"].as_array().into_iter().flatten() {
            if free["team"] == team_name
                && let Some(id) = free["rover"].as_u64()
            {
                wanted.insert(id);
            }
        }
        in_view.retain(|id, _| wanted.contains(id));
        for id in wanted {
            if in_view.contains_key(&id) {
                continue;
            }
            let kinds = TEAM_STATE.iter().chain(
                own.contains(&id)
                    .then_some(OWN_FRAMES.iter())
                    .into_iter()
                    .flatten(),
            );
            let mut subs = Vec::new();
            for kind in kinds {
                let context = context.clone();
                if let Ok(subscriber) = remote
                    .declare_subscriber(format!("{rp}/{id}/{kind}"))
                    .callback(move |sample| {
                        context.forward(
                            sample.key_expr().as_str(),
                            sample.payload().to_bytes().into_owned(),
                        )
                    })
                    .wait()
                {
                    subs.push(subscriber);
                }
            }
            in_view.insert(id, subs);
        }
        if now - last_ping >= Duration::from_secs(5) {
            last_ping = now;
            let start = Instant::now();
            rtt_ms = query_json(
                &remote,
                &format!("{rp}/match/ping"),
                None,
                Duration::from_secs(2),
            )
            .map(|_| start.elapsed().as_secs_f64() * 1000.0);
        }
        *status.lock().unwrap() = RelayStatus::Playing(Seated {
            seat: context.seat.lock().unwrap().id.clone(),
            team,
            rovers: own.iter().copied().collect(),
            phase: state["phase"].as_str().unwrap_or("lobby").to_owned(),
            players: state["seats"].as_array().map_or(0, Vec::len),
            rtt_ms,
        });
        std::thread::sleep(Duration::from_millis(250));
    }
    context.signed_put(&format!("{rp}/match/leave"), b"{}");
    drop(in_view);
    drop(subscribers);
    let _ = local.close().wait();
    let _ = remote.close().wait();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_validate_endpoints_and_names() {
        let mut settings = OnlineSettings::default();
        assert_eq!(settings.validate(), Ok(()));
        settings.server = "argos.example:7448".into();
        assert!(settings.validate().is_err());
        settings.server = "tcp/127.0.0.1:7447".into();
        settings.name = "a/b".into();
        assert!(settings.validate().is_err());
    }

    #[test]
    fn room_info_parses_what_the_host_publishes() {
        let lobby = crate::lobby::Lobby::default();
        let meta = crate::lobby::RoomMeta {
            code: "K7Q2M".into(),
            name: "Yojan's room".into(),
            host: "yojan".into(),
            password: true,
        };
        let fleet = [(0, Team::Red), (1, Team::Blue)];
        let body = serde_json::to_vec(&lobby.room_info(&meta, &fleet, Instant::now())).unwrap();
        let room: RoomInfo = serde_json::from_slice(&body).unwrap();
        assert_eq!(room.prefix, crate::lobby::room_prefix("K7Q2M"));
        assert!(room.password);
        assert_eq!(room.free(Team::Blue), 1);
    }

    #[test]
    fn local_commands_are_signed_only_for_own_rovers() {
        // The prefix swap both ways, without a network.
        let rest = "terra/rover/3/teleop".strip_prefix("terra/rover/").unwrap();
        assert_eq!(
            format!("{}/{rest}", crate::lobby::room_prefix("AB2CD")),
            "zorvane/room/AB2CD/rover/3/teleop"
        );
        assert!(is_command("teleop") && !is_command("pose"));
    }
}
