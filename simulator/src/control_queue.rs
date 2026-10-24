//! Terra control inbox rules: topic syntax, emergency-stop ordering, and the
//! simulated relay status.

/// One received control sample: kind, payload, and when it arrived.
pub type ControlAction<T> = (String, Vec<u8>, T);

/// Topic syntax: literal segments, plus `*` and `**` in expressions.
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

// Same emergency-stop ordering as Terra.
fn stop_token<T>(a: &ControlAction<T>) -> Option<String> {
    if a.0 == "hardware" {
        let v: serde_json::Value = serde_json::from_slice(&a.1).ok()?;
        let token = v["token"].as_str()?;
        return (v["action"] == "disarm" && (1..=64).contains(&token.len()))
            .then(|| format!("hardware:{token}"));
    }
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

/// Queues a control sample. A repeated stop replaces its earlier copy, and a
/// full queue drops the oldest non-stop sample first.
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
            | "hardware"
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    #[test]
    fn topics_are_whole_literal_segments() {
        assert!(valid_topic("terra/rover", false));
        assert!(valid_topic("terra/*/state", true));
        assert!(!valid_topic("terra/*/state", false));
        assert!(!valid_topic("terra//rover", false));
        assert!(!valid_topic("terra/../rover", false));
        assert!(!valid_topic("", false));
    }

    #[test]
    fn emergency_stop_survives_control_queue_pressure_and_runs_last() {
        let mut queue = VecDeque::new();
        enqueue_control_action(
            &mut queue,
            "safety".into(),
            br#"{"action":"stop","token":"stop-1"}"#.to_vec(),
            0,
        );
        for i in 1..400 {
            enqueue_control_action(&mut queue, "teleop".into(), b"{}".to_vec(), i);
        }
        let actions = drain_control_actions(&mut queue);
        assert_eq!(actions.len(), 256);
        assert_eq!(actions.last().unwrap().0, "safety");
    }

    #[test]
    fn hardware_disarm_survives_control_queue_pressure_and_runs_last() {
        let mut queue = VecDeque::new();
        enqueue_control_action(
            &mut queue,
            "hardware".into(),
            br#"{"action":"disarm","token":"disarm-1"}"#.to_vec(),
            0,
        );
        for i in 1..400 {
            enqueue_control_action(&mut queue, "teleop".into(), b"{}".to_vec(), i);
        }
        let actions = drain_control_actions(&mut queue);
        assert_eq!(actions.last().unwrap().0, "hardware");
    }

    #[test]
    fn unknown_kinds_are_dropped() {
        let mut queue = VecDeque::new();
        enqueue_control_action(&mut queue, "drive".into(), b"{}".to_vec(), 0);
        assert!(queue.is_empty());
    }
}
