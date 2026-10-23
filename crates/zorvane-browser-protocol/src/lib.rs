//! Shared browser-host wire protocol.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};

pub const VERSION: u32 = 1;
pub const MAX_PACKET_BYTES: usize = 17 * 1024 * 1024;
pub const MAX_RELIABLE: usize = 512;
pub const HEARTBEAT_MS: u64 = 100;
pub const LEASE_MS: u64 = 500;
const MAX_QUEUE_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Control {
    Hello {
        version: u32,
        token: String,
    },
    Ready {
        version: u32,
    },
    Heartbeat,
    Subscribe {
        topics: Vec<String>,
    },
    Manifest {
        run_id: String,
        manifest: serde_json::Value,
    },
    Record {
        event: serde_json::Value,
    },
    Recorder {
        healthy: bool,
    },
    Error {
        message: String,
    },
}

/// Four-byte little-endian JSON header length, header, then unmodified bytes.
#[derive(Debug, Clone, PartialEq)]
pub struct Publication {
    pub topic: String,
    pub encoding: String,
    pub payload: Vec<u8>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Header {
    topic: String,
    encoding: String,
}

impl Publication {
    pub fn new(topic: impl Into<String>, encoding: impl Into<String>, payload: Vec<u8>) -> Self {
        Self {
            topic: topic.into(),
            encoding: encoding.into(),
            payload,
        }
    }

    pub fn encode(&self) -> Result<Vec<u8>, String> {
        if !valid_topic(&self.topic, false) || self.encoding.len() > 128 {
            return Err("invalid publication topic or encoding".into());
        }
        let header = serde_json::to_vec(&Header {
            topic: self.topic.clone(),
            encoding: self.encoding.clone(),
        })
        .map_err(|e| e.to_string())?;
        if header.len() > 4096 || 4 + header.len() + self.payload.len() > MAX_PACKET_BYTES {
            return Err("publication too large".into());
        }
        let mut bytes = Vec::with_capacity(4 + header.len() + self.payload.len());
        bytes.extend_from_slice(&(header.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&header);
        bytes.extend_from_slice(&self.payload);
        Ok(bytes)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() < 4 || bytes.len() > MAX_PACKET_BYTES {
            return Err("invalid publication size".into());
        }
        let size = u32::from_le_bytes(bytes[..4].try_into().unwrap()) as usize;
        if size > 4096 || size > bytes.len() - 4 {
            return Err("invalid publication header size".into());
        }
        let header: Header =
            serde_json::from_slice(&bytes[4..4 + size]).map_err(|e| e.to_string())?;
        if !valid_topic(&header.topic, false) || header.encoding.len() > 128 {
            return Err("invalid publication header".into());
        }
        Ok(Self::new(
            header.topic,
            header.encoding,
            bytes[4 + size..].to_vec(),
        ))
    }
}

/// Topic syntax shared by both backends: literal segments, `*`, and `**`.
pub fn valid_topic(topic: &str, expression: bool) -> bool {
    !topic.is_empty()
        && topic.len() <= 512
        && topic.split('/').all(|s| {
            (expression && matches!(s, "*" | "**"))
                || (!s.is_empty()
                    && !matches!(s, "." | "..")
                    && s.bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"_-.:".contains(&b)))
        })
}

pub fn matches_topic(expression: &str, topic: &str) -> bool {
    fn matches(e: &[&str], t: &[&str]) -> bool {
        match e.split_first() {
            None => t.is_empty(),
            Some((&"**", rest)) => matches(rest, t) || (!t.is_empty() && matches(e, &t[1..])),
            Some((head, rest)) => {
                !t.is_empty() && (*head == "*" || *head == t[0]) && matches(rest, &t[1..])
            }
        }
    }
    valid_topic(expression, true)
        && valid_topic(topic, false)
        && matches(
            &expression.split('/').collect::<Vec<_>>(),
            &topic.split('/').collect::<Vec<_>>(),
        )
}

/// Latest-value telemetry is replaceable; commands and reports retain ordering.
pub fn replaceable(topic: &str) -> bool {
    topic.contains("/camera/")
        || topic.ends_with("/pose")
        || topic.ends_with("/status")
        || topic.ends_with("/fleet/state")
        || topic.ends_with("/map/occupancy")
        || topic.ends_with("/map/occupancy/heartbeat")
        || topic.ends_with("/goal/proposal")
}

#[derive(Default)]
pub struct Outbox {
    reliable: VecDeque<Publication>,
    latest: BTreeMap<String, Publication>,
    bytes: usize,
}

impl Outbox {
    pub fn push(&mut self, publication: Publication) -> Result<(), String> {
        if publication.payload.len() > MAX_PACKET_BYTES {
            return Err("publication too large".into());
        }
        if publication.topic.ends_with("/map/occupancy")
            && let Some(stale) = self
                .latest
                .remove(&format!("{}/heartbeat", publication.topic))
        {
            self.bytes -= stale.payload.len();
        }
        let latest = replaceable(&publication.topic);
        let previous = if latest {
            self.latest
                .get(&publication.topic)
                .map_or(0, |p| p.payload.len())
        } else {
            0
        };
        let bytes = self.bytes - previous + publication.payload.len();
        if bytes > MAX_QUEUE_BYTES
            || (!latest && self.reliable.len() >= MAX_RELIABLE)
            || (latest
                && !self.latest.contains_key(&publication.topic)
                && self.latest.len() >= 1024)
        {
            return Err("publication queue full".into());
        }
        self.bytes = bytes;
        if latest {
            self.latest.insert(publication.topic.clone(), publication);
        } else {
            self.reliable.push_back(publication);
        }
        Ok(())
    }

