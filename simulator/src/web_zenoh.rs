//! Browser pose publisher. The Rust Zenoh crate does not compile to wasm, so this
//! calls `zenoh_browser.js`, which speaks to zenohd through zenoh-ts.

use crate::robotics_pose::robotics_pose;
use crate::session::{Phase, SimSession};
use crate::terra::{Rover, RoverId};
use bevy::prelude::*;
use std::time::{Duration, Instant};
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = ["window", "zorvaneZenoh"], catch, js_name = connect)]
    fn js_connect(url: &str) -> Result<(), JsValue>;

    #[wasm_bindgen(js_namespace = ["window", "zorvaneZenoh"], catch, js_name = ready)]
    fn js_ready() -> Result<bool, JsValue>;

    #[wasm_bindgen(js_namespace = ["window", "zorvaneZenoh"], catch, js_name = publishPose)]
    fn js_publish_pose(id: u32, x: f64, y: f64, heading: f64) -> Result<(), JsValue>;
}

pub struct WebZenohPlugin;

impl Plugin for WebZenohPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<WebZenohState>()
            .add_systems(Update, publish_browser_pose);
    }
}

#[derive(Resource)]
struct WebZenohState {
    last_publish: Option<Instant>,
    last_attempt: Option<Instant>,
    warned: bool,
}

impl Default for WebZenohState {
    fn default() -> Self {
        Self {
            last_publish: None,
            last_attempt: None,
            warned: false,
        }
    }
}

fn publish_browser_pose(
    session: Option<Res<SimSession>>,
    rovers: Query<(&RoverId, &Transform), With<Rover>>,
    mut state: ResMut<WebZenohState>,
) {
    let Some(session) = session else {
        return;
    };
    if session.phase != Phase::Playing || session.router.is_empty() {
        return;
    }
    let now = Instant::now();
    let ready = match js_ready() {
        Ok(ready) => ready,
        Err(error) => {
            warn_once(&mut state, &error);
            return;
        }
    };
    if !ready {
        let due = state
            .last_attempt
            .is_none_or(|last| now.duration_since(last) >= Duration::from_secs(2));
        if due {
            state.last_attempt = Some(now);
            if let Err(error) = js_connect(&session.router) {
                warn_once(&mut state, &error);
            }
        }
        return;
    }
    if state
        .last_publish
        .is_some_and(|last| now.duration_since(last) < Duration::from_millis(100))
    {
        return;
    }
    state.last_publish = Some(now);
    for (id, transform) in &rovers {
        let (x, y, heading) = robotics_pose(transform.translation, transform.rotation);
        let id = u32::try_from(id.0).unwrap_or(u32::MAX);
        if let Err(error) = js_publish_pose(id, x, y, heading) {
            warn_once(&mut state, &error);
        }
    }
}

fn warn_once(state: &mut WebZenohState, error: &JsValue) {
    if state.warned {
        return;
    }
    state.warned = true;
    warn!("browser Zenoh is not publishing pose: {error:?}");
}
