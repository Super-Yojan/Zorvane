//! Transport-independent telemetry, leased control, mapping, and autonomy.
use crate::{
    depth_camera::{DepthCamera, DepthCameraConfig, DepthFrame},
    rgb_camera::RgbFrame,
    terra::{DriveCommand, KeyboardControlled, MAX_ROVERS, Rover, RoverFleet, RoverId},
};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
#[cfg(test)]
use terra_waypoint::GoalCommand;
use terra_waypoint::encode_status;
#[cfg(test)]
use terra_waypoint::{GoalState, WaypointConfig, WaypointController};
use std::time::{Duration, Instant};
use zenoh::Wait;
#[path = "native_bridge.rs"]
mod native;
use native::run_transport;
#[path = "bridge_recording.rs"]
mod recording;

#[path = "publish_gate.rs"]
mod publish_gate;

#[derive(Clone, Resource)]
pub struct BridgeConfig {
    pub enabled: bool,
    pub prefix: String,
    pub listen: String,
    /// Comma-separated locators, already split. Empty means no fleet session.
    pub connect: Vec<String>,
    pub user: Option<String>,
    pub password: Option<String>,
    pub tls_root_ca: Option<String>,
    pub config_file: Option<String>,
    pub frame_rate: f32,
    pub command_timeout: Duration,
}
impl Default for BridgeConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            prefix: "terra/rover".into(),
            listen: "tcp/127.0.0.1:7447".into(),
            connect: Vec::new(),
            user: None,
            password: None,
            tls_root_ca: None,
            config_file: None,
            frame_rate: 10.0,
            command_timeout: Duration::from_millis(500),
        }
    }
}
impl BridgeConfig {
    pub fn from_env() -> Self {
        Self {
            enabled: std::env::var("TERRA_ZENOH").map_or(true, |v| v != "0"),
            prefix: std::env::var("TERRA_ZENOH_PREFIX").unwrap_or_else(|_| "terra/rover".into()),
            listen: std::env::var("TERRA_ZENOH_LISTEN")
                .unwrap_or_else(|_| "tcp/127.0.0.1:7447".into()),
            connect: split_endpoints(&std::env::var("TERRA_ZENOH_CONNECT").unwrap_or_default()),
            user: nonempty_env("TERRA_ZENOH_USER", true),
            password: nonempty_env("TERRA_ZENOH_PASSWORD", false),
            tls_root_ca: nonempty_env("TERRA_ZENOH_TLS_ROOT_CA", true),
            config_file: nonempty_env("TERRA_ZENOH_CONFIG", true),
            ..default()
        }
    }
    fn validate(&self) -> Result<(), &'static str> {
        if !self.frame_rate.is_finite()
            || self.frame_rate < 0.1
            || self.frame_rate > 60.0
            || self.command_timeout.is_zero()
        {
            return Err("frame rate must be in [0.1,60] and command timeout must be positive");
        }
        if !crate::control_queue::valid_topic(&self.prefix, false) {
            return Err("prefix must be a concrete Zenoh key expression");
        }
        if self.config_path().is_none() {
            self.link_specs()?;
        }
        Ok(())
    }
    fn config_path(&self) -> Option<&str> {
        self.config_file
            .as_deref()
            .map(str::trim)
            .filter(|path| !path.is_empty())
    }
    /// Local peer, then optional fleet client. `TERRA_ZENOH_CONFIG` replaces both.
    #[cfg(test)]
    fn built_configs(&self) -> Result<Vec<zenoh::Config>, String> {
        if let Some(path) = self.config_path() {
            return zenoh::Config::from_file(path)
                .map(|config| vec![config])
                .map_err(|error| error.to_string());
        }
        let (local, fleet) = self.link_specs().map_err(str::to_string)?;
        local
            .into_iter()
            .chain(fleet)
            .map(|spec| spec.to_zenoh_config().map_err(|error| error.to_string()))
            .collect()
    }
    /// The listener stays a separate peer. Zenoh usrpwd covers every link on a
    /// session, so fleet credentials never share that listener.
    fn link_specs(&self) -> Result<(Option<ZenohLinkSpec>, Option<ZenohLinkSpec>), &'static str> {
        let listen = trimmed(&self.listen);
        let connect: Vec<String> = self
            .connect
            .iter()
            .filter_map(|endpoint| trimmed(endpoint))
            .collect();
        let user = self
            .user
            .as_deref()
            .map(str::trim)
            .filter(|user| !user.is_empty());
        let password = self
            .password
            .as_deref()
            .filter(|password| !password.is_empty());
        let tls_root_ca = self.tls_root_ca.as_deref().and_then(trimmed);
        match (user, password) {
            (None, None) => {}
            (Some(_), Some(_)) => {
                if connect.is_empty() {
                    return Err("TERRA_ZENOH_CONNECT is required when TERRA_ZENOH_USER is set");
                }
            }
            _ => {
                return Err("TERRA_ZENOH_USER and TERRA_ZENOH_PASSWORD must both be set");
            }
        }
        if tls_root_ca.is_some() && connect.is_empty() {
            return Err("TERRA_ZENOH_TLS_ROOT_CA requires TERRA_ZENOH_CONNECT");
        }
        for endpoint in listen.iter().chain(&connect) {
            if !valid_locator(endpoint) {
                return Err("Zenoh endpoints must be tcp/ or tls/ locators");
            }
        }
        if listen.is_none() && connect.is_empty() {
            return Err("set TERRA_ZENOH_LISTEN or TERRA_ZENOH_CONNECT");
        }
        let local = listen.map(|listen| ZenohLinkSpec {
            mode: "peer",
            listen: vec![listen],
            connect: Vec::new(),
            user: None,
            password: None,
            tls_root_ca: None,
            exit_on_connect_failure: false,
        });
        let fleet = if connect.is_empty() {
            None
        } else {
            Some(ZenohLinkSpec {
                mode: "client",
                listen: Vec::new(),
                connect,
                user: user.map(str::to_owned),
                password: password.map(str::to_owned),
                tls_root_ca,
                exit_on_connect_failure: true,
            })
        };
        Ok((local, fleet))
    }
}
fn nonempty_env(key: &str, trim: bool) -> Option<String> {
    let value = std::env::var(key).ok()?;
    let value = if trim { value.trim().to_owned() } else { value };
    (!value.is_empty()).then_some(value)
}
fn split_endpoints(value: &str) -> Vec<String> {
    value.split(',').filter_map(trimmed).collect()
}
fn trimmed(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}
fn valid_locator(endpoint: &str) -> bool {
    (endpoint.starts_with("tcp/") || endpoint.starts_with("tls/"))
        && endpoint.parse::<zenoh::config::EndPoint>().is_ok()
}
/// One Zenoh session. The fleet client is a second spec when `connect` is set.
struct ZenohLinkSpec {
    mode: &'static str,
    listen: Vec<String>,
    connect: Vec<String>,
    user: Option<String>,
    password: Option<String>,
    tls_root_ca: Option<String>,
    exit_on_connect_failure: bool,
}
impl ZenohLinkSpec {
    fn to_zenoh_config(&self) -> zenoh::Result<zenoh::Config> {
        let mut config = zenoh::Config::default();
        config.insert_json5("mode", &format!("\"{}\"", self.mode))?;
        if !self.listen.is_empty() {
            config.insert_json5(
                "listen/endpoints",
                &serde_json::to_string(&self.listen).expect("listen endpoints"),
            )?;
        }
        if !self.connect.is_empty() {
            config.insert_json5(
                "connect/endpoints",
                &serde_json::to_string(&self.connect).expect("connect endpoints"),
            )?;
        }
        config.insert_json5("scouting/multicast/enabled", "false")?;
        if self.exit_on_connect_failure {
            // Client mode already returns the connect error. Keep that if defaults change.
            config.insert_json5("connect/exit_on_failure", "true")?;
        }
        if let (Some(user), Some(password)) = (&self.user, &self.password) {
            config.insert_json5(
                "transport/auth/usrpwd/user",
                &serde_json::to_string(user).expect("user"),
            )?;
            config.insert_json5(
                "transport/auth/usrpwd/password",
                &serde_json::to_string(password).expect("password"),
            )?;
        }
        // Zenoh 1.10 TLS clients load webpki roots either way. A set CA is appended.
        if let Some(ca) = &self.tls_root_ca {
            config.insert_json5(
                "transport/link/tls/root_ca_certificate",
                &serde_json::to_string(ca).expect("root ca"),
            )?;
        }
        Ok(config)
    }
}
#[derive(Default)]
pub struct SimulatorBridgePlugin {
    pub config: BridgeConfig,
}
impl Plugin for SimulatorBridgePlugin {
    fn build(&self, app: &mut App) {
        if !self.config.enabled {
            return;
        }
        self.config
            .validate()
            .expect("invalid Terra Zenoh configuration");
        let shared = Arc::new(Shared::default());
        {
            let thread_shared = shared.clone();
            let config = self.config.clone();
            std::thread::Builder::new()
                .name("terra-zenoh".into())
                .spawn(move || {
                    if let Err(error) = run_transport(&config, &thread_shared) {
                        warn!("Terra Zenoh transport stopped: {error}");
                    }
                    thread_shared.connected.store(false, Ordering::Release);
                })
                .expect("could not start Zenoh worker");
        }
        app.insert_resource(self.config.clone())
            .insert_resource(Bridge { shared })
            .add_systems(
                PreUpdate,
                (
                    receive_fleet_request.before(crate::terra::reconcile_fleet),
                    claim_rovers.after(crate::terra::reconcile_fleet),
                ),
            )
            .add_systems(FixedUpdate, drive_autonomy)
            .add_systems(Update, (publish_frames, forward_next_commands, publish_next_state));
        if !app
            .world()
            .contains_resource::<Messages<crate::next_competition::NextCommand>>()
        {
            app.add_message::<crate::next_competition::NextCommand>();
        }
    }
}
#[derive(Default)]
struct Outgoing {
    frames: BTreeMap<(u64, &'static str), Vec<u8>>,
    fleet: Option<Vec<u8>>,
    goals: BTreeMap<u64, Vec<u8>>,
    states: BTreeMap<(u64, String), Vec<u8>>,
    occupancy: BTreeMap<u64, OccupancyPublication>,
    occupancy_heartbeats: std::collections::BTreeSet<u64>,
    reports: BTreeMap<u64, Vec<Vec<u8>>>,
    poses: BTreeMap<u64, Vec<u8>>,
    /// `<prefix>/next/score`, when a NEXT match is running.
    next_score: Option<Vec<u8>>,
}
struct OccupancyPublication {
    payload: Vec<u8>,
    baseline: OccupancyBaseline,
}
#[derive(Serialize)]
struct OccupancyBaseline {
    schema_version: u8,
    rover_id: u64,
    run_id: String,
    sequence: u64,
}
impl Outgoing {
    fn queue_occupancy(
        &mut self,
        id: u64,
        run_id: &str,
        sequence: u64,
        grid: &terra_mapping::MapSnapshot,
    ) {
        self.occupancy_heartbeats.remove(&id);
        self.occupancy.insert(
            id,
            OccupancyPublication {
                payload: serde_json::to_vec(&terra_autonomy::occupancy_telemetry(
                    id, run_id, sequence, grid,
                ))
                .unwrap(),
                baseline: OccupancyBaseline {
                    schema_version: 1,
                    rover_id: id,
                    run_id: run_id.into(),
                    sequence,
                },
            },
        );
    }
}
#[cfg(test)]
#[derive(Clone)]
struct GoalInbox {
    seq: u64,
    command: GoalCommand,
}
type LeasedCommand = Option<(DriveCommand, Instant)>;
#[derive(Default)]
struct Shared {
    commands: Mutex<BTreeMap<u64, LeasedCommand>>,
    #[cfg(test)]
    goals: Mutex<BTreeMap<u64, GoalInbox>>,
    actions: Mutex<BTreeMap<u64, std::collections::VecDeque<(String, Vec<u8>, Instant)>>>,
    authorities: Mutex<BTreeMap<u64, (String, u64)>>,
    epoch: std::sync::atomic::AtomicU64,
    connection_generation: std::sync::atomic::AtomicU64,
    fleet_request: Mutex<Option<usize>>,
    /// Dashboard LOAD and DUMP requests, by rover id.
    next_commands: Mutex<Vec<crate::next_competition::NextCommand>>,
    outgoing: Mutex<Outgoing>,
    connected: AtomicBool,
    stop: AtomicBool,
}
impl Shared {
    fn receive(&self, prefix: &str, key: &str, bytes: &[u8]) {
        if bytes.len() > 2048 {
            return;
        }
        if key == format!("{prefix}/fleet/size") {
            if let Some(count) = decode_fleet_request(bytes) {
                *self.fleet_request.lock().unwrap() = Some(count);
            }
            return;
        }
        let Some((id, kind)) = key
            .strip_prefix(&format!("{prefix}/"))
            .and_then(|rest| rest.split_once('/'))
        else {
            return;
        };
        let Ok(id) = id.parse::<u64>() else {
            return;
        };
        if !self.commands.lock().unwrap().contains_key(&id) {
            return;
        }
        if let Some(kind) = crate::next_competition::NextCommandKind::from_topic(kind) {
            self.next_commands
                .lock()
                .unwrap()
                .push(crate::next_competition::NextCommand { rover: id, kind });
            return;
        }
        if matches!(
            kind,
            "autonomy"
                | "teleop"
                | "safety"
                | "goal"
                | "goal/decision"
                | "mission/report"
                | "navigation/settings"
                | "navigation/source"
                | "navigation/resume"
                | "hardware"
                | "search"
                | "search/action"
                | "search/report/ack"
        ) {
            let mut actions = self.actions.lock().unwrap();
            crate::control_queue::enqueue_control_action(
                actions.entry(id).or_default(),
                kind.into(),
                bytes.to_vec(),
                Instant::now(),
            );
        }
    }

