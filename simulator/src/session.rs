//! In-app world selection. Headless runs and unit tests leave this resource unset
//! and keep the env-var startup path.

use crate::terra::MAX_ROVERS;
use crate::world::{SceneKind, WorldConfig};
use bevy::prelude::*;

/// Worlds the start screen and pause menu can load.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorldChoice {
    Town,
    Tiles,
    Next,
    #[cfg(all(feature = "gmu-viewer", not(target_arch = "wasm32")))]
    Gmu,
}

impl WorldChoice {
    pub fn label(self) -> &'static str {
        match self {
            Self::Town => "Practice town",
            Self::Tiles => "Outdoor map",
            Self::Next => "NEXT pitch",
            #[cfg(all(feature = "gmu-viewer", not(target_arch = "wasm32")))]
            Self::Gmu => "NEXT - GMU Stadium",
        }
    }

    pub fn from_env() -> Self {
        #[cfg(all(feature = "gmu-viewer", not(target_arch = "wasm32")))]
        if crate::gmu::requested() {
            return Self::Gmu;
        }
        #[cfg(all(feature = "gmu-viewer", not(target_arch = "wasm32")))]
        if crate::next_competition::enabled() { return Self::Gmu; }
        Self::Tiles
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Menu,
    Playing,
    Paused,
}

#[derive(Resource, Clone, Debug)]
pub struct SimSession {
    pub world: WorldChoice,
    pub rover_count: usize,
    pub phase: Phase,
    /// Bumped when the menu layout should be rebuilt.
    pub ui_epoch: u32,
    pub note: String,
}

impl SimSession {
    pub fn at_menu(world: WorldChoice, rover_count: usize) -> Self {
        Self {
            world,
            rover_count: rover_count.clamp(1, menu_rover_max()),
            phase: Phase::Menu,
            ui_epoch: 1,
            note: String::new(),
        }
    }

    /// Windowed and wasm start here. Env vars only preselect the highlighted world.
    pub fn from_env_menu() -> Self {
        let count = std::env::var("TERRA_ROVER_COUNT")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(1);
        Self::at_menu(WorldChoice::from_env(), count)
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

impl WorldConfig {
    pub fn for_choice(choice: WorldChoice) -> Self {
        let mut config = Self::default();
        match choice {
            WorldChoice::Town => {
                config.scene = SceneKind::Town;
            }
            WorldChoice::Tiles => {
                config.scene = SceneKind::Tiles;
                config.tiles.enabled = true;
                // The browser has no tile cache and no worker threads. The bundled
                // Terrarium PNG still covers the default Johnson Center anchor.
                config.tiles.fetch = cfg!(not(target_arch = "wasm32"));
                config.landscape.enabled = false;
                config.prepare_elevation_view();
            }
            WorldChoice::Next => config.prepare_next_field(),
            #[cfg(all(feature = "gmu-viewer", not(target_arch = "wasm32")))]
            WorldChoice::Gmu => {
                config.prepare_gmu_field();
            }
        }
        config
    }
}
