//! Subscribe to `terra/rover/<id>/pose`.
//!
//! `heading` and `yaw` in the JSON body are the same angle in radians.

use std::sync::{Arc, Mutex};
use std::time::Duration;

mod browser;
mod camera;
mod page;
pub use browser::{BrowserHostConfig, ConnectionState};
pub use camera::{CameraFrame, CameraHeader, decode_camera_frame};
pub use zorvane_browser_protocol::Publication;

use zenoh::Wait;
use zenoh::pubsub::Subscriber;

const ENDPOINT: &str = "tcp/127.0.0.1:7447";
const PREFIX: &str = "terra/rover";

/// Rover position in the robotics frame. `x` is −Bevy Z and `y` is −Bevy X.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    pub x: f64,
    pub y: f64,
    pub heading: f64,
}

impl Pose {
    /// Same value as [`Pose::heading`].
    pub fn yaw(self) -> f64 {
        self.heading
    }
}

#[derive(Debug)]
pub struct ClientError(String);

impl std::fmt::Display for ClientError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for ClientError {}

pub type Result<T> = std::result::Result<T, ClientError>;

/// A native Zenoh connection or an embedded WebSocket host for a browser simulator.
/// Subscriptions live until the client is dropped. Callbacks must return promptly.
pub struct Client {
    backend: Backend,
}

enum Backend {
    Native {
        subscribers: Mutex<Vec<Subscriber<()>>>,
        session: zenoh::Session,
    },
    Browser(browser::BrowserHost),
}

impl Client {
    /// Join a native simulator at `tcp/127.0.0.1:7447`.
    pub fn connect() -> Result<Self> {
        connect_to(ENDPOINT)
    }

    pub fn connect_to(endpoint: &str) -> Result<Self> {
        connect_to(endpoint)
    }

    /// Start a gateway inside this process; use `pairing_url` to connect a browser.
    pub fn host_browser(config: BrowserHostConfig) -> Result<Self> {
        Ok(Self {
            backend: Backend::Browser(browser::BrowserHost::start(config)?),
        })
    }

    /// URL contains a pairing secret. Share it only with the intended local browser.
    pub fn pairing_url(&self) -> Option<String> {
        match &self.backend {
            Backend::Browser(host) => Some(host.pairing_url().into()),
            _ => None,
        }
    }

    pub fn connection_state(&self) -> ConnectionState {
        match &self.backend {
            Backend::Browser(host) => host.connection_state(),
            Backend::Native { session, .. } => {
                if session.is_closed() {
                    ConnectionState::Disconnected("Zenoh session closed".into())
                } else {
                    ConnectionState::Connected
                }
            }
        }
    }

    pub fn wait_connected(&self, timeout: Duration) -> Result<()> {
        match &self.backend {
            Backend::Browser(host) => host.wait_connected(timeout),
            Backend::Native { session, .. } if session.is_closed() => {
                Err(ClientError("Zenoh session closed".into()))
            }
            _ => Ok(()),
        }
    }

    /// Subscribe to literal topics or whole-segment `*` / `**` expressions.
    pub fn subscribe<F>(&self, expression: &str, callback: F) -> Result<()>
    where
        F: Fn(Publication) + Send + Sync + 'static,
    {
        if !zorvane_browser_protocol::valid_topic(expression, true) {
            return Err(ClientError("invalid topic expression".into()));
        }
        match &self.backend {
            Backend::Browser(host) => host.subscribe(expression.into(), Arc::new(callback)),
            Backend::Native {
                subscribers,
                session,
            } => {
                let subscriber = session
                    .declare_subscriber(expression)
                    .callback(move |sample| {
                        callback(Publication::new(
                            sample.key_expr().as_str(),
                            sample.encoding().to_string(),
                            sample.payload().to_bytes().to_vec(),
                        ));
                    })
                    .wait()
                    .map_err(|e| ClientError(e.to_string()))?;
                subscribers.lock().unwrap().push(subscriber);
                Ok(())
            }
        }
    }

    /// Publish unmodified bytes and MIME encoding on a concrete topic.
    /// Browser commands fail when unpaired and are never replayed after reconnect.
    pub fn publish(&self, topic: &str, encoding: &str, payload: impl Into<Vec<u8>>) -> Result<()> {
        let publication = Publication::new(topic, encoding, payload.into());
        publication.encode().map_err(ClientError)?;
        match &self.backend {
            Backend::Browser(host) => host.publish(publication),
            Backend::Native { session, .. } => session
                .put(topic, publication.payload)
                .encoding(zenoh::bytes::Encoding::from(encoding.to_owned()))
                .wait()
                .map_err(|e| ClientError(e.to_string())),
        }
    }

    pub fn publish_json(&self, topic: &str, payload: &serde_json::Value) -> Result<()> {
        self.publish(
            topic,
            "application/json",
            serde_json::to_vec(payload).map_err(|e| ClientError(e.to_string()))?,
        )
    }