    #[cfg(test)]
    fn disconnect(&self) {
        self.connected.store(false, Ordering::Release);
        self.commands
            .lock()
            .unwrap()
            .values_mut()
            .for_each(|slot| *slot = None);
        self.authorities.lock().unwrap().clear();
        #[cfg(test)]
        self.goals.lock().unwrap().clear();
        self.actions.lock().unwrap().clear();
        *self.fleet_request.lock().unwrap() = None;
        *self.outgoing.lock().unwrap() = Outgoing::default();
    }
}
#[derive(Resource)]
struct Bridge {
    shared: Arc<Shared>,
}
impl Drop for Bridge {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Release);
    }
}
#[derive(Component)]
struct ZenohControlled;

/// Hands dashboard LOAD and DUMP requests to the NEXT game.
fn forward_next_commands(
    bridge: Res<Bridge>,
    mut writer: MessageWriter<crate::next_competition::NextCommand>,
) {
    for command in std::mem::take(&mut *bridge.shared.next_commands.lock().unwrap()) {
        writer.write(command);
    }
}

/// Publishes each rover's NEXT state and the score, five times a second.
fn publish_next_state(
    bridge: Res<Bridge>,
    time: Res<Time>,
    mut last: Local<f64>,
    config: Option<Res<crate::world::WorldConfig>>,
    score: Option<Res<crate::next_competition::NextTeamScore>>,
    rovers: Query<(
        &RoverId,
        &crate::terra::RoverSlot,
        &crate::next_competition::RoverBallLoad,
        Option<&crate::next_competition::DumpReady>,
    )>,
) {
    let now = time.elapsed_secs_f64();
    if now - *last < 0.2
        || config.is_none()
    {
        return;
    }
    *last = now;
    let mut outgoing = bridge.shared.outgoing.lock().unwrap();
    for (id, slot, load, ready) in &rovers {
        let status = crate::next_competition::status_json(slot.0, *load, ready.is_some_and(|r| r.0));
        outgoing
            .states
            .insert((id.0, "next/status".into()), serde_json::to_vec(&status).unwrap());
    }
    if let Some(score) = score {
        let score = crate::next_competition::score_json(*score);
        outgoing.next_score = Some(serde_json::to_vec(&score).unwrap());
    }
}
fn receive_fleet_request(bridge: Res<Bridge>, fleet: Option<ResMut<RoverFleet>>) {
    if let Some(count) = bridge.shared.fleet_request.lock().unwrap().take()
        && let Some(mut fleet) = fleet
    {
        let _ = fleet.set_count(count);
    }
}
fn claim_rovers(
    mut commands: Commands,
    bridge: Res<Bridge>,
    rovers: Query<(Entity, &RoverId, Option<&ZenohControlled>), With<Rover>>,
) {
    let mut incoming = bridge.shared.commands.lock().unwrap();
    let ids: std::collections::BTreeSet<_> = rovers.iter().map(|(_, id, _)| id.0).collect();
    incoming.retain(|id, _| ids.contains(id));
    for (entity, id, controlled) in &rovers {
        incoming.entry(id.0).or_default();
        if controlled.is_none() {
            commands
                .entity(entity)
                .remove::<KeyboardControlled>()
                .insert(ZenohControlled);
        }
    }
}
#[cfg(test)]
type RemoteRovers<'w, 's> = Query<
    'w,
    's,
    (&'static RoverId, &'static mut DriveCommand),
    (With<Rover>, With<ZenohControlled>),
>;

