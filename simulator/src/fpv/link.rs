//! Zenoh publish/subscribe limited to the FPV signaling keys.
//!
//! Ported from ARGOS `argos-gpui/src/fpv/link.rs`. The session is separate from
//! the simulator bridge so this path can only carry `fpv_signaling` payloads.

use fpv_signaling::{MAX_BYTES, Topic, topic_key};
use tokio::sync::mpsc;
use zenoh::Wait;

pub(super) struct Bus {
    session: zenoh::Session,
    prefix: String,
    subscribers: Vec<zenoh::pubsub::Subscriber<()>>,
}

impl Bus {
    pub(super) fn connect(
        endpoint: &str,
        prefix: &str,
        credentials: Option<(&str, &str)>,
    ) -> Result<Self, String> {
        let mut config = zenoh::Config::default();
        let set = |config: &mut zenoh::Config, key: &str, value: &str| {
            config
                .insert_json5(key, value)
                .map_err(|error| error.to_string())
        };
        set(&mut config, "mode", "\"client\"")?;
        set(
            &mut config,
            "connect/endpoints",
            &serde_json::to_string(&vec![endpoint]).unwrap(),
        )?;
        set(&mut config, "scouting/multicast/enabled", "false")?;
        set(&mut config, "connect/timeout_ms", "3000")?;
        if let Some((user, password)) = credentials {
            set(
                &mut config,
                "transport/auth/usrpwd/user",
                &serde_json::to_string(user).unwrap(),
            )?;
            set(
                &mut config,
                "transport/auth/usrpwd/password",
                &serde_json::to_string(password).unwrap(),
            )?;
        }
        let session = zenoh::open(config)
            .wait()
            .map_err(|error| format!("FPV link could not connect: {error}"))?;
        Ok(Self {
            session,
            prefix: prefix.trim_matches('/').to_string(),
            subscribers: Vec::new(),
        })
    }

    pub(super) fn subscribe(&mut self, rover: u64, topic: Topic) -> mpsc::Receiver<Vec<u8>> {
        let (tx, rx) = mpsc::channel(64);
        let key = topic_key(&self.prefix, rover, topic);
        if let Ok(subscriber) = self
            .session
            .declare_subscriber(key)
            .callback(move |sample| {
                let _ = tx.try_send(sample.payload().to_bytes().to_vec());
            })
            .wait()
        {
            self.subscribers.push(subscriber);
        }
        rx
    }

    /// Publish one signaling sample on one of the WebRTC suffixes.
    pub(super) fn publish(&self, rover: u64, topic: Topic, body: &str) -> Result<(), String> {
        if body.len() > MAX_BYTES {
            return Err("FPV signaling payload exceeds 65536 bytes".into());
        }
        self.session
            .put(topic_key(&self.prefix, rover, topic), body)
            .wait()
            .map_err(|error| format!("FPV publish failed: {error}"))
    }
}