    pub fn subscribe_pose<F>(&self, rover_id: u64, callback: F) -> Result<()>
    where
        F: Fn(Pose) + Send + Sync + 'static,
    {
        self.subscribe(&format!("{PREFIX}/{rover_id}/pose"), move |sample| {
            if let Some(pose) = parse_pose(&sample.payload) {
                callback(pose);
            }
        })
    }

    /// One velocity sample; refresh faster than the simulator's 500 ms lease.
    pub fn set_velocity(&self, rover_id: u64, linear: f64, angular: f64) -> Result<()> {
        if !linear.is_finite()
            || !angular.is_finite()
            || linear.abs() > f32::MAX as f64
            || angular.abs() > f32::MAX as f64
        {
            return Err(ClientError("velocity must be finite float32 values".into()));
        }
        self.publish_json(
            &format!("{PREFIX}/{rover_id}/cmd_vel"),
            &serde_json::json!({"linear":linear,"angular":angular}),
        )
    }

    pub fn set_fleet_size(&self, count: usize) -> Result<()> {
        if count > 32 {
            return Err(ClientError("fleet size must be 0 through 32".into()));
        }
        self.publish_json(
            &format!("{PREFIX}/fleet/size"),
            &serde_json::json!({"count":count}),
        )
    }

    /// Publish one goal using the simulator's existing local/WGS84 JSON contract.
    pub fn send_goal(&self, rover_id: u64, goal: &serde_json::Value) -> Result<()> {
        let bytes = serde_json::to_vec(goal).map_err(|e| ClientError(e.to_string()))?;
        if bytes.len() > 2048 {
            return Err(ClientError("goal exceeds 2048 bytes".into()));
        }
        self.publish(
            &format!("{PREFIX}/{rover_id}/goal"),
            "application/json",
            bytes,
        )
    }

    pub fn cancel_goal(&self, rover_id: u64) -> Result<()> {
        self.send_goal(rover_id, &serde_json::json!({"cancel":true}))
    }
}

pub fn parse_pose(payload: &[u8]) -> Option<Pose> {
    let value: serde_json::Value = serde_json::from_slice(payload).ok()?;
    let x = value.get("x")?.as_f64()?;
    let y = value.get("y")?.as_f64()?;
    let heading = value
        .get("heading")
        .and_then(serde_json::Value::as_f64)
        .or_else(|| value.get("yaw").and_then(serde_json::Value::as_f64))?;
    Some(Pose { x, y, heading })
}

fn connect_to(endpoint: &str) -> Result<Client> {
    let session = open_session("client", "connect/endpoints", endpoint)?;
    Ok(Client {
        backend: Backend::Native {
            subscribers: Mutex::new(Vec::new()),
            session,
        },
    })
}

fn open_session(mode: &str, key: &str, endpoint: &str) -> Result<zenoh::Session> {
    let mut config = zenoh::Config::default();
    config
        .insert_json5("mode", &format!("\"{mode}\""))
        .map_err(|error| ClientError(error.to_string()))?;
    config
        .insert_json5(key, &serde_json::json!([endpoint]).to_string())
        .map_err(|error| ClientError(error.to_string()))?;
    config
        .insert_json5("scouting/multicast/enabled", "false")
        .map_err(|error| ClientError(error.to_string()))?;
    zenoh::open(config)
        .wait()
        .map_err(|error| ClientError(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn pose_json_uses_heading() {
        let pose = parse_pose(br#"{"x":1.5,"y":-2,"yaw":0.25,"heading":0.25}"#).unwrap();
        assert_eq!(pose.x, 1.5);
        assert_eq!(pose.y, -2.0);
        assert_eq!(pose.heading, 0.25);
        assert_eq!(pose.yaw(), pose.heading);
    }

    #[test]
    fn yaw_fills_in_when_heading_is_absent() {
        let pose = parse_pose(br#"{"x":0,"y":1,"yaw":1.2}"#).unwrap();
        assert_eq!(pose.heading, 1.2);
    }

    #[test]
    fn subscriber_receives_a_pose() {
        let endpoint = "tcp/127.0.0.1:17447";
        let peer = open_session("peer", "listen/endpoints", endpoint).unwrap();
        let client = connect_to(endpoint).unwrap();
        let (sender, receiver) = mpsc::channel();
        client
            .subscribe_pose(0, move |pose| {
                let _ = sender.send(pose);
            })
            .unwrap();
        let payload = r#"{"x":3.0,"y":4.0,"yaw":0.5,"heading":0.5}"#;
        let pose = (0..20)
            .find_map(|_| {
                peer.put("terra/rover/0/pose", payload).wait().unwrap();
                receiver.recv_timeout(Duration::from_millis(200)).ok()
            })
            .expect("pose sample");
        assert_eq!(pose.x, 3.0);
        assert_eq!(pose.y, 4.0);
        assert_eq!(pose.heading, 0.5);
    }
}
