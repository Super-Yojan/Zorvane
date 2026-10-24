//! Native durable operational evidence; browser builds retain their existing telemetry.
use serde_json::Value;
#[cfg(feature = "native")]
use terra_observability::EventInput;
pub struct Projector {
    last: std::collections::BTreeMap<String, (f64, String)>,
    period: f64,
}
impl Default for Projector {
    fn default() -> Self {
        Self {
            last: Default::default(),
            period: default_period(),
        }
    }
}
fn default_period() -> f64 {
    #[cfg(feature = "native")]
    {
        terra_observability::runtime_settings().control_sample_seconds
    }
    #[cfg(not(feature = "native"))]
    {
        0.1
    }
}
impl Projector {
    #[cfg(feature = "native")]
    pub fn project(&mut self, value: Value) -> Vec<EventInput> {
        let rover = value["rover_id"].as_u64().unwrap_or(0).to_string();
        let time = value["time"].as_f64().unwrap_or(0.);
        let key = serde_json::json!([
            value["status"]["reason"],
            value["status"]["safety"],
            value["status"]["requested_level"],
            value["status"]["active_source"]
        ])
        .to_string();
        let previous = self
            .last
            .entry(rover.clone())
            .or_insert((f64::NEG_INFINITY, String::new()));
        let changed = previous.1 != key;
        let mut events = Vec::new();
        let mut emit = |kind: &str, attributes: Value| {
            let mut e = EventInput::new(&rover, "simulator", kind, attributes);
            e.session_id = value["run_id"].as_str().map(str::to_string);
            e.monotonic_ns = (time.max(0.) * 1e9) as u64;
            events.push(e);
        };
        if changed {
            emit(
                "autonomy.transition",
                serde_json::json!({"previous":previous.1,"reason":value["status"]["reason"],"status":value["status"]}),
            );
            previous.1 = key;
        }
        if changed || time - previous.0 >= self.period {
            emit("control.summary", value.clone());
            previous.0 = time;
        }
        if let Some(discrete) = value["events"].as_array() {
            for event in discrete {
                emit("autonomy.event", event.clone());
            }
        }
        events
    }
}
pub fn initialize() {
    #[cfg(feature = "native")]
    terra_observability::initialize_runtime("zorvane");
}
pub fn record_tick(value: Value) {
    #[cfg(feature = "native")]
    {
        use std::sync::{Mutex, OnceLock};
        static PROJECTOR: OnceLock<Mutex<Projector>> = OnceLock::new();
        if let Ok(mut p) = PROJECTOR
            .get_or_init(|| Mutex::new(Projector::default()))
            .try_lock()
        {
            for e in p.project(value) {
                terra_observability::record_runtime_input(e);
            }
        }
    }
    #[cfg(not(feature = "native"))]
    let _ = value;
}
