//! Start screen and pause state. Headless runs and unit tests leave this
//! resource unset and spawn the field at startup.

use crate::terra::MAX_ROVERS;
use bevy::prelude::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Menu,
    Playing,
    Paused,
}

#[derive(Resource, Clone, Debug)]
pub struct SimSession {
    pub rover_count: usize,
    pub phase: Phase,
    /// Bumped when the menu layout should be rebuilt.
    pub ui_epoch: u32,
}

impl SimSession {
    pub fn at_menu(rover_count: usize) -> Self {
        Self {
            rover_count: rover_count.clamp(1, menu_rover_max()),
            phase: Phase::Menu,
            ui_epoch: 1,
        }
    }

    /// Windowed runs start here. `TERRA_ROVER_COUNT` preselects the fleet size.
    /// A relaunch from the menu (`ZORVANE_AUTOSTART=1`) skips straight to play.
    pub fn from_env_menu() -> Self {
        let count = std::env::var("TERRA_ROVER_COUNT")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(1);
        let mut session = Self::at_menu(count);
        if std::env::var(crate::settings::AUTOSTART).as_deref() == Ok("1") {
            session.phase = Phase::Playing;
        }
        session
    }
}

pub fn menu_rover_max() -> usize {
    MAX_ROVERS.min(8)
}

/// Present only for the frame that should despawn and respawn the world.
#[derive(Resource, Default)]
pub struct RebuildWorld;

pub fn interactive_menu() -> bool {
    if std::env::var("ZORVANE_MENU").as_deref() == Ok("0") {
        return false;
    }
    if std::env::var("TERRA_HEADLESS").as_deref() == Ok("1") {
        return false;
    }
    if std::env::var("ZORVANE_SMOKE").as_deref() == Ok("1") {
        return false;
    }
    true
}

pub fn drive_allowed(session: Option<Res<SimSession>>) -> bool {
    session.is_none_or(|session| session.phase == Phase::Playing)
}
