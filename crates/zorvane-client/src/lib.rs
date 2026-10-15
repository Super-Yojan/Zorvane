//! Subscribe to `terra/rover/<id>/pose` from Python's Rust counterpart.
//!
//! `heading` and `yaw` in the JSON body are the same angle in radians.

use std::net::{SocketAddr, TcpStream};
use std::sync::Mutex;
use std::time::Duration;

use zenoh::Wait;
use zenoh::pubsub::Subscriber;

const DEFAULT_ENDPOINT: &str = "tcp/127.0.0.1:7447";
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

type PoseSubscriber = Subscriber<()>;

/// A Zenoh session plus the pose subscribers created from it.
pub struct Client {
    subscribers: Mutex<Vec<PoseSubscriber>>,
    session: zenoh::Session,
}

impl Client {
    /// Join `router`, or the simulator when it is already listening.
    ///
    /// `None` probes `tcp/127.0.0.1:7447`. An open port is a client connection
    /// (the simulator's peer, or the student router). A closed port makes this
    /// process listen there so a simulator started with
    /// `TERRA_ZENOH_CONNECT=tcp/127.0.0.1:7447` can join it.
    ///
    /// An explicit router is always a client connect. `ws://` is rejected:
    /// native clients use TCP, and the browser menu uses the WebSocket form.
    pub fn connect(router: Option<&str>) -> Result<Self> {
        connect_endpoint(router, DEFAULT_ENDPOINT)
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

fn connect_endpoint(router: Option<&str>, fallback: &str) -> Result<Client> {
    let (mode, key, endpoint) = if let Some(router) = router {
        reject_websocket(router)?;
        if !router.starts_with("tcp/") {
            return Err(ClientError(
                "router must look like tcp/127.0.0.1:7447. ws:// is only for the browser simulator."
                    .into(),
            ));
        }
        ("client", "connect/endpoints", router)
    } else if tcp_open(fallback)? {
        ("client", "connect/endpoints", fallback)
    } else {
        ("peer", "listen/endpoints", fallback)
    };
    let session = open_session(mode, key, endpoint)?;
    Ok(Client {
        subscribers: Mutex::new(Vec::new()),
        session,
    })
}

fn reject_websocket(router: &str) -> Result<()> {
    if router.starts_with("ws://")
        || router.starts_with("wss://")
        || router.starts_with("ws/")
        || router.starts_with("wss/")
    {
        return Err(ClientError(
            "Rust uses a TCP router such as tcp/127.0.0.1:7447. ws:// is only for the browser simulator."
                .into(),
        ));
    }
    Ok(())
}

fn split_tcp(endpoint: &str) -> Result<(&str, u16)> {
    let Some(hostport) = endpoint.strip_prefix("tcp/") else {
        return Err(ClientError(format!(
            "expected tcp/host:port, got {endpoint}"
        )));
    };
    let Some((host, port)) = hostport.rsplit_once(':') else {
        return Err(ClientError(format!(
            "expected tcp/host:port, got {endpoint}"
        )));
    };
    let port = port
        .parse()
        .map_err(|_| ClientError(format!("expected tcp/host:port, got {endpoint}")))?;
    Ok((host, port))
}

fn tcp_open(endpoint: &str) -> Result<bool> {
    let (host, port) = split_tcp(endpoint)?;
    let address: SocketAddr = format!("{host}:{port}")
        .parse()
        .map_err(|_| ClientError(format!("expected tcp/host:port, got {endpoint}")))?;
    Ok(TcpStream::connect_timeout(&address, Duration::from_millis(250)).is_ok())
}

fn open_session(mode: &str, key: &str, endpoint: &str) -> Result<zenoh::Session> {
    let mut config = zenoh::Config::default();
    config
        .insert_json5("mode", &format!("\"{mode}\""))
        .map_err(|error| ClientError(error.to_string()))?;
    let endpoints = serde_json::json!([endpoint]).to_string();
    config
        .insert_json5(key, &endpoints)
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
    fn websocket_urls_are_rejected() {
        let Err(error) = Client::connect(Some("ws://localhost:10000")) else {
            panic!("websocket router was accepted");
        };
        let message = error.to_string();
        assert!(message.contains("TCP"), "{message}");
    }

    #[test]
    fn peer_listener_delivers_a_pose_to_the_subscriber() {
        let endpoint = "tcp/127.0.0.1:17447";
        let listener = connect_endpoint(None, endpoint).unwrap();
        let (sender, receiver) = mpsc::channel();
        listener
            .subscribe_pose(0, move |pose| {
                let _ = sender.send(pose);
            })
            .unwrap();
        let publisher = connect_endpoint(Some(endpoint), endpoint).unwrap();
        publisher
            .session
            .put(
                "terra/rover/0/pose",
                r#"{"x":3.0,"y":4.0,"yaw":0.5,"heading":0.5}"#,
            )
            .wait()
            .unwrap();
        let pose = receiver
            .recv_timeout(Duration::from_secs(3))
            .expect("pose sample");
        assert_eq!(pose.x, 3.0);
        assert_eq!(pose.y, 4.0);
        assert_eq!(pose.heading, 0.5);
    }
}
