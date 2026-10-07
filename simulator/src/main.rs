mod depth_camera;
mod geo;
mod landscape;
mod mission;
mod occupancy_map;
mod physics;
mod rgb_camera;
mod terra;
mod vehicle;
mod velocity_controller;
mod voxel_terrain;
mod world;
mod zenoh_bridge;

use bevy::prelude::*;
use depth_camera::TerraDepthCameraPlugin;
use physics::TerraPhysicsPlugin;
use terra::TerraPlugin;
use world::TerraWorldPlugin;
use zorvane_vehicle::VehicleBody;

fn asset_root() -> String {
    std::env::var("ZORVANE_ASSETS")
        .unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/assets").into())
}

fn main() {
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
    mission::install(&mut app);
    let defaults = DefaultPlugins.set(AssetPlugin {
        file_path: asset_root(),
        ..default()
    });
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

    app.insert_resource(vehicle::Vehicles(vehicles))
        .insert_resource(chassis)
        .insert_resource(terra::RoverFleet::from_env().expect("invalid fleet configuration"))
        .add_plugins((
            TerraPhysicsPlugin,
            TerraWorldPlugin {
                config: world::WorldConfig::from_env().expect("invalid Terra world configuration"),
            },
            TerraPlugin { config: drive },
            velocity_controller::TerraVelocityControlPlugin,
            TerraDepthCameraPlugin::default(),
            occupancy_map::TerraOccupancyMapPlugin,
            rgb_camera::TerraRgbCameraPlugin,
            zenoh_bridge::TerraZenohPlugin {
                config: zenoh_bridge::ZenohBridgeConfig::from_env(),
            },
        ));
    if std::env::var("ZORVANE_SMOKE").as_deref() == Ok("1") {
        app.add_systems(Update, smoke_exit);
    }
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
