//! Standalone visual-only GMU tile viewer. No robotics bridge or sensor plugins.
#[path = "../src/gmu/draco.rs"]
mod native_draco;
use bevy::{input::mouse::AccumulatedMouseScroll, prelude::*};
use bevy_3d_tiles::{
    EcefOrigin, P3dtParams, TilePrepareHook, Tiles3dAttach, Tiles3dCamera, Tiles3dConfig,
    Tiles3dPlugin, TilesetCredits, geodesy,
};

// Google Maps' stadium place location, not a surveyed pitch centre.
const LAT: f64 = 38.833673;
const LON: f64 = -77.3161661;
const HELP: &str = "GMU desktop viewer\n\
    Run: cargo run -p zorvane --example gmu_viewer --features gmu-viewer\n\
    Set GOOGLE_MAPS_API_KEY in your environment (Map Tiles API enabled).\n\
    --offline: show a synthetic reference field without making tile requests.\n\
    Arrows: orbit | W/A/S/D: pan | Q/E: lower/raise focus | scroll or +/-: zoom\n\
    Shift: faster | R: reset | Escape: quit\n\
    Initial height is approximate; Q/E adjusts the view, not the data.";

#[derive(Resource)]
struct Viewer {
    key: Option<String>,
}

#[derive(Resource)]
struct Orbit {
    focus: Vec3,
    yaw: f32,
    pitch: f32,
    radius: f32,
}
impl Default for Orbit {
    fn default() -> Self {
        Self {
            focus: Vec3::new(0.0, 140.0, 0.0),
            yaw: 0.0,
            pitch: 0.65,
            radius: 450.0,
        }
    }
}
#[derive(Component)]
struct Attribution;

fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("{HELP}");
        return;
    }
    if args.iter().any(|a| a != "--offline") {
        eprintln!("Unknown argument.\n{HELP}");
        std::process::exit(2);
    }
    // The upstream plugin has an automatic development-source override.
    // Refuse it so this viewer cannot unexpectedly open a second tileset,
    // especially during the explicitly network-free offline check.
    if std::env::var_os("TT_TILES3D").is_some_and(|v| !v.is_empty()) {
        eprintln!("Unset TT_TILES3D before running the GMU viewer.");
        std::process::exit(2);
    }
    let offline = args.iter().any(|a| a == "--offline");
    let key = if offline {
        None
    } else {
        match std::env::var("GOOGLE_MAPS_API_KEY") {
            Ok(key) if !key.trim().is_empty() => Some(key.trim().to_owned()),
            _ => {
                eprintln!("GOOGLE_MAPS_API_KEY is missing.\n{HELP}");
                std::process::exit(2);
            }
        }
    };
    App::new()
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "Zorvane · GMU geographic viewer".into(),
                        resolution: (1280, 800).into(),
                        ..default()
                    }),
                    ..default()
                })
                .set(bevy::log::LogPlugin {
                    // Network error logs can contain credential-bearing URLs. Keep the
                    // prototype quiet; status and setup guidance stay in the window.
                    filter: "off".into(),
                    ..default()
                }),
        )
        .insert_resource(TilePrepareHook(Some(std::sync::Arc::new(|bytes, geo| {
            Box::pin(async move {
                let requests = bevy_3d_tiles_prepare::draco_requests(bytes)?;
                let decoded = requests
                    .iter()
                    .map(|request| native_draco::decode(&request.compressed, &request.attr_ids))
                    .collect::<Result<Vec<_>, _>>()?;
                bevy_3d_tiles_prepare::prepare_tile_extracting_with_draco(bytes, geo, decoded)
            })
        }))))
        .add_plugins(Tiles3dPlugin)
        .insert_resource(Tiles3dConfig {
            max_concurrent_loads: 4,
            max_spawns_per_frame: 2,
            memory_budget_bytes: 256 * 1024 * 1024,
            detail_falloff_m: 1500.0,
            ..default()
        })
        .insert_resource(Viewer { key })
        .init_resource::<Orbit>()
        .insert_resource(ClearColor(Color::srgb(0.08, 0.11, 0.15)))
        .add_systems(Startup, setup)
        .add_systems(Update, (control_camera, show_credits, capture_check))
        .run();
}

// Optional verification capture of our own render target. Includes attribution
// and never exports mesh data or sensor imagery.
fn capture_check(time: Res<Time>, mut commands: Commands, mut captured: Local<bool>) {
    if !*captured && time.elapsed_secs() >= 60.0 {
        *captured = true;
        if let Some(path) = std::env::var_os("GMU_VIEWER_CAPTURE_PATH") {
            commands
                .spawn(bevy::render::view::screenshot::Screenshot::primary_window())
                .observe(bevy::render::view::screenshot::save_to_disk(
                    std::path::PathBuf::from(path),
                ));
        }
    }
}

