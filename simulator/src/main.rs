mod depth_camera;
mod geo;
mod landscape;
mod menu;
mod mission;
mod next_competition;
mod occupancy_map;
mod physics;
mod rgb_camera;
mod robotics_pose;
mod session;
mod target_search;
mod terra;
mod vehicle;
mod velocity_controller;
#[cfg(feature = "native")]
mod voxel_terrain;
#[cfg(not(feature = "native"))]
mod web_tiles;
mod world;
#[cfg(feature = "native")]
mod zenoh_bridge;

use bevy::prelude::*;
use depth_camera::TerraDepthCameraPlugin;
use physics::TerraPhysicsPlugin;
use terra::TerraPlugin;
use world::TerraWorldPlugin;

fn asset_root() -> String {
    if let Ok(path) = std::env::var("ZORVANE_ASSETS") {
        return path;
    }
    // Trunk copies `assets/` next to the wasm module. CARGO_MANIFEST_DIR is a
    // host path and does not exist in the browser.
    #[cfg(target_arch = "wasm32")]
    {
        "assets".to_string()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        concat!(env!("CARGO_MANIFEST_DIR"), "/assets").into()
    }
}

fn main() {
    #[cfg(target_arch = "wasm32")]
    console_error_panic_hook::set_once();
    let vehicles = zorvane_vehicle::VehicleRegistry::from_env().expect("invalid ZORVANE_VEHICLE");
    let body = vehicles.active();
    let drive = terra::DriveConfig::from_body(body).unwrap_or_else(|reason| {
        panic!(
            "vehicle '{}' cannot be driven by this Zorvane build: {reason}",
            body.id()
        )
    });
    let chassis = physics::RoverPhysicsConfig::from_body(body).expect("invalid vehicle chassis");
    eprintln!("Zorvane vehicle: {} ({})", body.display_name(), body.id());

    let mut app = App::new();
    if session::interactive_menu() {
        app.insert_resource(session::SimSession::from_env_menu());
    }
    mission::install(&mut app);
    next_competition::install(&mut app);
    // A static host has no .meta sidecars. Skipping the lookup avoids a 404
    // before each asset, including the 118 MB rover mesh.
    #[cfg(target_arch = "wasm32")]
    let assets = AssetPlugin {
        file_path: asset_root(),
        meta_check: bevy::asset::AssetMetaCheck::Never,
        ..default()
    };
    #[cfg(not(target_arch = "wasm32"))]
    let assets = AssetPlugin {
        file_path: asset_root(),
        ..default()
    };
    let defaults = DefaultPlugins.set(assets);
    if std::env::var("TERRA_HEADLESS").as_deref() == Ok("1") {
        app.add_plugins(
            defaults
                .disable::<bevy::winit::WinitPlugin>()
                .set(WindowPlugin {
                    primary_window: None,
                    exit_condition: bevy::window::ExitCondition::DontExit,
                    ..default()
                }),
        );
        app.add_plugins(bevy::app::ScheduleRunnerPlugin::run_loop(
            std::time::Duration::from_millis(5),
        ));
    } else {
        app.add_plugins(defaults);
    }

    // The menu installs the town's water and voxel plugins, then spawns the
    // chosen scene when Start is pressed. Env vars still preselect that choice.
    let world_config = if session::interactive_menu() {
        world::WorldConfig::default()
    } else {
        world::WorldConfig::from_env().expect("invalid Terra world configuration")
    };
    app.insert_resource(vehicle::Vehicles(vehicles))
        .insert_resource(chassis)
        .insert_resource(terra::RoverFleet::from_env().expect("invalid fleet configuration"))
        .add_plugins((
            TerraPhysicsPlugin,
            TerraWorldPlugin {
                config: world_config,
            },
            TerraPlugin { config: drive },
            velocity_controller::TerraVelocityControlPlugin,
            TerraDepthCameraPlugin::default(),
            occupancy_map::TerraOccupancyMapPlugin,
            rgb_camera::TerraRgbCameraPlugin,
            #[cfg(feature = "native")]
            zenoh_bridge::TerraZenohPlugin {
                config: zenoh_bridge::ZenohBridgeConfig::from_env(),
            },
        ));
    if session::interactive_menu() {
        app.add_plugins(menu::MenuPlugin);
    }
    if std::env::var("ZORVANE_SMOKE").as_deref() == Ok("1") {
        app.add_systems(Update, smoke_exit);
    }
    // Rebuilding a world despawns entities that a queued UI command may still
    // name. On wasm that panic aborts the module; log it and keep rendering.
    #[cfg(target_arch = "wasm32")]
    app.insert_resource(bevy::ecs::error::FallbackErrorHandler(
        bevy::ecs::error::warn,
    ));
    app.run();
}

/// Headless boot check. Exits after a few frames once the world schedule is running.
fn smoke_exit(mut frames: Local<u32>, mut exit: MessageWriter<AppExit>) {
    *frames += 1;
    if *frames >= 3 {
        eprintln!("Zorvane smoke: world started");
        exit.write(AppExit::Success);
    }
}
