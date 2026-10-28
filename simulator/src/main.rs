mod bridge;
mod control_queue;
mod cutscene;
mod depth_camera;
mod fpv;
mod lobby;
mod mason_field;
mod menu;
mod next_competition;
mod object_detection;
mod observability;
mod observation_capture;
mod occupancy_map;
mod online;
mod online_menu;
mod physics;
mod rgb_camera;
mod robotics_pose;
mod session;
mod settings;
mod spectator;
mod terra;
mod vehicle;
mod velocity_controller;
mod world;

use bevy::prelude::*;
use depth_camera::TerraDepthCameraPlugin;
use physics::TerraPhysicsPlugin;
use terra::TerraPlugin;
use world::TerraWorldPlugin;

fn asset_root() -> String {
    if let Ok(path) = std::env::var("ZORVANE_ASSETS") {
        return path;
    }
    packaged_assets().unwrap_or_else(|| concat!(env!("CARGO_MANIFEST_DIR"), "/assets").into())
}

/// A packaged build carries its assets: beside the executable in the Windows
/// and Linux zips, or in `Contents/Resources` of the macOS app bundle.
fn packaged_assets() -> Option<String> {
    let exe = std::env::current_exe().ok()?.canonicalize().ok()?;
    let dir = exe.parent()?;
    [dir.join("assets"), dir.join("../Resources/assets")]
        .into_iter()
        .find(|path| path.join("cutscene/next_intro.json").is_file())
        .and_then(|path| path.canonicalize().ok())
        .map(|path| path.to_string_lossy().into_owned())
}

fn main() {
    settings::wait_for_listen_port();
    observability::initialize();
    observation_capture::initialize();
    // CrabTrapBot is the NEXT rover; ZORVANE_VEHICLE still chooses explicitly.
    let vehicles = if std::env::var_os("ZORVANE_VEHICLE").is_none() {
        zorvane_vehicle::VehicleRegistry::from_id(Some("crab-trap-bot"))
    } else {
        zorvane_vehicle::VehicleRegistry::from_env()
    }
    .expect("invalid ZORVANE_VEHICLE");
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
    next_competition::install(&mut app);
    mason_field::install(&mut app);
    cutscene::install(&mut app);
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

    // With the menu, the field spawns when Start is pressed.
    let world_config = if session::interactive_menu() {
        world::WorldConfig::default()
    } else {
        world::WorldConfig::from_env()
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
            object_detection::TerraObjectDetectionPlugin,
            bridge::SimulatorBridgePlugin {
                config: bridge::BridgeConfig::from_env(),
            },
        ));
    install_fpv(&mut app);
    online_menu::install_headless_join(&mut app);
    if session::interactive_menu() {
        app.add_plugins(menu::MenuPlugin);
    }
    if std::env::var("ZORVANE_SMOKE").as_deref() == Ok("1") {
        app.add_systems(Update, smoke_exit);
    }
    app.run();
}

/// WebRTC FPV for dashboards, on the simulator's own Zenoh router. Off with the
/// bridge (`TERRA_ZENOH=0`) or with `ZORVANE_FPV=0`.
fn install_fpv(app: &mut App) {
    let config = bridge::BridgeConfig::from_env();
    if !config.enabled || std::env::var("ZORVANE_FPV").as_deref() == Ok("0") {
        return;
    }
    let (endpoint, credentials) = if !config.listen.is_empty() {
        (config.listen.replace("0.0.0.0", "127.0.0.1"), None)
    } else if let Some(remote) = config.connect.first() {
        (
            remote.clone(),
            config.user.clone().zip(config.password.clone()),
        )
    } else {
        return;
    };
    let (user, password) = credentials.unzip();
    app.add_plugins(fpv::TerraFpvPlugin {
        endpoint,
        prefix: config.prefix,
        user,
        password,
    });
}

/// Headless boot check. Exits after a few frames once the world schedule is running.
fn smoke_exit(mut frames: Local<u32>, mut exit: MessageWriter<AppExit>) {
    *frames += 1;
    if *frames >= 3 {
        eprintln!("Zorvane smoke: world started");
        exit.write(AppExit::Success);
    }
}

mod outdoor;