fn setup(
    mut commands: Commands,
    viewer: Res<Viewer>,
    mut attach: MessageWriter<Tiles3dAttach>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut fonts: ResMut<Assets<Font>>,
) {
    let font = fonts.add(Font::from_bytes(
        include_bytes!("../assets/fonts/FiraMono-Regular.ttf").to_vec(),
    ));
    commands.insert_resource(EcefOrigin {
        world_from_ecef: Some(geodesy::world_from_ecef(LAT, LON, 0.0)),
    });
    commands.spawn((Camera3d::default(), Transform::default(), Tiles3dCamera));
    if let Some(key) = &viewer.key {
        let anchor = commands
            .spawn((Transform::IDENTITY, Visibility::default()))
            .id();
        attach.write(Tiles3dAttach {
            anchor,
            url: "https://tile.googleapis.com/v1/3dtiles/root.json".into(),
            label: "George Mason Stadium".into(),
            p3dt: Some(P3dtParams {
                api_key: key.clone(),
                daily_request_cap: 10,
            }),
            ..default()
        });
    } else {
        commands.spawn((
            Mesh3d(meshes.add(Plane3d::default().mesh().size(68.0, 105.0))),
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color: Color::srgb(0.12, 0.38, 0.22),
                unlit: true,
                ..default()
            })),
            Transform::from_xyz(0.0, 140.0, 0.0),
        ));
    }
    commands.spawn((Text::new(format!("GMU / GEORGE MASON STADIUM\n{}\nArrows orbit · WASD pan · Q/E height · +/- or scroll zoom · R reset · Esc exit",
        if viewer.key.is_some() { "LIVE TILE VIEWER · visual only" } else { "OFFLINE CHECK · synthetic 105 × 68 m reference, not GMU imagery" })),
        TextFont { font: bevy::text::FontSource::Handle(font.clone()), font_size: bevy::text::FontSize::Px(16.0), ..default() }, TextColor(Color::WHITE),
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.85)),
        Node { position_type: PositionType::Absolute, top: px(16), left: px(16), right: px(16), padding: UiRect::all(px(12)), ..default() }));
    commands.spawn((
        Attribution,
        Text::new(""),
        TextFont {
            font: bevy::text::FontSource::Handle(font.clone()),
            font_size: bevy::text::FontSize::Px(16.0),
            ..default()
        },
        TextColor(Color::WHITE),
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.9)),
        Node {
            position_type: PositionType::Absolute,
            bottom: px(8),
            left: px(8),
            right: px(8),
            padding: UiRect::all(px(10)),
            ..default()
        },
    ));
}

fn control_camera(
    keys: Res<ButtonInput<KeyCode>>,
    scroll: Res<AccumulatedMouseScroll>,
    time: Res<Time>,
    mut orbit: ResMut<Orbit>,
    mut cameras: Query<&mut Transform, With<Tiles3dCamera>>,
    mut exit: MessageWriter<AppExit>,
) {
    if keys.just_pressed(KeyCode::Escape) {
        exit.write(AppExit::Success);
    }
    if keys.just_pressed(KeyCode::KeyR) {
        *orbit = Orbit::default();
    }
    let dt = time.delta_secs().min(0.1);
    let axis =
        |positive, negative| f32::from(keys.pressed(positive)) - f32::from(keys.pressed(negative));
    orbit.yaw += axis(KeyCode::ArrowRight, KeyCode::ArrowLeft) * dt;
    orbit.pitch = (orbit.pitch + axis(KeyCode::ArrowUp, KeyCode::ArrowDown) * dt).clamp(0.08, 1.5);
    let speed = if keys.pressed(KeyCode::ShiftLeft) {
        200.0
    } else {
        40.0
    };
    orbit.focus += Vec3::new(
        axis(KeyCode::KeyD, KeyCode::KeyA),
        axis(KeyCode::KeyE, KeyCode::KeyQ),
        axis(KeyCode::KeyS, KeyCode::KeyW),
    ) * speed
        * dt;
    let zoom = axis(KeyCode::Equal, KeyCode::Minus) * dt + scroll.delta.y.clamp(-10.0, 10.0) * 0.05;
    orbit.radius = (orbit.radius * (-zoom).exp()).clamp(10.0, 2500.0);
    let offset = Vec3::new(
        orbit.yaw.sin() * orbit.pitch.cos(),
        orbit.pitch.sin(),
        orbit.yaw.cos() * orbit.pitch.cos(),
    ) * orbit.radius;
    for mut camera in &mut cameras {
        *camera =
            Transform::from_translation(orbit.focus + offset).looking_at(orbit.focus, Vec3::Y);
    }
}

fn show_credits(
    credits: Res<TilesetCredits>,
    viewer: Res<Viewer>,
    time: Res<Time>,
    mut labels: Query<&mut Text, With<Attribution>>,
) {
    let message = if viewer.key.is_none() {
        "Offline reference · no Google requests".into()
    } else if credits.google_visible {
        format!("Google Maps\n{}", credits.lines.join(" · "))
    } else if time.elapsed_secs() > 30.0 {
        "Google Maps · No visible tiles yet. Check internet, API key restrictions, Map Tiles API, billing and quota. Try R to reset.".into()
    } else {
        "Google Maps · Loading 3D tiles…".into()
    };
    for mut text in &mut labels {
        if text.0 != message {
            text.0.clone_from(&message);
        }
    }
}