    pub fn pop(&mut self) -> Option<Publication> {
        let value = self.reliable.pop_front().or_else(|| {
            // Small state updates precede bulk sensor data.
            let key = self
                .latest
                .keys()
                .find(|key| !key.contains("/camera/"))
                .or_else(|| self.latest.keys().next())?
                .clone();
            self.latest.remove(&key)
        })?;
        self.bytes -= value.payload.len();
        Some(value)
    }
}

/// Simulated counterpart of the phone's streamed motor-relay observation.
/// Readiness describes the virtual actuator connection; autonomy holds remain
/// on autonomy/status. Neutral output is ready to accept a new command stream.
pub fn simulated_relay_status(
    run_id: &str,
    connected: bool,
    output_active: bool,
) -> serde_json::Value {
    serde_json::json!({
        "schema_version": 1,
        "run_id": run_id,
        "control_mode": "streamed_relay",
        "connected": connected,
        "ready": connected,
        "output_active": connected && output_active,
        "fault": null,
        "simulated": true,
        "reason": if connected { "Simulated actuation ready" } else { "Simulator transport disconnected" },
    })
}

// Same emergency-stop ordering as Terra, generic over the platform clock.
pub type ControlAction<T> = (String, Vec<u8>, T);
fn stop_token<T>(a: &ControlAction<T>) -> Option<String> {
    if a.0 != "safety" {
        return None;
    }
    let v: serde_json::Value = serde_json::from_slice(&a.1).ok()?;
    let o = v.as_object()?;
    let token = o.get("token")?.as_str()?;
    (o.len() == 2
        && v["action"] == "stop"
        && (1..=64).contains(&token.len())
        && token
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._:-".contains(&b)))
    .then(|| token.into())
}
fn is_stop<T>(a: &ControlAction<T>) -> bool {
    stop_token(a).is_some()
}
pub fn enqueue_control_action<T>(
    q: &mut std::collections::VecDeque<ControlAction<T>>,
    kind: String,
    payload: Vec<u8>,
    received: T,
) {
    if !matches!(
        kind.as_str(),
        "autonomy"
            | "goal"
            | "navigation/settings"
            | "navigation/source"
            | "navigation/resume"
            | "teleop"
            | "safety"
            | "goal/decision"
            | "mission/report"
            | "search"
            | "search/action"
            | "search/report/ack"
    ) {
        return;
    }
    let action = (kind, payload, received);
    if let Some(token) = stop_token(&action) {
        q.retain(|a| stop_token(a).as_deref() != Some(&token));
    }
    if q.len() >= 256 {
        if let Some(i) = q.iter().position(|a| !is_stop(a)) {
            q.remove(i);
        } else {
            return;
        }
    }
    q.push_back(action);
}
/// A stop dominates every other request received in the same control tick, including reset.
pub fn drain_control_actions<T>(
    q: &mut std::collections::VecDeque<ControlAction<T>>,
) -> Vec<ControlAction<T>> {
    let mut actions = std::mem::take(q).into_iter().collect::<Vec<_>>();
    actions.sort_by_key(is_stop);
    actions
}