#[cfg(test)]
fn apply_remote_commands(
    bridge: Res<Bridge>,
    config: Res<BridgeConfig>,
    keys: Option<Res<ButtonInput<KeyCode>>>,
    mut rovers: RemoteRovers<'_, '_>,
) {
    let ready = bridge.shared.connected.load(Ordering::Acquire)
        && !keys.is_some_and(|keys| keys.pressed(KeyCode::Space));
    let incoming = bridge.shared.commands.lock().unwrap();
    for (id, mut request) in &mut rovers {
        *request = if ready {
            fresh_command(
                incoming.get(&id.0).copied().flatten(),
                Instant::now(),
                config.command_timeout,
            )
        } else {
            DriveCommand::default()
        };
    }
}
#[cfg(test)]
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Twist {
    linear: f32,
    angular: f32,
}
#[cfg(test)]
fn decode_command(bytes: &[u8]) -> Option<DriveCommand> {
    if bytes.len() > 2048 {
        return None;
    }
    let Twist { linear, angular } = serde_json::from_slice(bytes).ok()?;
    (linear.is_finite() && angular.is_finite()).then_some(DriveCommand { linear, angular })
}
#[cfg(test)]
fn fresh_command(
    command: Option<(DriveCommand, Instant)>,
    now: Instant,
    timeout: Duration,
) -> DriveCommand {
    command
        .filter(|(_, received)| now.saturating_duration_since(*received) < timeout)
        .map(|(command, _)| command)
        .unwrap_or_default()
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FleetRequest {
    count: usize,
}
fn decode_fleet_request(bytes: &[u8]) -> Option<usize> {
    if bytes.len() > 2048 {
        return None;
    }
    let request: FleetRequest = serde_json::from_slice(bytes).ok()?;
    (request.count <= MAX_ROVERS).then_some(request.count)
}

#[cfg(test)]
struct TrackedGoal {
    seq: u64,
    follower: WaypointController,
    last_payload: Vec<u8>,
    last_sent: Option<Instant>,
}
#[cfg(test)]
impl TrackedGoal {
    fn new() -> Self {
        Self {
            seq: 0,
            follower: WaypointController::new(WaypointConfig::default())
                .expect("default waypoint configuration"),
            last_payload: Vec::new(),
            last_sent: None,
        }
    }
}
#[cfg(test)]
#[derive(Default)]
struct GoalRuntime {
    rovers: BTreeMap<u64, TrackedGoal>,
}

#[cfg(test)]
fn drive_goals(
    bridge: Res<Bridge>,
    world: Option<Res<crate::world::WorldConfig>>,
    keys: Option<Res<ButtonInput<KeyCode>>>,
    mut rovers: Query<
        (&RoverId, &Transform, &mut DriveCommand),
        (With<Rover>, With<ZenohControlled>),
    >,
    mut runtime: Local<GoalRuntime>,
) {
    let incoming = bridge.shared.goals.lock().unwrap().clone();
    let half = world
        .as_ref()
        .map(|world| (f64::from(world.size) * 0.5 - 1.0).max(1.0))
        .unwrap_or(500.0);
    let anchor = world
        .as_ref()
        .and(crate::mason_field::geographic_reference());
    let paused = keys.is_some_and(|keys| keys.pressed(KeyCode::Space));
    let mut seen = std::collections::BTreeSet::new();
    for (id, transform, mut command) in &mut rovers {
        seen.insert(id.0);
        let tracked = runtime.rovers.entry(id.0).or_insert_with(TrackedGoal::new);
        if let Some(anchor) = anchor {
            let _ = tracked
                .follower
                .set_origin(anchor.latitude, anchor.longitude);
        }
        if let Some(inbox) = incoming.get(&id.0)
            && inbox.seq != tracked.seq
        {
            tracked.seq = inbox.seq;
            let _ = tracked.follower.accept(&inbox.command, half);
        }
        let (x, y, yaw) =
            crate::robotics_pose::robotics_pose(transform.translation, transform.rotation);
        let output = tracked.follower.step(x, y, yaw);
        if output.status.state != GoalState::Idle {
            if paused || output.status.state != GoalState::Active {
                *command = DriveCommand::default();
            } else {
                *command = DriveCommand {
                    linear: output.linear as f32,
                    angular: output.angular as f32,
                };
            }
        }
        let status = output.status;
        let Some(payload) = encode_status(&status) else {
            continue;
        };
        let period = if status.state == GoalState::Idle {
            Duration::from_secs(1)
        } else {
            Duration::from_millis(200)
        };
        let due = tracked
            .last_sent
            .is_none_or(|sent| sent.elapsed() >= period);
        if payload != tracked.last_payload || due {
            bridge
                .shared
                .outgoing
                .lock()
                .unwrap()
                .goals
                .insert(id.0, payload.clone());
            tracked.last_payload = payload;
            tracked.last_sent = Some(Instant::now());
        }
    }
    runtime.rovers.retain(|id, _| seen.contains(id));
}

#[derive(Serialize)]
struct FrameHeader {
    rover_id: u64,
    version: u8,
    width: u32,
    height: u32,
    sequence: u64,
    received_at: f64,
    encoding: &'static str,
    vertical_fov: f32,
    near: f32,
    far: f32,
    /// Simulation time paired with this GPU copy. Omitted when the frame has no exposure.
    #[serde(skip_serializing_if = "Option::is_none")]
    exposure_time: Option<f64>,
    /// Optical pose in the robotics world (position X/Y/Z, quaternion from optical axes). Depth only.
    #[serde(skip_serializing_if = "Option::is_none")]
    camera: Option<CameraPoseHeader>,
    /// Rover body origin and heading in the same robotics frame, sampled with the exposure.
    #[serde(skip_serializing_if = "Option::is_none")]
    body: Option<BodyPoseHeader>,
}
#[derive(Serialize)]
struct CameraPoseHeader {
    x: f64,
    y: f64,
    z: f64,
    qx: f64,
    qy: f64,
    qz: f64,
    qw: f64,
}
#[derive(Serialize)]
struct BodyPoseHeader {
    x: f64,
    y: f64,
    yaw: f64,
}
fn packet(header: FrameHeader, pixels: &[u8]) -> Option<Vec<u8>> {
    let mut bytes = serde_json::to_vec(&header).ok()?;
    bytes.push(b'\n');
    bytes.extend_from_slice(pixels);
    Some(bytes)
}
#[cfg(test)]
fn encode_depth(frame: &DepthFrame) -> Option<Vec<u8>> {
    encode_depth_with_config(0, frame, &DepthCameraConfig::default())
}
fn encode_depth_with_config(
    id: u64,
    frame: &DepthFrame,
    config: &DepthCameraConfig,
) -> Option<Vec<u8>> {
    if frame.depth_metres.len() != frame.width as usize * frame.height as usize {
        return None;
    }
    let pixels: Vec<u8> = frame
        .depth_metres
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect();
    let (exposure_time, camera, body) = frame.exposure.map_or((None, None, None), |exposure| {
        let pose = crate::occupancy_map::camera_pose(&exposure.camera_transform);
        let (x, y, yaw) = crate::occupancy_map::body_pose(&exposure.body_transform);
        (
            Some(exposure.timestamp),
            Some(CameraPoseHeader {
                x: pose.position.x,
                y: pose.position.y,
                z: pose.position.z,
                qx: pose.orientation.x,
                qy: pose.orientation.y,
                qz: pose.orientation.z,
                qw: pose.orientation.w,
            }),
            Some(BodyPoseHeader { x, y, yaw }),
        )
    });
    packet(
        FrameHeader {
            rover_id: id,
            version: 1,
            width: frame.width,
            height: frame.height,
            sequence: frame.sequence,
            received_at: frame.received_at,
            encoding: "32FC1_LE",
            vertical_fov: config.vertical_fov,
            near: config.near,
            far: config.far,
            exposure_time,
            camera,
            body,
        },
        &pixels,
    )
}
#[derive(Default)]
struct PublishState {
    epoch: u64,
    last_publish: Option<Instant>,
    sequences: BTreeMap<u64, (Option<u64>, Option<u64>)>,
    fleet_ids: Vec<u64>,
    last_fleet_publish: Option<Instant>,
}
fn publish_frames(
    bridge: Res<Bridge>,
    config: Res<BridgeConfig>,
    depth: Query<(&RoverId, &DepthCamera, &DepthFrame)>,
    rgb: Query<(&RoverId, &RgbFrame, &crate::rgb_camera::RgbCamera)>,
    rovers: Query<(&RoverId, &Transform), With<Rover>>,
    mut state: Local<PublishState>,
) {
    let epoch = bridge.shared.epoch.load(Ordering::Relaxed);
    if state.epoch != epoch {
        *state = PublishState { epoch, ..default() };
    }
    let now = Instant::now();
    if !bridge.shared.connected.load(Ordering::Acquire)
        || state.last_publish.is_some_and(|last| {
            now.duration_since(last) < Duration::from_secs_f32(1.0 / config.frame_rate)
        })
    {
        return;
    }
    state.last_publish = Some(now);
    let mut ids: Vec<_> = rovers.iter().map(|(id, _)| id.0).collect();
    ids.sort();
    state
        .sequences
        .retain(|id, _| ids.binary_search(id).is_ok());
    let mut outgoing = bridge.shared.outgoing.lock().unwrap();
    outgoing
        .frames
        .retain(|(id, _), _| ids.binary_search(id).is_ok());
    if state.fleet_ids != ids
        || state
            .last_fleet_publish
            .is_none_or(|last| now.duration_since(last) >= Duration::from_secs(1))
    {
        outgoing.fleet = Some(
            serde_json::to_vec(
                &serde_json::json!({"count": ids.len(), "max_count": MAX_ROVERS, "ids": ids}),
            )
            .unwrap(),
        );
        state.fleet_ids = ids.clone();
        state.last_fleet_publish = Some(now);
    }
    for (id, transform) in &rovers {
        let (x, y, yaw) =
            crate::robotics_pose::robotics_pose(transform.translation, transform.rotation);
        outgoing
            .poses
            .insert(id.0, crate::robotics_pose::pose_payload(x, y, yaw));
    }
    for (id, camera, frame) in &depth {
        if ids.binary_search(&id.0).is_err() {
            continue;
        }
        let sequences = state.sequences.entry(id.0).or_default();
        if frame.sequence > 0
            && sequences.0 != Some(frame.sequence)
            && let Some(packet) = encode_depth_with_config(id.0, frame, &camera.config)
        {
            outgoing.frames.insert((id.0, "depth"), packet);
            sequences.0 = Some(frame.sequence);
        }
    }
    for (id, frame, camera) in &rgb {
        if ids.binary_search(&id.0).is_err() {
            continue;
        }
        let sequences = state.sequences.entry(id.0).or_default();
        if frame.sequence > 0
            && sequences.1 != Some(frame.sequence)
            && frame.rgba.len() == frame.width as usize * frame.height as usize * 4
            && let Some(packet) = packet(
                FrameHeader {
                    rover_id: id.0,
                    version: 1,
                    width: frame.width,
                    height: frame.height,
                    sequence: frame.sequence,
                    received_at: frame.received_at,
                    encoding: "RGBA8_SRGB",
                    vertical_fov: camera.config.vertical_fov,
                    near: camera.config.near,
                    far: camera.config.far,
                    exposure_time: None,
                    camera: None,
                    body: None,
                },
                &frame.rgba,
            )
        {
            outgoing.frames.insert((id.0, "rgb"), packet);
            sequences.1 = Some(frame.sequence);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    fn json_at(config: &zenoh::Config, key: &str) -> serde_json::Value {
        serde_json::from_str(
            &config
                .get_json(key)
                .unwrap_or_else(|_| panic!("missing {key}")),
        )
        .unwrap()
    }

    fn configs(config: BridgeConfig) -> Vec<zenoh::Config> {
        config.built_configs().unwrap()
    }

    fn mount_replay_runtimes(mut commands:Commands,rovers:Query<(Entity,Option<&Transform>),(With<Rover>,Without<crate::velocity_controller::VelocityControlState>)>){
        for(entity,transform)in &rovers{
            let rt=terra_runtime::Runtime::new(terra_runtime::RuntimeConfig{mode:terra_runtime::ActuationMode::DirectEffort,..default()},"test".into()).unwrap();
            commands.entity(entity).insert(crate::velocity_controller::VelocityControlState::new(rt));
            if transform.is_none(){commands.entity(entity).insert(Transform::default());}
        }
    }
    fn publish_bound_drive(app:&App,client:&zenoh::Session,id:u64,linear:f64,angular:f64,sequence:u64){
        let shared=&app.world().resource::<Bridge>().shared;
        if let Some((run,revision))=shared.authorities.lock().unwrap().get(&id){
            client.put(format!("terra/rover/{id}/teleop"),serde_json::json!({"linear":linear,"angular":angular,"run_id":run,"authority_revision":revision,"operator_session_id":"live-test","sequence":sequence}).to_string()).wait().unwrap();
        }
    }
    #[test]
    fn default_session_only_listens_on_localhost() {
        let built = configs(BridgeConfig::default());
        assert_eq!(built.len(), 1);
        assert_eq!(json_at(&built[0], "mode"), "peer");
        assert_eq!(
            json_at(&built[0], "listen/endpoints"),
            serde_json::json!(["tcp/127.0.0.1:7447"])
        );
        assert_eq!(
            json_at(&built[0], "connect/endpoints"),
            serde_json::json!([])
        );
        assert_eq!(json_at(&built[0], "scouting/multicast/enabled"), false);
        assert_eq!(
            json_at(&built[0], "transport/auth/usrpwd/user"),
            serde_json::Value::Null
        );
        assert_eq!(
            json_at(&built[0], "transport/link/tls/root_ca_certificate"),
            serde_json::Value::Null
        );
    }

    #[test]
    fn connect_keeps_a_separate_local_listener() {
        let built = configs(BridgeConfig {
            connect: split_endpoints(
                " tcp/10.0.0.8:7447, ,tls/argos.34.135.204.210.sslip.io:7448 ",
            ),
            ..default()
        });
        assert_eq!(built.len(), 2);
        assert_eq!(json_at(&built[0], "mode"), "peer");
        assert_eq!(
            json_at(&built[0], "listen/endpoints"),
            serde_json::json!(["tcp/127.0.0.1:7447"])
        );
        assert_eq!(
            json_at(&built[0], "connect/endpoints"),
            serde_json::json!([])
        );
        assert_eq!(
            json_at(&built[0], "transport/auth/usrpwd/password"),
            serde_json::Value::Null
        );
        assert_eq!(json_at(&built[1], "mode"), "client");
        assert!(
            !json_at(&built[1], "listen/endpoints")
                .to_string()
                .contains("127.0.0.1:7447")
        );
        assert_eq!(
            json_at(&built[1], "connect/endpoints"),
            serde_json::json!([
                "tcp/10.0.0.8:7447",
                "tls/argos.34.135.204.210.sslip.io:7448"
            ])
        );
        assert_eq!(json_at(&built[1], "scouting/multicast/enabled"), false);
        assert_eq!(json_at(&built[1], "connect/exit_on_failure"), true);
        assert_eq!(
            json_at(&built[1], "transport/auth/usrpwd/user"),
            serde_json::Value::Null
        );
        assert_eq!(
            json_at(&built[1], "transport/link/tls/root_ca_certificate"),
            serde_json::Value::Null
        );
    }

    #[test]
    fn credentials_and_optional_root_ca_stay_on_the_fleet_client() {
        let built = configs(BridgeConfig {
            prefix: "terra/phone".into(),
            connect: vec!["tls/argos.34.135.204.210.sslip.io:7448".into()],
            user: Some("terra".into()),
            password: Some("not-the-fleet-password".into()),
            tls_root_ca: Some("/tmp/extra-ca.pem".into()),
            ..default()
        });
        assert_eq!(built.len(), 2);
        assert_eq!(
            json_at(&built[0], "transport/auth/usrpwd/user"),
            serde_json::Value::Null
        );
        assert_eq!(
            json_at(&built[0], "transport/auth/usrpwd/password"),
            serde_json::Value::Null
        );
        assert_eq!(
            json_at(&built[0], "transport/link/tls/root_ca_certificate"),
            serde_json::Value::Null
        );
        assert_eq!(json_at(&built[1], "transport/auth/usrpwd/user"), "terra");
        assert_eq!(
            json_at(&built[1], "transport/auth/usrpwd/password"),
            "not-the-fleet-password"
        );
        assert_eq!(
            json_at(&built[1], "transport/link/tls/root_ca_certificate"),
            "/tmp/extra-ca.pem"
        );
        assert_eq!(
            json_at(&built[1], "connect/endpoints"),
            serde_json::json!(["tls/argos.34.135.204.210.sslip.io:7448"])
        );
    }

    #[test]
    fn empty_listen_is_a_fleet_client_only() {
        let built = configs(BridgeConfig {
            listen: "  ".into(),
            connect: vec!["tcp/127.0.0.1:7449".into()],
            ..default()
        });
        assert_eq!(built.len(), 1);
        assert_eq!(json_at(&built[0], "mode"), "client");
        assert_eq!(
            json_at(&built[0], "connect/endpoints"),
            serde_json::json!(["tcp/127.0.0.1:7449"])
        );
    }

    #[test]
    fn config_file_replaces_listen_connect_and_credentials() {
        let path =
            std::env::temp_dir().join(format!("zorvane-zenoh-config-{}.json5", std::process::id()));
        std::fs::write(
            &path,
            r#"{ mode: "client", connect: { endpoints: ["tcp/127.0.0.1:9"] } }"#,
        )
        .unwrap();
        let built = configs(BridgeConfig {
            listen: "not-a-locator".into(),
            connect: vec!["tls/argos.34.135.204.210.sslip.io:7448".into()],
            user: Some("terra".into()),
            password: Some("not-the-fleet-password".into()),
            config_file: Some(path.display().to_string()),
            ..default()
        });
        let _ = std::fs::remove_file(&path);
        assert_eq!(built.len(), 1);
        assert_eq!(json_at(&built[0], "mode"), "client");
        assert_eq!(
            json_at(&built[0], "connect/endpoints"),
            serde_json::json!(["tcp/127.0.0.1:9"])
        );
        assert_eq!(
            json_at(&built[0], "transport/auth/usrpwd/user"),
            serde_json::Value::Null
        );
        assert_eq!(
            json_at(&built[0], "transport/auth/usrpwd/password"),
            serde_json::Value::Null
        );
    }

    #[test]
    fn link_spec_rejects_incomplete_or_invalid_endpoints() {
        let err = |config: BridgeConfig| match config.link_specs() {
            Err(error) => error,
            Ok(_) => panic!("expected a Zenoh configuration error"),
        };
        assert_eq!(
            err(BridgeConfig {
                user: Some("terra".into()),
                ..default()
            }),
            "TERRA_ZENOH_USER and TERRA_ZENOH_PASSWORD must both be set"
        );
        assert_eq!(
            err(BridgeConfig {
                password: Some("not-the-fleet-password".into()),
                ..default()
            }),
            "TERRA_ZENOH_USER and TERRA_ZENOH_PASSWORD must both be set"
        );
        assert_eq!(
            err(BridgeConfig {
                user: Some("terra".into()),
                password: Some("not-the-fleet-password".into()),
                ..default()
            }),
            "TERRA_ZENOH_CONNECT is required when TERRA_ZENOH_USER is set"
        );
        assert_eq!(
            err(BridgeConfig {
                tls_root_ca: Some("/tmp/extra-ca.pem".into()),
                ..default()
            }),
            "TERRA_ZENOH_TLS_ROOT_CA requires TERRA_ZENOH_CONNECT"
        );
        assert_eq!(
            err(BridgeConfig {
                listen: String::new(),
                ..default()
            }),
            "set TERRA_ZENOH_LISTEN or TERRA_ZENOH_CONNECT"
        );
        assert_eq!(
            err(BridgeConfig {
                connect: vec!["udp/127.0.0.1:7447".into()],
                ..default()
            }),
            "Zenoh endpoints must be tcp/ or tls/ locators"
        );
        assert!(
            BridgeConfig {
                config_file: Some("/tmp/ignored-because-validate-skips-specs.json5".into()),
                listen: "not-a-locator".into(),
                ..default()
            }
            .validate()
            .is_ok()
        );
    }

    #[test]
    fn next_load_and_dump_reach_the_game_as_typed_commands() {
        use crate::next_competition::{NextCommand, NextCommandKind};
        let shared = Shared::default();
        shared.commands.lock().unwrap().insert(0, None);
        shared.receive("terra/rover", "terra/rover/0/next/load", b"{}");
        shared.receive("terra/rover", "terra/rover/0/next/dump", b"{}");
        // Unknown rovers and other NEXT-looking keys are ignored.
        shared.receive("terra/rover", "terra/rover/9/next/dump", b"{}");
        shared.receive("terra/rover", "terra/rover/0/next/launch", b"{}");
        assert_eq!(
            *shared.next_commands.lock().unwrap(),
            vec![
                NextCommand { rover: 0, kind: NextCommandKind::Load },
                NextCommand { rover: 0, kind: NextCommandKind::Dump },
            ]
        );
        assert!(shared.actions.lock().unwrap().is_empty());
    }

    #[test]
    fn shared_handler_validates_routes_and_disconnect_discards_intents() {
        let shared = Shared::default();
        shared.commands.lock().unwrap().insert(3, None);
        shared.connected.store(true, Ordering::Release);
        shared.receive(
            "terra/rover",
            "terra/rover/3/cmd_vel",
            br#"{"linear":0.4,"angular":0.2}"#,
        );
        assert!(
            shared.commands.lock().unwrap()[&3].is_none(),
            "unbound cmd_vel is no longer accepted"
        );
        shared.receive(
            "terra/rover",
            "terra/rover/99/cmd_vel",
            br#"{"linear":1,"angular":0}"#,
        );
        assert!(!shared.commands.lock().unwrap().contains_key(&99));
        shared.receive(
            "terra/rover",
            "terra/rover/3/safety",
            br#"{"action":"stop","token":"stop-1"}"#,
        );
        shared.receive(
            "terra/rover",
            "terra/rover/3/goal",
            br#"{"frame":"local","x":2,"y":0}"#,
        );
        shared.receive("terra/rover", "terra/rover/fleet/size", br#"{"count":2}"#);
        assert!(shared.goals.lock().unwrap().is_empty());
        assert_eq!(shared.actions.lock().unwrap()[&3].len(), 2);
        assert_eq!(*shared.fleet_request.lock().unwrap(), Some(2));
        shared.disconnect();
        assert!(!shared.connected.load(Ordering::Acquire));
        assert!(shared.commands.lock().unwrap()[&3].is_none());
        assert!(shared.goals.lock().unwrap().is_empty());
        assert!(shared.actions.lock().unwrap().is_empty());
        assert!(shared.fleet_request.lock().unwrap().is_none());
    }

    #[test]
    fn latest_occupancy_payload_and_baseline_replace_together() {
        let mut outgoing = Outgoing::default();
        let grid = terra_mapping::MapSnapshot {
            width: 1,
            height: 1,
            resolution: 0.1,
            origin_x: 0.0,
            origin_y: 0.0,
            occupancy: vec![100],
        };
        outgoing.queue_occupancy(1, "old", 7, &grid);
        outgoing.occupancy_heartbeats.insert(1);
        outgoing.queue_occupancy(1, "new", 8, &grid);
        assert!(!outgoing.occupancy_heartbeats.contains(&1));
        let drained = std::mem::take(&mut outgoing);
        assert!(outgoing.occupancy.is_empty());
        assert_eq!(drained.occupancy.len(), 1);
        let publication = &drained.occupancy[&1];
        let payload: serde_json::Value = serde_json::from_slice(&publication.payload).unwrap();
        assert_eq!(payload["run_id"], publication.baseline.run_id);
        assert_eq!(payload["sequence"], publication.baseline.sequence);
        assert_eq!(publication.baseline.run_id, "new");
        assert_eq!(publication.baseline.sequence, 8);
    }
    #[test]
    fn accepts_finite_twist_and_rejects_invalid_or_oversized_messages() {
        let command = decode_command(br#"{"linear":1.0,"angular":-0.5}"#).unwrap();
        assert_eq!(command.linear, 1.0);
        assert_eq!(command.angular, -0.5);
        for bytes in [
            b"bad".as_slice(),
            br#"{"linear":1e100,"angular":0}"#,
            br#"{"linear":1}"#,
        ] {
            assert!(decode_command(bytes).is_none());
        }
        assert!(decode_command(&vec![b' '; 2049]).is_none());
    }
    #[test]
    fn timeout_and_missing_commands_stop_the_rover() {
        use std::time::{Duration, Instant};
        let now = Instant::now();
        let timeout = Duration::from_millis(500);
        let command = DriveCommand {
            linear: 1.0,
            angular: 0.2,
        };
        assert_eq!(
            fresh_command(Some((command, now)), now, timeout).linear,
            1.0
        );
        assert_eq!(
            fresh_command(Some((command, now)), now + timeout, timeout).linear,
            0.0
        );
        assert_eq!(fresh_command(None, now, timeout).angular, 0.0);
    }
    #[test]
    fn depth_wire_format_preserves_metres_nan_and_metadata() {
        let frame = crate::depth_camera::DepthFrame {
            width: 2,
            height: 1,
            sequence: 7,
            received_at: 1.25,
            depth_metres: vec![2.5, f32::NAN],
            exposure: None,
        };
        let packet = encode_depth(&frame).unwrap();
        let split = packet.iter().position(|b| *b == b'\n').unwrap();
        let header: serde_json::Value = serde_json::from_slice(&packet[..split]).unwrap();
        assert_eq!(header["encoding"], "32FC1_LE");
        assert_eq!(header["sequence"], 7);
        assert_eq!(header["width"], 2);
        assert_eq!(header["received_at"], 1.25);
        assert!(header.get("camera").is_none());
        assert!(header.get("body").is_none());
        assert!(header.get("exposure_time").is_none());
        let data = &packet[split + 1..];
        assert_eq!(data.len(), 8);
        assert_eq!(f32::from_le_bytes(data[..4].try_into().unwrap()), 2.5);
        assert!(f32::from_le_bytes(data[4..].try_into().unwrap()).is_nan());
    }
    #[test]
    fn depth_packet_pose_round_trips_into_the_phone_decoder() {
        use crate::depth_camera::{DepthCameraConfig, DepthExposure, DepthFrame};
        use crate::occupancy_map::{body_pose, camera_pose};
        use terra_mapping::{CameraIntrinsics, CameraPose, LocalOccupancyMap, MapConfig};
        let camera_transform = GlobalTransform::from(Transform::from_xyz(-0.25, 0.5, -0.25));
        let body_transform = GlobalTransform::from(Transform::from_xyz(0.0, 0.2, -1.0));
        let frame = DepthFrame {
            width: 1,
            height: 1,
            sequence: 4,
            received_at: 3.0,
            depth_metres: vec![2.0],
            exposure: Some(DepthExposure {
                sensor: Entity::PLACEHOLDER,
                timestamp: 1.5,
                camera_transform,
                body_transform,
            }),
        };
        let config = DepthCameraConfig {
            resolution: UVec2::ONE,
            vertical_fov: 60.0_f32.to_radians(),
            ..default()
        };
        let packet = encode_depth_with_config(0, &frame, &config).unwrap();
        let decoded = terra_transport::decode_depth_frame(&packet).expect("posed depth");
        let pose = camera_pose(&camera_transform);
        let (body_x, body_y, body_yaw) = body_pose(&body_transform);
        assert_eq!(decoded.sequence, 4);
        assert!((decoded.timestamp - 1.5).abs() < 1e-9);
        assert!((decoded.camera_x - pose.position.x).abs() < 1e-5);
        assert!((decoded.camera_y - pose.position.y).abs() < 1e-5);
        assert!((decoded.camera_z - pose.position.z).abs() < 1e-5);
        assert!((decoded.quaternion_x - pose.orientation.x).abs() < 1e-5);
        assert!((decoded.quaternion_y - pose.orientation.y).abs() < 1e-5);
        assert!((decoded.quaternion_z - pose.orientation.z).abs() < 1e-5);
        assert!((decoded.quaternion_w - pose.orientation.w).abs() < 1e-5);
        assert!((decoded.body_x - body_x).abs() < 1e-5);
        assert!((decoded.body_y - body_y).abs() < 1e-5);
        assert!((decoded.body_yaw - body_yaw).abs() < 1e-5);
        assert!((decoded.cx).abs() < 1e-9 && decoded.cy.abs() < 1e-9);
        let mut map = LocalOccupancyMap::new(MapConfig {
            pixel_stride: 1,
            ..MapConfig::default()
        })
        .unwrap();
        map.recenter(decoded.body_x, decoded.body_y).unwrap();
        map.integrate_depth(
            decoded.timestamp,
            CameraIntrinsics {
                width: decoded.width,
                height: decoded.height,
                fx: decoded.fx,
                fy: decoded.fy,
                cx: decoded.cx,
                cy: decoded.cy,
            },
            CameraPose {
                position: terra_types::Vector3 {
                    x: decoded.camera_x,
                    y: decoded.camera_y,
                    z: decoded.camera_z,
                },
                orientation: terra_types::Quaternion {
                    x: decoded.quaternion_x,
                    y: decoded.quaternion_y,
                    z: decoded.quaternion_z,
                    w: decoded.quaternion_w,
                },
            },
            &decoded.depth_metres,
        )
        .unwrap();
        let grid = map.snapshot();
        let col = ((2.25 - grid.origin_x) / grid.resolution).floor() as usize;
        let row = ((0.25 - grid.origin_y) / grid.resolution).floor() as usize;
        assert!(
            grid.occupancy[row * grid.width as usize + col] > 50,
            "endpoint (2.25, 0.25) must be occupied"
        );
        assert!(
            terra_transport::decode_depth_frame(
                &encode_depth(&DepthFrame {
                    exposure: None,
                    ..frame
                })
                .unwrap()
            )
            .is_none()
        );
    }
    #[test]
    #[ignore = "requires local TCP sockets; tests two real Zenoh sessions"]
    fn zenoh_round_trip_frames_drive_and_deadman_stop() {
        let socket = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("tcp/{}", socket.local_addr().unwrap());
        drop(socket);
        let config = BridgeConfig {
            listen: endpoint.clone(),
            command_timeout: Duration::from_millis(100),
            ..default()
        };
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(SimulatorBridgePlugin { config })
            .add_systems(PreUpdate,mount_replay_runtimes);
        let rover = app
            .world_mut()
            .spawn((
                Rover,
                RoverId(0),
                KeyboardControlled,
                DriveCommand::default(),
            ))
            .id();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !app
            .world()
            .resource::<Bridge>()
            .shared
            .connected
            .load(Ordering::Acquire)
        {
            assert!(Instant::now() < deadline, "bridge failed to open");
            std::thread::sleep(Duration::from_millis(5));
        }
        let mut client_config = zenoh::Config::default();
        client_config.insert_json5("mode", "\"client\"").unwrap();
        client_config
            .insert_json5(
                "connect/endpoints",
                &serde_json::json!([endpoint.clone()]).to_string(),
            )
            .unwrap();
        client_config
            .insert_json5("scouting/multicast/enabled", "false")
            .unwrap();
        let client = zenoh::open(client_config).wait().unwrap();
        let subscriber = client
            .declare_subscriber("terra/rover/0/camera/*")
            .wait()
            .unwrap();
        let camera_config = DepthCameraConfig {
            resolution: UVec2::new(2, 1),
            ..default()
        };
        let depth_entity = app
            .world_mut()
            .spawn((
                RoverId(0),
                DepthCamera {
                    config: camera_config.clone(),
                    depth_texture: default(),
                    preview_texture: default(),
                },
                DepthFrame {
                    width: 2,
                    height: 1,
                    sequence: 1,
                    received_at: 1.0,
                    depth_metres: vec![2.0, f32::NAN],
                    exposure: None,
                },
            ))
            .id();
        let rgb_entity = app
            .world_mut()
            .spawn((
                RoverId(0),
                crate::rgb_camera::RgbCamera {
                    config: camera_config,
                },
                RgbFrame {
                    width: 2,
                    height: 1,
                    sequence: 1,
                    received_at: 1.1,
                    rgba: vec![10, 20, 30, 255, 40, 50, 60, 255],
                    ..Default::default()
                },
            ))
            .id();
        let mut python = std::env::var("TERRA_ZENOH_TEST_PYTHON").ok().map(|python| {
            std::process::Command::new(python)
                .arg(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/tools/test_zenoh_client.py"
                ))
                .env("TERRA_TEST_ZENOH_ENDPOINT", endpoint.clone())
                .spawn()
                .expect("could not start Python interoperability test")
        });
        let mut received_rgb = false;
        let mut received_depth = false;
        let mut moved = false;
        let mut sequence=0;
        while Instant::now() < deadline && !(received_rgb && received_depth && moved) {
            sequence+=1;
            if python.is_none() {
                publish_bound_drive(&app,&client,0,1.0,0.25,sequence);
            }
            // Force new frames to cover discovery without retaining old samples.
            app.world_mut()
                .get_mut::<DepthFrame>(depth_entity)
                .unwrap()
                .sequence += 1;
            app.world_mut()
                .get_mut::<RgbFrame>(rgb_entity)
                .unwrap()
                .sequence += 1;
            app.update();
            moved |= app.world().get::<DriveCommand>(rover).unwrap().linear == 1.0;
            while let Ok(Some(sample)) = subscriber.try_recv() {
                let payload = sample.payload().to_bytes();
                let split = payload.iter().position(|b| *b == b'\n').unwrap();
                let header: serde_json::Value = serde_json::from_slice(&payload[..split]).unwrap();
                let pixels = &payload[split + 1..];
                assert_eq!(pixels.len(), 8);
                if header["encoding"] == "32FC1_LE" {
                    assert_eq!(f32::from_le_bytes(pixels[..4].try_into().unwrap()), 2.0);
                    assert!(f32::from_le_bytes(pixels[4..].try_into().unwrap()).is_nan());
                    received_depth = true;
                } else {
                    assert_eq!(header["encoding"], "RGBA8_SRGB");
                    assert_eq!(pixels, [10, 20, 30, 255, 40, 50, 60, 255]);
                    received_rgb = true;
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            received_rgb && received_depth && moved,
            "RGB={received_rgb}, depth={received_depth}, drove={moved}, output={:?}, authority={:?}", app.world().get::<crate::velocity_controller::VelocityControlState>(rover).map(|c| c.runtime.autonomy_json.clone()), app.world().resource::<Bridge>().shared.authorities.lock().unwrap()
        );
        if let Some(python) = python.as_mut() {
            assert!(
                python.wait().unwrap().success(),
                "Python interoperability failed"
            );
        }
        assert!(app.world().get::<KeyboardControlled>(rover).is_none());
        for _ in 0..65 {std::thread::sleep(Duration::from_millis(10));app.update();}
        assert_eq!(app.world().get::<DriveCommand>(rover).unwrap().linear, 0.0);
        assert_eq!(app.world().get::<DriveCommand>(rover).unwrap().angular, 0.0);
        client.close().wait().unwrap();
        let shared = app.world().resource::<Bridge>().shared.clone();
        drop(app);
        while shared.connected.load(Ordering::Acquire) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            !shared.connected.load(Ordering::Acquire),
            "worker did not shut down"
        );
    }
    #[test]
    fn drive_commands_are_isolated_between_rovers() {
        let mut app = App::new();
        let shared = Arc::new(Shared::default());
        shared.connected.store(true, Ordering::Release);
        shared.commands.lock().unwrap().insert(
            0,
            Some((
                DriveCommand {
                    linear: 1.0,
                    angular: 0.0,
                },
                Instant::now(),
            )),
        );
        app.insert_resource(Bridge { shared })
            .insert_resource(BridgeConfig::default())
            .add_systems(FixedUpdate, apply_remote_commands);
        let first = app
            .world_mut()
            .spawn((
                Rover,
                crate::terra::RoverId(0),
                ZenohControlled,
                DriveCommand::default(),
            ))
            .id();
        let second = app
            .world_mut()
            .spawn((
                Rover,
                crate::terra::RoverId(1),
                ZenohControlled,
                DriveCommand::default(),
            ))
            .id();
        app.world_mut().run_schedule(FixedUpdate);
        assert_eq!(app.world().get::<DriveCommand>(first).unwrap().linear, 1.0);
        assert_eq!(app.world().get::<DriveCommand>(second).unwrap().linear, 0.0);
    }
    #[test]
    fn waypoint_goal_drives_north_and_publishes_status() {
        let shared = Arc::new(Shared::default());
        shared.connected.store(true, Ordering::Release);
        shared.commands.lock().unwrap().insert(0, None);
        shared.goals.lock().unwrap().insert(
            0,
            GoalInbox {
                seq: 1,
                command: GoalCommand::Local {
                    x: 10.0,
                    y: 0.0,
                    yaw: None,
                    token: Some("goal-1".into()),
                },
            },
        );
        let mut app = App::new();
        app.insert_resource(Bridge {
            shared: shared.clone(),
        })
        .insert_resource(BridgeConfig::default())
        .add_systems(FixedUpdate, drive_goals);
        app.world_mut().spawn((
            Rover,
            RoverId(0),
            ZenohControlled,
            Transform::default(),
            DriveCommand::default(),
        ));
        app.world_mut().run_schedule(FixedUpdate);
        let world = app.world_mut();
        let command = world
            .query_filtered::<&DriveCommand, With<Rover>>()
            .single(world)
            .unwrap();
        assert!(command.linear > 0.5, "{}", command.linear);
        assert!(command.angular.abs() < 0.05, "{}", command.angular);
        let payload = shared
            .outgoing
            .lock()
            .unwrap()
            .goals
            .get(&0)
            .cloned()
            .unwrap();
        let status: serde_json::Value = serde_json::from_slice(&payload).unwrap();
        assert_eq!(status["state"], "active");
        assert_eq!(status["goal_id"], 1);
        assert_eq!(status["token"], "goal-1");
        assert!(status["distance"].as_f64().unwrap() > 9.0);
    }
    #[test]
    fn fleet_requests_validate_count_and_apply_to_the_resource() {
        assert_eq!(decode_fleet_request(br#"{"count":3}"#), Some(3));
        assert_eq!(decode_fleet_request(br#"{"count":0}"#), Some(0));
        for input in [
            br#"{"count":33}"#.as_slice(),
            br#"{"count":-1}"#,
            br#"{"count":1.5}"#,
            b"invalid",
        ] {
            assert!(decode_fleet_request(input).is_none());
        }
        let shared = Arc::new(Shared::default());
        *shared.fleet_request.lock().unwrap() = Some(3);
        let mut app = App::new();
        app.insert_resource(Bridge { shared })
            .init_resource::<RoverFleet>()
            .add_systems(Update, receive_fleet_request);
        app.update();
        assert_eq!(app.world().resource::<RoverFleet>().count(), 3);
    }
    #[test]
    #[ignore = "requires local TCP sockets; verifies live fleet resizing"]
    fn zenoh_resizes_fleet_and_keeps_drive_commands_independent() {
        let socket = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("tcp/{}", socket.local_addr().unwrap());
        drop(socket);
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(crate::terra::DriveConfig::default())
            .insert_resource(crate::physics::RoverPhysicsConfig::default())
            .init_resource::<RoverFleet>()
            .add_systems(PreUpdate,mount_replay_runtimes.after(crate::terra::reconcile_fleet))
            .add_systems(PreUpdate, crate::terra::reconcile_fleet)
            .add_plugins(SimulatorBridgePlugin {
                config: BridgeConfig {
                    listen: endpoint.clone(),
                    ..default()
                },
            });
        let deadline = Instant::now() + Duration::from_secs(10);
        while !app
            .world()
            .resource::<Bridge>()
            .shared
            .connected
            .load(Ordering::Acquire)
        {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        let mut config = zenoh::Config::default();
        config.insert_json5("mode", "\"client\"").unwrap();
        config
            .insert_json5(
                "connect/endpoints",
                &serde_json::json!([endpoint]).to_string(),
            )
            .unwrap();
        config
            .insert_json5("scouting/multicast/enabled", "false")
            .unwrap();
        let client = zenoh::open(config).wait().unwrap();
        let states = client
            .declare_subscriber("terra/rover/fleet/state")
            .wait()
            .unwrap();
        for target in [3, 1, 2, 0] {
            let mut acknowledged = false;
            while Instant::now() < deadline {
                client
                    .put(
                        "terra/rover/fleet/size",
                        serde_json::json!({"count": target}).to_string(),
                    )
                    .wait()
                    .unwrap();
                app.update();
                    while let Ok(Some(state)) = states.try_recv() {
                    let state: serde_json::Value =
                        serde_json::from_slice(&state.payload().to_bytes()).unwrap();
                    acknowledged |= state["count"] == target;
                }
                let world = app.world_mut();
                let count = world
                    .query_filtered::<&RoverId, With<Rover>>()
                    .iter(world)
                    .count();
                if count == target && acknowledged {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            let world = app.world_mut();
            let mut ids: Vec<_> = world
                .query_filtered::<&RoverId, With<Rover>>()
                .iter(world)
                .map(|id| id.0)
                .collect();
            ids.sort();
            assert_eq!(ids.len(), target);
            assert!(acknowledged);
            if target == 2 {
                assert_eq!(ids, [0, 3]);
            }
            if target == 3 {
                let mut independent = false;
                let mut sequence=0;
                while Instant::now() < deadline {
                    sequence+=1;
                    publish_bound_drive(&app,&client,0,1.0,0.0,sequence);
                    publish_bound_drive(&app,&client,1,-1.0,0.5,sequence);
                    app.update();
                            let world = app.world_mut();
                    let mut requests: Vec<_> = world
                        .query_filtered::<(&RoverId, &DriveCommand), With<Rover>>()
                        .iter(world)
                        .map(|(id, command)| (id.0, command.linear))
                        .collect();
                    requests.sort_by_key(|(id, _)| *id);
                    independent = requests == [(0, 1.0), (1, -0.15), (2, 0.0)];
                    if independent {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
                assert!(independent,"statuses: {:?}", app.world_mut().query::<&crate::velocity_controller::VelocityControlState>().iter(app.world()).map(|c|c.runtime.autonomy_json.clone()).collect::<Vec<_>>());
            }
        }
        client.close().wait().unwrap();
    }
    #[test]
    #[ignore = "requires local TCP sockets; mobile adapter drives real Avian physics"]
    fn mobile_adapter_drives_avian_and_disconnect_stops() {
        use avian3d::prelude::*;
        let socket = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("tcp/{}", socket.local_addr().unwrap());
        drop(socket);
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            TransformPlugin,
            bevy::asset::AssetPlugin::default(),
            bevy::mesh::MeshPlugin,
            crate::physics::TerraPhysicsPlugin,
            crate::velocity_controller::TerraVelocityControlPlugin,
            SimulatorBridgePlugin {
                config: BridgeConfig {
                    listen: endpoint.clone(),
                    ..default()
                },
            },
        ))
        .insert_resource(crate::terra::DriveConfig::default())
        .insert_resource(Time::<Fixed>::from_hz(100.0))
        .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            Duration::from_millis(10),
        ));
        app.finish();
        app.cleanup();
        app.world_mut().spawn((
            RigidBody::Static,
            Collider::cuboid(100.0, 0.2, 100.0),
            Transform::from_xyz(0.0, -0.1, 0.0),
        ));
        let rover = app
            .world_mut()
            .spawn((
                Rover,
                RoverId(9),
                crate::physics::RoverBody::default(),
                Transform::from_xyz(0.0, 0.1, 0.0),
                DriveCommand::default(),
                crate::terra::WheelSpeeds::default(),
                crate::terra::Odometry::default(),
            ))
            .id();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !app
            .world()
            .resource::<Bridge>()
            .shared
            .connected
            .load(Ordering::Acquire)
        {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
        app.update();
        let client =
            terra_transport::RoverConnection::connect(&endpoint, "terra/rover", 9).unwrap();
        for _ in 0..700 {
            client.set_target(1.0, 0.2).unwrap();
            app.update();
            std::thread::sleep(Duration::from_millis(1));
        }
        let command = app.world().get::<DriveCommand>(rover).unwrap();
        assert_eq!(command.linear, 1.0);
        assert_eq!(command.angular, 0.2);
        assert!(
            app.world()
                .get::<Position>(rover)
                .unwrap()
                .0
                .with_y(0.0)
                .length()
                > 1.0
        );
        assert!(app.world().get::<LinearVelocity>(rover).unwrap().length() > 0.5);
        client.disconnect();
        for _ in 0..600 {
            app.update();
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(app.world().get::<DriveCommand>(rover).unwrap().linear, 0.0);
        assert!(app.world().get::<LinearVelocity>(rover).unwrap().length() < 0.05);
        assert!(app.world().get::<AngularVelocity>(rover).unwrap().y.abs() < 0.05);
        let shared = app.world().resource::<Bridge>().shared.clone();
        drop(app);
        while shared.connected.load(Ordering::Acquire) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(!shared.connected.load(Ordering::Acquire));
    }
}

struct AutonomousRover {
    /// Starts armed, matching the outdoor simulator. An explicit hardware disarm clears it.
    armed: bool,
    near_miss: terra_experiment::NearMiss,
    contacts: std::collections::BTreeSet<Entity>,
    observed_area: terra_experiment::ObservedArea,
}
#[derive(Component, Default)]
pub(crate) struct AutonomyMotorGate {
    pub managed: bool,
    pub hold: bool,
    pub reset: bool,
}
struct AutonomyRuntime {
    rovers: BTreeMap<u64, AutonomousRover>,
    occupancy_gates: BTreeMap<u64, publish_gate::Gate<terra_mapping::MapSnapshot>>,
    was_connected: bool,
    epoch: u64,
    connection_generation: u64,
    run_id: String,
    recorder: Option<recording::Recorder>,
    mission: terra_experiment::Mission,
    last_publish: f64,
    sequence: u64,
}
impl Default for AutonomyRuntime {
    fn default() -> Self {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let run_id = format!("run-{stamp}");
        crate::observation_capture::set_session(&run_id);
        let config = terra_experiment::MissionConfig::rescue(
            std::env::var("TERRA_MISSION_SEED")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(42),
        );
        let manifest = serde_json::json!({"run_id":run_id,"platform":"bevy","runtime_source_revision":terra_runtime::SOURCE_REVISION,"mission":config,"trial_design":std::env::var("TERRA_TRIAL_DESIGN").unwrap_or_else(|_|"adaptive".into()),"planner":terra_navigation::PlannerConfig::default(),"collision_observation":"simulator_contacts","target_detector":{"id":"sim-survivor-v1","classes":["survivor"],"range_m":5,"half_fov_radians":std::f64::consts::FRAC_PI_3,"confirmation":terra_autonomy::ConfirmationConfig::default()},"git_revision":option_env!("TERRA_GIT_REVISION").unwrap_or("unknown")});
        let recorder = recording::Recorder::native(manifest.clone());
        Self {
            rovers: BTreeMap::new(),
            occupancy_gates: BTreeMap::new(),
            was_connected: false,
            epoch: 0,
            connection_generation: 0,
            run_id,
            recorder,
            mission: terra_experiment::Mission::new(config).unwrap(),
            last_publish: -1.,
            sequence: 0,
        }
    }
}
/// Hardware arm/disarm from the outdoor adapter. Disarm stops on this tick.
/// Arm requires a fresh unreplayed token, the current run, and navigation authority.
/// Source changes, resume, and goals stay on the shared runtime.
fn apply_hardware_command(
    rt: &mut terra_runtime::Runtime,
    bytes: &[u8],
    age: f64,
    armed: &mut bool,
) {
    let reject = |rt: &mut terra_runtime::Runtime, reason: &str| {
        rt.outdoor.result = Some(format!("rejected: {reason}"));
    };
    let Ok(v) = serde_json::from_slice::<serde_json::Value>(bytes) else {
        reject(rt, "invalid_json");
        return;
    };
    if v["action"] == "disarm" {
        *armed = false;
        rt.outdoor.pause("disarmed");
        rt.autonomy.clear_guidance();
        rt.outdoor.result = Some("accepted".into());
        return;
    }
    if !age.is_finite()
        || !(0.0..terra_types::timing::CONTROL_REQUEST_FRESHNESS_SECONDS).contains(&age)
    {
        reject(rt, "request_expired");
        return;
    }
    let Some(token) = v["token"].as_str() else {
        reject(rt, "missing_token");
        return;
    };
    if !terra_autonomy::valid_token(token) || rt.outdoor_tokens.iter().any(|t| t == token) {
        reject(rt, "invalid_or_replayed_token");
        return;
    }
    if rt.outdoor_tokens.len() == 128 {
        rt.outdoor_tokens.pop_front();
    }
    rt.outdoor_tokens.push_back(token.to_owned());
    rt.outdoor.token = Some(token.to_owned());
    if v["run_id"] != rt.autonomy.run_id {
        reject(rt, "run_changed");
        return;
    }
    if v["action"] != "arm" {
        reject(rt, "arm_rejected");
        return;
    }
    let Some(revision) = v["authority_revision"].as_u64() else {
        reject(rt, "revision_missing");
        return;
    };
    let pose = rt.pose.unwrap_or_default().0;
    let run_id = rt.autonomy.run_id.clone();
    if !rt
        .autonomy
        .navigation_configuration_allowed(&run_id, revision, pose)
    {
        reject(rt, "arm_rejected");
        return;
    }
    *armed = true;
    rt.outdoor.result = Some("accepted".into());
}
#[allow(clippy::type_complexity)]
fn drive_autonomy(
    bridge: Res<Bridge>,
    time: Res<Time<Fixed>>,
    world: Option<Res<crate::world::WorldConfig>>,
    keys: Option<Res<ButtonInput<KeyCode>>>,
    mut entities: Commands,
    contacts: Option<avian3d::prelude::Collisions>,
    grounds: Query<(), With<crate::world::Ground>>,
    mut rovers: Query<
        (
            Entity,
            &RoverId,
            &Transform,
            &mut DriveCommand,
            Option<&crate::occupancy_map::RoverOccupancyMap>,
            Option<&avian3d::prelude::LinearVelocity>,
            Option<&avian3d::prelude::AngularVelocity>,
            Option<&mut AutonomyMotorGate>,
            Option<&mut crate::velocity_controller::VelocityControlState>,
        ),
        (With<Rover>, With<ZenohControlled>),
    >,
    mut runtime: Local<AutonomyRuntime>,
) {
    use terra_autonomy::*;

    let now = time.elapsed_secs_f64();
    let generation = bridge.shared.connection_generation.load(Ordering::Acquire);
    let replaced = runtime.connection_generation != generation;
    let link_changed =
        runtime.was_connected != bridge.shared.connected.load(Ordering::Acquire) || replaced;
    runtime.connection_generation = generation;
    let connected = bridge.shared.connected.load(Ordering::Acquire);
    let epoch = bridge.shared.epoch.load(Ordering::Relaxed);
    if connected != runtime.was_connected || runtime.epoch != epoch {
        runtime.occupancy_gates.clear();
        runtime.was_connected = connected;
        runtime.epoch = epoch;
    }
    let mut actions = std::mem::take(&mut *bridge.shared.actions.lock().unwrap());
    let space = keys.is_some_and(|k| k.pressed(KeyCode::Space));
    let due = now - runtime.last_publish >= 0.2;
    if due {
        runtime.last_publish = now;
    }
    let mut seen = std::collections::BTreeSet::new();
    for (entity, id, _transform, mut command, map, _velocity, _angular, gate, control) in
        &mut rovers
    {
        let Some(mut control) = control else { continue };
        seen.insert(id.0);
        let output_active = control.output.left != 0.0 || control.output.right != 0.0;
        let rt = &mut control.runtime;
        if rt.run_id != runtime.run_id {
            rt.run_id = runtime.run_id.clone();
            rt.autonomy.run_id = runtime.run_id.clone();
            rt.rover_id = Some(id.0);
        }
        if replaced {
            rt.link(false);
        }
        if link_changed {
            let _ = rt.ingest(terra_runtime::RuntimeEvent::Link {
                connected,
                received_at: now,
            });
        }
        rt.recording_healthy = runtime.recorder.as_ref().is_none_or(|r| !r.failed());
        let mut tracked = runtime
            .rovers
            .remove(&id.0)
            .unwrap_or_else(|| AutonomousRover {
                armed: true,
                near_miss: Default::default(),
                contacts: Default::default(),
                observed_area: Default::default(),
            });
        let pose = rt.pose.unwrap_or_default().0;
        let (x, y, _) = (pose.x, pose.y, pose.yaw);
        let snapshot = map.map(|m| m.map.snapshot());
        let map_time = map.and_then(|m| m.last_exposure);
        if let (Some(snapshot), Some(timestamp)) = (snapshot.as_ref(), map_time) {
            let _ = rt.ingest(terra_runtime::RuntimeEvent::Map {
                snapshot: snapshot.clone(),
                timestamp,
                sequence: map.map_or(0, |m| m.last_sequence),
                received_at: now,
            });
        }
        // The pitch is placed on Field 3 (West Campus), so outdoor GPS
        // navigation works on it.
        let geo_reference = world
            .as_ref()
            .and(crate::mason_field::geographic_reference());
        rt.outdoor.supported = geo_reference.is_some();
        if let Some(reference) = geo_reference {
            rt.autonomy
                .set_origin(reference.latitude, reference.longitude);
            let (latitude, longitude, true_heading) = reference.sample(pose);
            let _ = rt.ingest(terra_runtime::RuntimeEvent::Gps {
                sample: terra_autonomy::outdoor::GpsFix {
                    timestamp: now,
                    latitude,
                    longitude,
                    accuracy: 1.0,
                },
                received_at: now,
            });
            let _ = rt.ingest(terra_runtime::RuntimeEvent::Heading {
                sample: terra_autonomy::outdoor::HeadingFix {
                    timestamp: now,
                    true_heading,
                    accuracy: 1.0,
                },
                received_at: now,
            });
        } else {
            rt.outdoor.invalidate_sensors();
        }
        if let Some(mut queue) = actions.remove(&id.0) {
            for (kind, payload, received) in
                crate::control_queue::drain_control_actions(&mut queue)
            {
                if kind == "hardware" {
                    apply_hardware_command(
                        rt,
                        &payload,
                        received.elapsed().as_secs_f64(),
                        &mut tracked.armed,
                    );
                    continue;
                }
                if kind == "mission/report" {
                    if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&payload) {
                        if let Some(target) = v["survivor_id"].as_u64() {
                            runtime.mission.report(id.0, target, now);
                        }
                    }
                    continue;
                }
                let _ = rt.ingest(terra_runtime::RuntimeEvent::Remote {
                    kind,
                    payload,
                    received_at: (now - received.elapsed().as_secs_f64()).max(0.0),
                });
            }
        }
        let mut hardware = crate::control_queue::simulated_relay_status(
            &runtime.run_id,
            tracked.armed,
            output_active,
        );
        hardware["armed"] = serde_json::json!(tracked.armed);
        let _ = rt.ingest(terra_runtime::RuntimeEvent::Hardware {
            status: hardware,
            received_at: now,
        });
        if space {
            rt.autonomy.hold_emergency_stop();
        }
        let Ok(runtime_output) = rt.step(now) else {
            continue;
        };
        let runtime_status = runtime_output.snapshot.value.clone();
        let mut output = runtime_output.autonomy;
        let mut motor = runtime_output.motor;
        let outdoor_hold = rt.outdoor.selected && rt.outdoor.phase() != "planning";
        if !tracked.armed || outdoor_hold {
            output.twist.linear = 0.0;
            output.twist.angular = 0.0;
            output.reset_controller = true;
            rt.controller.reset();
            motor.left = 0.0;
            motor.right = 0.0;
            motor.target_forward = 0.0;
            motor.target_yaw_rate = 0.0;
            motor.stop_reason = terra_types::StopReason::None;
        }
        for event in runtime_output.events {
            let event = serde_json::json!({"kind":"runtime.event","event":event,"run_id":runtime.run_id,"rover_id":id.0,"time":now});
            crate::observability::record_tick(event.clone());
            if let Some(recorder) = runtime.recorder.as_ref() {
                let _ = recorder.record(event);
            }
        }
        bridge
            .shared
            .authorities
            .lock()
            .unwrap()
            .insert(id.0, (output.status.run_id.clone(), output.status.revision));
        *command = DriveCommand {
            linear: output.twist.linear as f32,
            angular: output.twist.angular as f32,
        };
        let hold = output.status.safety != "clear" || !tracked.armed || outdoor_hold;
        let managed = output.command_origin != terra_autonomy::CommandOrigin::LocalManual;
        if let Some(mut gate) = gate {
            gate.managed = managed;
            gate.hold = hold;
            gate.reset = output.reset_controller;
        } else {
            entities.entity(entity).insert(AutonomyMotorGate {
                managed,
                hold,
                reset: output.reset_controller,
            });
        }
        // Observations require a complete ray through observed free occupancy. Ground truth is never published directly.
        if let Some(m) = snapshot.as_ref() {
            let candidates = runtime.mission.candidates().to_vec();
            for target in candidates {
                let distance = (target.x - x).hypot(target.y - y);
                let n = (distance / m.resolution).ceil() as usize;
                let visible = n > 0
                    && distance <= 5.
                    && (0..=n).all(|i| {
                        let t = i as f64 / n as f64;
                        let a = ((x + (target.x - x) * t - m.origin_x) / m.resolution).floor();
                        let b = ((y + (target.y - y) * t - m.origin_y) / m.resolution).floor();
                        a >= 0. && b >= 0. && a < m.width as f64 && b < m.height as f64 && {
                            let value = m.occupancy[b as usize * m.width as usize + a as usize];
                            (0..65).contains(&value)
                        }
                    });
                if visible {
                    runtime.mission.observe_target(id.0, target.id, x, y, now);
                }
            }
        }
        let observed_area = snapshot
            .as_ref()
            .and_then(|m| tracked.observed_area.observe(m));
        let clearance = snapshot.as_ref().and_then(|m| {
            terra_navigation::observed_clearance(m, pose, rt.autonomy.planner.config.radius)
        });
        if let Some(kind) = tracked.near_miss.update(clearance) {
            output.events.push(DecisionEvent {
                kind: kind.into(),
                token: None,
                reason: "observed_clearance".into(),
                level: output.status.requested_level,
            });
        }
        let touching = contacts
            .as_ref()
            .map(|c| {
                c.entities_colliding_with(entity)
                    .filter(|e| grounds.get(*e).is_err())
                    .collect::<std::collections::BTreeSet<_>>()
            })
            .unwrap_or_default();
        for target in touching.difference(&tracked.contacts) {
            output.events.push(DecisionEvent {
                kind: "contact_start".into(),
                token: None,
                reason: format!("simulator_contact:{target:?}"),
                level: output.status.requested_level,
            });
        }
        tracked.contacts = touching;
        runtime.mission.update_pose(id.0, x, y, now);
        runtime.sequence += 1;
        let event = serde_json::json!({"kind":"control_tick","run_id":runtime.run_id,"sequence":runtime.sequence,"rover_id":id.0,"time":now,"status":output.status,"selected":output.twist,"source_command":output.intent,"pose":pose,"goal":output.goal,"proposal":output.proposal,"events":output.events,"runtime":runtime_status["runtime"],"mission":runtime.mission.status(now),"search":output.search,"clearance":clearance,"observed_free_area":observed_area});
        crate::observability::record_tick(event.clone());
        if let Some(r) = runtime.recorder.as_ref() {
            let _ = r.record(event);
        }
        if due || output.reset_controller {
            let utc = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs_f64();
            let mut outgoing = bridge.shared.outgoing.lock().unwrap();
            outgoing.reports.insert(
                id.0,
                output
                    .pending_reports
                    .iter()
                    .map(|r| serde_json::to_vec(r).unwrap())
                    .collect(),
            );
            outgoing
                .goals
                .insert(id.0, encode_status(&output.goal).unwrap());
            outgoing.states.insert(
                (id.0, "navigation/outdoor".into()),
                serde_json::to_vec(&runtime_status["outdoor"]).unwrap(),
            );
            if let Some(reference) = geo_reference {
                outgoing.states.insert(
                    (id.0, "localization".into()),
                    serde_json::to_vec(&serde_json::json!({
                        "version": 1,
                        "mode": "geographic",
                        "originLatitude": reference.latitude,
                        "originLongitude": reference.longitude,
                        "anchorX": 0.0,
                        "anchorY": 0.0,
                        "rotation": -reference.heading.to_radians(),
                        "simulated": true
                    }))
                    .unwrap(),
                );
            }
            for (kind, value) in [
                (
                    "hardware/status",
                    {
                        let mut status = crate::control_queue::simulated_relay_status(
                            &runtime.run_id,
                            connected && tracked.armed,
                            !hold
                                && !output.reset_controller
                                && (motor.left != 0.0 || motor.right != 0.0),
                        );
                        status["armed"] = serde_json::json!(tracked.armed);
                        status
                    },
                ),
                ("autonomy/status", {
                    let mut value = serde_json::to_value(&output.status).unwrap();
                    value["navigation"] = runtime_status["navigation"].clone();
                    value["navigation"]["outdoor"] = runtime_status["outdoor"].clone();
                    value["pose_quality"] = runtime_status["pose_quality"].clone();
                    value["connectivity"] = runtime_status["connectivity"].clone();
                    value["runtime"] = runtime_status["runtime"].clone();
                    value
                }),
                (
                    "exploration/status",
                    serde_json::to_value(&output.exploration).unwrap(),
                ),
                (
                    "search/status",
                    serde_json::to_value(&output.search).unwrap(),
                ),
                (
                    "goal/proposal",
                    serde_json::to_value(&output.proposal).unwrap(),
                ),
                (
                    "mission/status",
                    serde_json::to_value(runtime.mission.status(now)).unwrap(),
                ),
                (
                    "experiment/status",
                    serde_json::json!({"run_id":runtime.run_id,"schema_version":1,"run_elapsed":now,"utc":utc,"recording":runtime.recorder.as_ref().is_some_and(|r|!r.failed()),"runtime":runtime_status["runtime"]}),
                ),
            ] {
                outgoing
                    .states
                    .insert((id.0, kind.into()), serde_json::to_vec(&value).unwrap());
            }
        }
        if connected && let Some(grid) = snapshot.as_ref() {
            let run_id = runtime.run_id.clone();
            let sequence = map.map(|m| m.last_sequence).unwrap_or(0);
            let publication = runtime.occupancy_gates.entry(id.0).or_default().observe(
                &run_id,
                grid,
                sequence,
                map_time.is_some_and(|t| now - t < terra_types::timing::MAP_FRESHNESS_SECONDS),
                Duration::from_secs_f64(now),
                |a, b| {
                    a.width == b.width
                        && a.height == b.height
                        && a.resolution == b.resolution
                        && a.origin_x == b.origin_x
                        && a.origin_y == b.origin_y
                        && a.occupancy == b.occupancy
                },
            );
            let mut outgoing = bridge.shared.outgoing.lock().unwrap();
            match publication {
                publish_gate::Publication::Full => {
                    outgoing.queue_occupancy(id.0, &run_id, sequence, grid);
                }
                publish_gate::Publication::Heartbeat => {
                    outgoing.occupancy_heartbeats.insert(id.0);
                }
                publish_gate::Publication::None => {}
            }
        }
        control.output = motor;
        runtime.rovers.insert(id.0, tracked);
    }
    runtime.rovers.retain(|id, _| seen.contains(id));
    runtime.occupancy_gates.retain(|id, _| seen.contains(id));
    bridge
        .shared
        .authorities
        .lock()
        .unwrap()
        .retain(|id, _| seen.contains(id));
}

#[cfg(test)]
mod autonomy_integration_tests {
    use super::*;
    #[test]
    fn settings_observe_the_preceding_motor_output() {
        let shared = Arc::new(Shared::default());
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(Bridge {
                shared: shared.clone(),
            })
            .add_systems(FixedUpdate, drive_autonomy);
        let rt = terra_runtime::Runtime::new(Default::default(), "test".into()).unwrap();
        let mut control = crate::velocity_controller::VelocityControlState::new(rt);
        control.output.left = 0.4;
        let entity = app
            .world_mut()
            .spawn((
                Rover,
                RoverId(1),
                ZenohControlled,
                Transform::default(),
                DriveCommand::default(),
                control,
            ))
            .id();
        app.world_mut()
            .resource_mut::<Time<Fixed>>()
            .advance_by(Duration::from_millis(10));
        app.world_mut().run_schedule(FixedUpdate);
        let control = app
            .world()
            .get::<crate::velocity_controller::VelocityControlState>(entity)
            .unwrap();
        assert_eq!(
            control.runtime.hardware_observation.as_ref().unwrap().0["output_active"],
            true
        );
    }
    #[test]
    fn runtime_selects_teleop_and_latches_stop_per_rover() {
        let shared = Arc::new(Shared::default());
        shared.connected.store(true, Ordering::Release);
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(Bridge {
                shared: shared.clone(),
            })
            .add_systems(FixedUpdate, drive_autonomy);
        for id in [1, 2] {
            shared.commands.lock().unwrap().insert(id, None);
            let rt = terra_runtime::Runtime::new(
                terra_runtime::RuntimeConfig {
                    mode: terra_runtime::ActuationMode::DirectEffort,
                    ..Default::default()
                },
                format!("test-{id}"),
            )
            .unwrap();
            app.world_mut().spawn((
                Rover,
                RoverId(id),
                ZenohControlled,
                Transform::default(),
                DriveCommand::default(),
                crate::velocity_controller::VelocityControlState::new(rt),
            ));
        }
        app.world_mut()
            .resource_mut::<Time<Fixed>>()
            .advance_by(Duration::from_millis(10));
        app.world_mut().run_schedule(FixedUpdate);
        let (run, revision) = shared.authorities.lock().unwrap()[&1].clone();
        let bytes=serde_json::to_vec(&serde_json::json!({"linear":0.3,"angular":0,"run_id":run,"authority_revision":revision,"operator_session_id":"test","sequence":1})).unwrap();
        shared.receive("terra/rover", "terra/rover/1/teleop", &bytes);
        app.world_mut()
            .resource_mut::<Time<Fixed>>()
            .advance_by(Duration::from_millis(10));
        app.world_mut().run_schedule(FixedUpdate);
        let mut q = app.world_mut().query::<(&RoverId, &DriveCommand)>();
        let out = q
            .iter(app.world())
            .map(|(id, c)| (id.0, c.linear))
            .collect::<BTreeMap<_, _>>();
        assert_eq!(
            out[&1],
            0.3,
            "states: {:?}",
            shared
                .outgoing
                .lock()
                .unwrap()
                .states
                .iter()
                .filter(|((id, _), _)| *id == 1)
                .map(|(k, v)| (k, String::from_utf8_lossy(v).to_string()))
                .collect::<Vec<_>>()
        );
        assert_eq!(out[&2], 0.);
        shared.epoch.fetch_add(1, Ordering::Relaxed);
        app.world_mut()
            .resource_mut::<Time<Fixed>>()
            .advance_by(Duration::from_millis(10));
        app.world_mut().run_schedule(FixedUpdate);
        let mut q = app.world_mut().query::<(&RoverId, &DriveCommand)>();
        assert_eq!(
            q.iter(app.world())
                .find(|(id, _)| id.0 == 1)
                .unwrap()
                .1
                .linear,
            0.3
        );
        let revision_before = shared.authorities.lock().unwrap()[&1].1;
        shared.connection_generation.fetch_add(1, Ordering::Release);
        app.world_mut()
            .resource_mut::<Time<Fixed>>()
            .advance_by(Duration::from_millis(10));
        app.world_mut().run_schedule(FixedUpdate);
        assert!(shared.authorities.lock().unwrap()[&1].1 > revision_before);
        let mut q = app.world_mut().query::<(&RoverId, &DriveCommand)>();
        assert!(
            q.iter(app.world())
                .all(|(_, command)| command.linear == 0.0)
        );
        shared.receive(
            "terra/rover",
            "terra/rover/1/safety",
            br#"{"action":"stop","token":"stop"}"#,
        );
        app.world_mut()
            .resource_mut::<Time<Fixed>>()
            .advance_by(Duration::from_millis(10));
        app.world_mut().run_schedule(FixedUpdate);
        let mut q = app.world_mut().query::<(&RoverId, &DriveCommand)>();
        assert!(q.iter(app.world()).all(|(_, c)| c.linear == 0.));
        let payload =
            shared.outgoing.lock().unwrap().states[&(1, "autonomy/status".into())].clone();
        let value: serde_json::Value = serde_json::from_slice(&payload).unwrap();
        assert_eq!(value["safety"], "emergency_stop");
    }

    #[test]
    fn a_bound_reset_clears_an_emergency_stop_and_teleop_drives_again() {
        let shared = Arc::new(Shared::default());
        shared.connected.store(true, Ordering::Release);
        shared.commands.lock().unwrap().insert(1, None);
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(Bridge {
                shared: shared.clone(),
            })
            .add_systems(FixedUpdate, drive_autonomy);
        let rt = terra_runtime::Runtime::new(
            terra_runtime::RuntimeConfig {
                mode: terra_runtime::ActuationMode::DirectEffort,
                ..Default::default()
            },
            "test-1".into(),
        )
        .unwrap();
        app.world_mut().spawn((
            Rover,
            RoverId(1),
            ZenohControlled,
            Transform::default(),
            DriveCommand::default(),
            crate::velocity_controller::VelocityControlState::new(rt),
        ));
        let step = |app: &mut App| {
            app.world_mut()
                .resource_mut::<Time<Fixed>>()
                .advance_by(Duration::from_millis(10));
            app.world_mut().run_schedule(FixedUpdate);
        };
        let status = |shared: &Shared| -> serde_json::Value {
            let states = &shared.outgoing.lock().unwrap().states;
            serde_json::from_slice(&states[&(1, "autonomy/status".into())]).unwrap()
        };
        let linear = |app: &mut App| {
            let mut q = app.world_mut().query::<&DriveCommand>();
            q.single(app.world()).unwrap().linear
        };
        step(&mut app);
        shared.receive(
            "terra/rover",
            "terra/rover/1/safety",
            br#"{"action":"stop","token":"stop-1"}"#,
        );
        step(&mut app);
        assert_eq!(status(&shared)["safety"], "emergency_stop");
        // Like teleop, a reset must be bound to the current run and revision.
        shared.receive(
            "terra/rover",
            "terra/rover/1/safety",
            br#"{"action":"reset","token":"reset-unbound"}"#,
        );
        step(&mut app);
        assert_eq!(status(&shared)["result"], "rejected");
        assert_eq!(status(&shared)["safety"], "emergency_stop");
        let (run, revision) = shared.authorities.lock().unwrap()[&1].clone();
        let reset = serde_json::json!({"action":"reset","token":"reset-1","run_id":run,"authority_revision":revision});
        shared.receive(
            "terra/rover",
            "terra/rover/1/safety",
            &serde_json::to_vec(&reset).unwrap(),
        );
        step(&mut app);
        assert_eq!(status(&shared)["result"], "accepted");
        assert_eq!(status(&shared)["safety"], "clear");
        let (run, revision) = shared.authorities.lock().unwrap()[&1].clone();
        let teleop = serde_json::json!({"linear":0.3,"angular":0,"run_id":run,"authority_revision":revision,"operator_session_id":"test","sequence":1});
        shared.receive(
            "terra/rover",
            "terra/rover/1/teleop",
            &serde_json::to_vec(&teleop).unwrap(),
        );
        step(&mut app);
        assert_eq!(linear(&mut app), 0.3);
    }

    #[test]
    fn hardware_disarm_holds_motion_until_a_fresh_arm() {
        let shared = Arc::new(Shared::default());
        shared.connected.store(true, Ordering::Release);
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(Bridge {
                shared: shared.clone(),
            })
            .add_systems(FixedUpdate, drive_autonomy);
        shared.commands.lock().unwrap().insert(1, None);
        let rt = terra_runtime::Runtime::new(
            terra_runtime::RuntimeConfig {
                mode: terra_runtime::ActuationMode::DirectEffort,
                ..Default::default()
            },
            "test-1".into(),
        )
        .unwrap();
        app.world_mut().spawn((
            Rover,
            RoverId(1),
            ZenohControlled,
            Transform::default(),
            DriveCommand::default(),
            crate::velocity_controller::VelocityControlState::new(rt),
        ));
        let step = |app: &mut App| {
            app.world_mut()
                .resource_mut::<Time<Fixed>>()
                .advance_by(Duration::from_millis(10));
            app.world_mut().run_schedule(FixedUpdate);
        };
        let linear = |app: &mut App| {
            let mut q = app.world_mut().query::<(&RoverId, &DriveCommand)>();
            q.iter(app.world())
                .find(|(id, _)| id.0 == 1)
                .unwrap()
                .1
                .linear
        };
        step(&mut app);
        let (run, revision) = shared.authorities.lock().unwrap()[&1].clone();
        let teleop = |sequence: u64, revision: u64| {
            serde_json::to_vec(&serde_json::json!({
                "linear": 0.3,
                "angular": 0.0,
                "run_id": run,
                "authority_revision": revision,
                "operator_session_id": "test",
                "sequence": sequence
            }))
            .unwrap()
        };
        shared.receive("terra/rover", "terra/rover/1/teleop", &teleop(1, revision));
        step(&mut app);
        assert_eq!(linear(&mut app), 0.3);
        shared.receive(
            "terra/rover",
            "terra/rover/1/hardware",
            br#"{"action":"disarm","token":"disarm-1"}"#,
        );
        step(&mut app);
        assert_eq!(linear(&mut app), 0.0);
        let status = shared.outgoing.lock().unwrap().states[&(1, "hardware/status".into())].clone();
        let status: serde_json::Value = serde_json::from_slice(&status).unwrap();
        assert_eq!(status["armed"], false);
        assert_eq!(status["ready"], false);
        let (_run_after, revision_after) = shared.authorities.lock().unwrap()[&1].clone();
        let rejected = serde_json::to_vec(&serde_json::json!({
            "action": "arm",
            "token": "arm-stale",
            "run_id": "other-run",
            "authority_revision": revision_after
        }))
        .unwrap();
        shared.receive("terra/rover", "terra/rover/1/hardware", &rejected);
        shared.receive(
            "terra/rover",
            "terra/rover/1/teleop",
            &teleop(2, revision_after),
        );
        step(&mut app);
        assert_eq!(
            linear(&mut app),
            0.0,
            "a rejected arm must leave the rover disarmed"
        );
        let (run_armed, revision_armed) = shared.authorities.lock().unwrap()[&1].clone();
        let arm = serde_json::to_vec(&serde_json::json!({
            "action": "arm",
            "token": "arm-1",
            "run_id": run_armed,
            "authority_revision": revision_armed
        }))
        .unwrap();
        shared.receive("terra/rover", "terra/rover/1/hardware", &arm);
        shared.receive(
            "terra/rover",
            "terra/rover/1/teleop",
            &teleop(3, revision_armed),
        );
        step(&mut app);
        assert_eq!(linear(&mut app), 0.3);
    }
}
