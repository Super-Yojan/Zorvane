//! Subscribe to `terra/rover/<id>/pose`.
//!
//! `heading` and `yaw` in the JSON body are the same angle in radians.

use std::sync::Mutex;

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

/// A Zenoh session joined to the simulator, plus pose subscribers.
pub struct Client {
    subscribers: Mutex<Vec<Subscriber<()>>>,
    session: zenoh::Session,
}

impl Client {
    /// Join the simulator. It already listens on `tcp/127.0.0.1:7447`.
    pub fn connect() -> Result<Self> {
        connect_to(ENDPOINT)
    }

    /// Invoke `callback` for each sample on `terra/rover/<id>/pose`.
    ///
    /// The subscription stays alive until this client is dropped.
    pub fn subscribe_pose<F>(&self, rover_id: u64, callback: F) -> Result<()>
    where
        F: Fn(Pose) + Send + Sync + 'static,
    {
        let subscriber = self
            .session
            .declare_subscriber(format!("{PREFIX}/{rover_id}/pose"))
            .callback(move |sample| {
                let bytes = sample.payload().to_bytes();
                if let Some(pose) = parse_pose(bytes.as_ref()) {
                    callback(pose);
                }
            })
            .wait()
            .map_err(|error| ClientError(error.to_string()))?;
        self.subscribers
            .lock()
            .expect("pose subscribers")
            .push(subscriber);
        Ok(())
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
        subscribers: Mutex::new(Vec::new()),
        session,
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
