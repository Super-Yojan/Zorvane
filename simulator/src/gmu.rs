//! Spectator-only Google scenery for GMU NEXT and elevation worlds.
//! Google geometry is render-only: spectator layer 1, never sensor layer 0.
mod credentials;
mod draco;
use crate::world::{WorldCamera, WorldConfig, WorldElement};
use bevy::{
    camera::visibility::RenderLayers,
    math::{DMat4, DQuat},
    prelude::*,
    tasks::{IoTaskPool, Task, block_on, futures_lite::future},
};
use bevy_3d_tiles::{
    EcefOrigin, P3dtParams, TileGeometry, TilePrepareHook, Tiles3dAttach, Tiles3dCamera,
    Tiles3dConfig, Tiles3dDetach, Tiles3dPlugin, Tiles3dSet, TilesetCredits,
};

const GOOGLE_LAYER: usize = 1;
pub fn requested() -> bool {
    matches!(
        std::env::var("TERRA_NEXT_GMU").as_deref(),
        Ok("1") | Ok("true") | Ok("TRUE")
    )
}
#[derive(Resource)]
struct GeoidOffset(f64);
impl Default for GeoidOffset {
    fn default() -> Self {
        Self(-31.86)
    }
}

fn spectator_layers() -> RenderLayers {
    RenderLayers::from_layers(&[0, GOOGLE_LAYER])
}

/// Independent nominal placement; no Google mesh sampling, tracing or colliders.
/// Center/heading: OpenStreetMap way 233194670 (ODbL, © OSM contributors).
/// Height: USGS 127.60157 m + NOAA GEOID18 -31.86 m, approximately 95.74 m.
#[derive(Resource, Clone, Copy)]
struct Placement {
    latitude: f64,
    longitude: f64,
    elevation: f64,
    heading: f64,
}
impl Default for Placement {
    fn default() -> Self {
        Self {
            latitude: 38.833755825,
            longitude: -77.31659325,
            elevation: 95.74,
            heading: 4.81259,
        }
    }
}
impl Placement {
    fn from_env() -> Result<Self, String> {
        let mut p = Self::default();
        for (name, target) in [
            ("TERRA_GMU_LAT", &mut p.latitude),
            ("TERRA_GMU_LON", &mut p.longitude),
            ("TERRA_GMU_ELEVATION", &mut p.elevation),
            ("TERRA_GMU_HEADING", &mut p.heading),
        ] {
            if let Ok(value) = std::env::var(name) {
                *target = value
                    .parse()
                    .map_err(|_| format!("{name} must be a finite number"))?;
            }
        }
        p.validate()?;
        Ok(p)
    }
    fn validate(&self) -> Result<(), String> {
        if !self.latitude.is_finite()
            || !(-85.0..=85.0).contains(&self.latitude)
            || !self.longitude.is_finite()
            || !(-180.0..=180.0).contains(&self.longitude)
            || !self.elevation.is_finite()
            || !(-500.0..=9000.0).contains(&self.elevation)
            || !self.heading.is_finite()
            || !(0.0..360.0).contains(&self.heading)
        {
            return Err("GMU placement requires latitude [-85,85], longitude [-180,180], ellipsoid elevation [-500,9000] m and heading [0,360) degrees".into());
        }
        Ok(())
    }
    fn world_from_ecef(self) -> DMat4 {
        // Heading is clockwise from north for the pitch's -Z direction.
        DMat4::from_quat(DQuat::from_rotation_y(self.heading.to_radians()))
            * bevy_3d_tiles::geodesy::world_from_ecef(self.latitude, self.longitude, self.elevation)
    }
}

fn placement_for_world(
    config: &WorldConfig,
    stadium: Placement,
    patch: Option<&crate::geo::GeoPatch>,
    geoid_offset: f64,
) -> Option<Placement> {
    if config.gmu_stadium {
        return Some(stadium);
    }
    if config.scene == crate::world::SceneKind::Tiles {
        return patch.map(|p| Placement {
            latitude: p.anchor.latitude,
            longitude: p.anchor.longitude,
            elevation: f64::from(p.origin_elevation) + geoid_offset,
            heading: 0.0,
        });
    }
    None
}

#[derive(Resource)]
pub(crate) struct Access {
    key: Option<String>,
    pub status: String,
    offline: bool,
    attempted: bool,
    pending: Option<Task<Result<String, String>>>,
}
impl Access {
    fn new(key: Option<String>, offline: bool) -> Self {
        let status = if offline {
            "Offline practice field"
        } else if key.is_some() {
            "Connected. GMU imagery loads when you start."
        } else {
            "Select GMU to connect using Google Cloud."
        }
        .into();
        Self {
            key,
            status,
            offline,
            attempted: false,
            pending: None,
        }
    }
}
#[derive(Resource, Default)]
struct Stream {
    camera: Option<Entity>,
    anchor: Option<Entity>,
    started: f64,
    retry: bool,
}
#[derive(Component)]
struct GmuLabel;
#[derive(Resource, Default)]
struct AttributionFont(Handle<Font>);

pub fn install(app: &mut App) -> Result<(), String> {
    if std::env::var_os("TT_TILES3D").is_some_and(|v| !v.is_empty()) {
        return Err("Unset TT_TILES3D before using the GMU-enabled simulator".into());
    }
    let placement = Placement::from_env()?;
    let geoid_offset = std::env::var("TERRA_SCENERY_GEOID_OFFSET")
        .ok()
        .map(|s| s.parse::<f64>())
        .transpose()
        .map_err(|_| "TERRA_SCENERY_GEOID_OFFSET must be a finite number of metres")?
        .unwrap_or(-31.86);
    if !geoid_offset.is_finite() || geoid_offset.abs() > 120.0 {
        return Err("TERRA_SCENERY_GEOID_OFFSET must be between -120 and 120 metres".into());
    }
    let offline = std::env::var("TERRA_GMU_OFFLINE").as_deref() == Ok("1")
        || std::env::var("TERRA_HEADLESS").as_deref() == Ok("1");
    let key = (!offline)
        .then(|| std::env::var("GOOGLE_MAPS_API_KEY").ok())
        .flatten()
        .filter(|k| !k.trim().is_empty())
        .map(|k| k.trim().to_owned());
    let font = app
        .world_mut()
        .resource_mut::<Assets<Font>>()
        .add(Font::from_bytes(
            include_bytes!("../assets/fonts/FiraMono-Regular.ttf").to_vec(),
        ));
    app.insert_resource(AttributionFont(font))
        .insert_resource(placement)
        .insert_resource(GeoidOffset(geoid_offset))
        .insert_resource(Access::new(key, offline))
        .init_resource::<Stream>()
        .insert_resource(TilePrepareHook(Some(std::sync::Arc::new(|bytes, geo| {
            Box::pin(async move {
                let decoded = bevy_3d_tiles_prepare::draco_requests(bytes)?
                    .iter()
                    .map(|r| draco::decode(&r.compressed, &r.attr_ids))
                    .collect::<Result<Vec<_>, _>>()?;
                bevy_3d_tiles_prepare::prepare_tile_extracting_with_draco(bytes, geo, decoded)
            })
        }))))
        .insert_resource(Tiles3dConfig {
            max_concurrent_loads: 4,
            max_spawns_per_frame: 2,
            memory_budget_bytes: 256 * 1024 * 1024,
            detail_falloff_m: 1500.0,
            ..default()
        })
        .add_plugins(Tiles3dPlugin)
        .add_systems(
            Update,
            (resolve_access, sync_scene)
                .chain()
                .after(crate::world::finish_rebuild)
                .before(Tiles3dSet::Receive),
        )
        .add_systems(
            Update,
            isolate_tiles
                .after(Tiles3dSet::Receive)
                .before(Tiles3dSet::Drive),
        )
        .add_systems(
            Update,
            (
                (sync_simulation_surfaces, show_status)
                    .chain()
                    .after(Tiles3dSet::Drive),
                switch_view,
                capture_check,
            ),
        );
    Ok(())
}

fn resolve_access(
    config: Res<WorldConfig>,
    session: Option<Res<crate::session::SimSession>>,
    mut access: ResMut<Access>,
    mut stream: ResMut<Stream>,
    retry: Query<&Interaction, (Changed<Interaction>, With<crate::menu::GmuRetryButton>)>,
) {
    if access.offline {
        return;
    }
    if let Some(task) = access.pending.as_mut()
        && let Some(result) = block_on(future::poll_once(task))
    {
        access.pending = None;
        match result {
            Ok(key) => {
                access.key = Some(key);
                access.status = "Connected. GMU imagery loads when you start.".into();
            }
            Err(message) => {
                access.status = message;
            }
        }
    }
    if retry.iter().any(|i| *i == Interaction::Pressed) && access.pending.is_none() {
        access.attempted = false;
        stream.retry = access.key.is_some();
    }
    let selected = config.uses_google_scenery()
        || session.is_some_and(|s| {
            matches!(
                s.world,
                crate::session::WorldChoice::Gmu | crate::session::WorldChoice::Tiles
            )
        });
    if selected && access.key.is_none() && !access.attempted {
        access.attempted = true;
        access.status = "Connecting to Google Cloud...".into();
        access.pending = Some(IoTaskPool::get().spawn(async { credentials::existing_key() }));
    }
}

fn sync_simulation_surfaces(
    mut commands: Commands,
    config: Res<WorldConfig>,
    credits: Res<TilesetCredits>,
    surfaces: Query<
        (Entity, Option<&RenderLayers>),
        Or<(
            With<crate::next_competition::PitchSurface>,
            With<crate::world::Ground>,
            With<crate::geo::GeoTint>,
            With<crate::geo::GeoObstacle>,
        )>,
    >,
) {
    // Keep authored geometry available to RGB/depth, even when the real pitch
    // imagery replaces it for the spectator. No Google geometry is extracted.
    let layer = if config.uses_google_scenery() && credits.google_visible {
        2
    } else {
        0
    };
    let expected = RenderLayers::layer(layer);
    for (entity, current) in &surfaces {
        if current != Some(&expected) {
            commands.entity(entity).insert(expected.clone());
        }
    }
}

// Runs after receipt/deferred spawns, before the selection pass reveals tiles.
// Layer assignment is per primitive: Bevy does not inherit RenderLayers.
pub(crate) fn isolate_tiles(mut commands: Commands, tiles: Query<Entity, Added<TileGeometry>>) {
    for entity in &tiles {
        commands.entity(entity).insert((
            RenderLayers::layer(GOOGLE_LAYER),
            bevy::light::NotShadowCaster,
        ));
    }
}

fn sync_scene(
    mut commands: Commands,
    config: Res<WorldConfig>,
    placement: Res<Placement>,
    geoid_offset: Res<GeoidOffset>,
    patch: Option<Res<crate::geo::GeoPatch>>,
    font: Res<AttributionFont>,
    access: Res<Access>,
    cameras: Query<Entity, With<WorldCamera>>,
    mut stream: ResMut<Stream>,
    time: Res<Time<Real>>,
    mut attach: MessageWriter<Tiles3dAttach>,
    mut detach: MessageWriter<Tiles3dDetach>,
) {
    let frame = placement_for_world(&config, *placement, patch.as_deref(), geoid_offset.0);
    let camera = frame.and_then(|_| cameras.single().ok());
    if stream.retry {
        stream.retry = false;
        stream.started = time.elapsed_secs_f64();
        if let Some(anchor) = stream.anchor.take() {
            detach.write(Tiles3dDetach { anchor });
            if let Ok(mut entity) = commands.get_entity(anchor) {
                entity.despawn();
            }
        }
    }
    if camera != stream.camera {
        if let Some(anchor) = stream.anchor.take() {
            detach.write(Tiles3dDetach { anchor });
            if let Ok(mut entity) = commands.get_entity(anchor) {
                entity.despawn();
            }
        }
        stream.camera = camera;
        if let Some(camera) = camera {
            stream.started = time.elapsed_secs_f64();
            commands
                .entity(camera)
                .insert((spectator_layers(), Tiles3dCamera));
            commands.insert_resource(EcefOrigin {
                world_from_ecef: frame.map(Placement::world_from_ecef),
            });
            spawn_status(&mut commands, &font);
        }
    }
    if camera.is_none() || stream.anchor.is_some() {
        return;
    }
    if let Some(key) = &access.key {
        let anchor = commands
            .spawn((
                Name::new("GMU visual scenery"),
                WorldElement,
                Transform::IDENTITY,
                Visibility::default(),
            ))
            .id();
        stream.anchor = Some(anchor);
        attach.write(Tiles3dAttach {
            anchor,
            url: "https://tile.googleapis.com/v1/3dtiles/root.json".into(),
            label: "Map world spectator scenery".into(),
            p3dt: Some(P3dtParams {
                api_key: key.clone(),
                daily_request_cap: 10,
            }),
            ..default()
        });
    }
}

fn spawn_status(commands: &mut Commands, font: &AttributionFont) {
    commands.spawn((
        WorldElement,
        GmuLabel,
        GlobalZIndex(100),
        Text::new(""),
        TextFont {
            font: bevy::text::FontSource::Handle(font.0.clone()),
            font_size: bevy::text::FontSize::Px(14.0),
            ..default()
        },
        TextColor(Color::WHITE),
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.9)),
        Node {
            position_type: PositionType::Absolute,
            bottom: px(8),
            left: px(8),
            right: px(8),
            padding: UiRect::all(px(8)),
            ..default()
        },
    ));
}

fn show_status(
    credits: Res<TilesetCredits>,
    config: Res<WorldConfig>,
    access: Res<Access>,
    stream: Res<Stream>,
    time: Res<Time<Real>>,
    mut labels: Query<&mut Text, With<GmuLabel>>,
) {
    let scenery = if access.key.is_none() {
        access.status.clone()
    } else if credits.google_visible {
        format!("Google Maps | {}", credits.lines.join(" | "))
    } else if time.elapsed_secs_f64() - stream.started > 30.0 {
        "Google Maps | No visible scenery yet: check connection, key, billing, quota and placement"
            .into()
    } else {
        "Google Maps | Loading scenery...".into()
    };
    let title = if config.scene == crate::world::SceneKind::Tiles {
        "Elevation world"
    } else {
        "NEXT - GMU Stadium"
    };
    let source = if config.scene == crate::world::SceneKind::Tiles {
        "Robot sensors and physics: elevation-derived terrain (Terrarium)"
    } else {
        "Competition: Zorvane | Placement: © OpenStreetMap contributors (openstreetmap.org/copyright)"
    };
    let message = format!(
        "{title} | F6: overview / follow | approximate alignment\n{source}\nGoogle scenery: spectator only\n{scenery}"
    );
    for mut label in &mut labels {
        if label.0 != message {
            label.0.clone_from(&message);
        }
    }
}

pub(crate) fn switch_view(
    keys: Res<ButtonInput<KeyCode>>,
    mut config: ResMut<WorldConfig>,
    buttons: Query<&Interaction, (Changed<Interaction>, With<crate::menu::GmuViewButton>)>,
) {
    if config.uses_google_scenery()
        && (keys.just_pressed(KeyCode::F6) || buttons.iter().any(|i| *i == Interaction::Pressed))
    {
        config.follow_rover = !config.follow_rover;
        config.camera_offset = if config.follow_rover {
            if config.scene == crate::world::SceneKind::Tiles {
                Vec3::new(12.0, 18.0, 20.0)
            } else {
                Vec3::new(0.0, 3.2, 5.5)
            }
        } else {
            Vec3::new(100.0, 160.0, 180.0)
        };
    }
}

// Optional screenshot of the spectator render only, including attribution.
fn capture_check(time: Res<Time<Real>>, mut commands: Commands, mut captured: Local<bool>) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use avian3d::prelude::{Collider, RigidBody};

    #[test]
    fn streamed_geometry_never_reaches_sensors_or_physics() {
        let mut app = App::new();
        app.add_systems(Update, isolate_tiles);
        let entity = app
            .world_mut()
            .spawn((bevy_3d_tiles::TileGeometry { set_id: 1 }, Mesh3d::default()))
            .id();
        app.update();
        let tile = app.world().entity(entity);
        let layers = tile.get::<RenderLayers>().unwrap();
        assert!(!layers.intersects(&RenderLayers::from_layers(&[0, 2])));
        assert!(layers.intersects(&spectator_layers()));
        assert!(tile.contains::<bevy::light::NotShadowCaster>());
        assert!(!tile.contains::<Collider>());
        assert!(!tile.contains::<RigidBody>());
    }

    #[test]
    fn geographic_anchor_and_heading_preserve_local_metres() {
        let placement = Placement::default();
        let (x, y, z) = bevy_3d_tiles::geodesy::geodetic_to_ecef(
            placement.latitude,
            placement.longitude,
            placement.elevation,
        );
        let matrix = placement.world_from_ecef();
        assert!(
            matrix
                .transform_point3(bevy::math::DVec3::new(x, y, z))
                .length()
                < 1e-6
        );
        assert!((matrix.transform_vector3(bevy::math::DVec3::X).length() - 1.0).abs() < 1e-10);
        assert!(
            Placement {
                elevation: f64::NAN,
                ..placement
            }
            .validate()
            .is_err()
        );
    }
    #[test]
    fn menu_restart_and_switch_detach_old_scenery_and_preserve_competition() {
        use crate::{
            session::{Phase, RebuildWorld, SimSession, WorldChoice},
            world::TerraWorldPlugin,
        };
        let mut app = App::new();
        app.insert_resource(Assets::<Mesh>::default())
            .insert_resource(Assets::<StandardMaterial>::default())
            .insert_resource(Time::<()>::default())
            .insert_resource(Time::<Real>::default())
            .insert_resource(SimSession::at_menu(WorldChoice::Gmu, 1))
            .insert_resource(Placement::default())
            .init_resource::<AttributionFont>()
            .init_resource::<GeoidOffset>()
            .insert_resource(Access::new(Some("test-only-not-a-real-key".into()), false))
            .init_resource::<Stream>()
            .add_message::<Tiles3dAttach>()
            .add_message::<Tiles3dDetach>()
            .add_plugins(TerraWorldPlugin::default())
            .add_systems(Update, sync_scene.after(crate::world::finish_rebuild));
        app.update();
        assert!(app.world().resource::<Stream>().anchor.is_none());
        app.world_mut().resource_mut::<SimSession>().phase = Phase::Playing;
        app.insert_resource(RebuildWorld);
        app.update();
        let first = app.world().resource::<Stream>().anchor.unwrap();
        let mut attaches = bevy::ecs::message::MessageCursor::<Tiles3dAttach>::default();
        assert_eq!(
            attaches
                .read(app.world().resource::<Messages<Tiles3dAttach>>())
                .count(),
            1
        );
        let world = app.world_mut();
        assert_eq!(
            world
                .query::<&crate::next_competition::TennisBall>()
                .iter(world)
                .count(),
            6
        );
        assert_eq!(
            world
                .query::<&crate::next_competition::Bucket>()
                .iter(world)
                .count(),
            20
        );
        app.insert_resource(RebuildWorld);
        app.update();
        let second = app.world().resource::<Stream>().anchor.unwrap();
        assert_ne!(first, second);
        assert!(app.world().get_entity(first).is_err());
        let mut detaches = bevy::ecs::message::MessageCursor::<Tiles3dDetach>::default();
        assert_eq!(
            detaches
                .read(app.world().resource::<Messages<Tiles3dDetach>>())
                .map(|m| m.anchor)
                .collect::<Vec<_>>(),
            vec![first]
        );
        app.world_mut().resource_mut::<SimSession>().world = WorldChoice::Next;
        app.insert_resource(RebuildWorld);
        app.update();
        assert!(app.world().resource::<Stream>().anchor.is_none());
        assert!(app.world().get_entity(second).is_err());
        assert_eq!(
            detaches
                .read(app.world().resource::<Messages<Tiles3dDetach>>())
                .map(|m| m.anchor)
                .collect::<Vec<_>>(),
            vec![second]
        );
        let world = app.world_mut();
        assert_eq!(world.query::<&GmuLabel>().iter(world).count(), 0);
        assert_eq!(
            world
                .query::<&crate::next_competition::Bucket>()
                .iter(world)
                .count(),
            20
        );
        assert_eq!(world.query::<&Tiles3dCamera>().iter(world).count(), 0);
    }
    #[test]
    fn overview_toggle_returns_to_follow_without_moving_competition() {
        let mut app = App::new();
        app.insert_resource(WorldConfig::for_choice(crate::session::WorldChoice::Gmu))
            .init_resource::<ButtonInput<KeyCode>>()
            .add_systems(Update, switch_view);
        for follow in [true, false] {
            let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            keys.reset_all();
            keys.press(KeyCode::F6);
            app.update();
            let config = app.world().resource::<WorldConfig>();
            assert_eq!(config.follow_rover, follow);
            assert!(config.gmu_stadium);
            assert_eq!(config.size, crate::next_competition::WORLD_SIZE);
        }
    }
    #[test]
    fn real_field_replaces_only_spectator_surface_and_recovers_offline() {
        let mut app = App::new();
        app.insert_resource(WorldConfig::for_choice(crate::session::WorldChoice::Gmu))
            .init_resource::<TilesetCredits>()
            .add_systems(Update, sync_simulation_surfaces);
        let surface = app
            .world_mut()
            .spawn(crate::next_competition::PitchSurface)
            .id();
        app.update();
        assert!(
            app.world()
                .get::<RenderLayers>(surface)
                .unwrap()
                .intersects(&spectator_layers())
        );
        app.world_mut()
            .resource_mut::<TilesetCredits>()
            .google_visible = true;
        app.update();
        let layers = app.world().get::<RenderLayers>(surface).unwrap();
        assert!(!layers.intersects(&spectator_layers()));
        assert!(layers.intersects(&RenderLayers::from_layers(&[0, 2])));
        app.world_mut()
            .resource_mut::<TilesetCredits>()
            .google_visible = false;
        app.update();
        assert!(
            app.world()
                .get::<RenderLayers>(surface)
                .unwrap()
                .intersects(&spectator_layers())
        );
        app.world_mut()
            .resource_mut::<TilesetCredits>()
            .google_visible = true;
        *app.world_mut().resource_mut::<WorldConfig>() =
            WorldConfig::for_choice(crate::session::WorldChoice::Next);
        app.update();
        assert!(
            app.world()
                .get::<RenderLayers>(surface)
                .unwrap()
                .intersects(&RenderLayers::layer(0))
        );
    }
    #[test]
    fn elevation_scenery_uses_terrain_origin_and_keeps_collision_blocks_for_sensors() {
        use crate::geo::{GeoAnchor, GeoPatch, PatchCell};
        let patch = GeoPatch {
            anchor: GeoAnchor::try_new(38.8297, -77.3075, 15).unwrap(),
            origin_elevation: 132.5,
            step: 4.0,
            cells: vec![PatchCell {
                bevy_x: 10.0,
                bevy_z: 0.0,
                relative_height: 2.0,
                obstacle: true,
                block_height: 2.0,
            }],
        };
        let config = WorldConfig::for_choice(crate::session::WorldChoice::Tiles);
        assert!(config.uses_google_scenery());
        let placement =
            placement_for_world(&config, Placement::default(), Some(&patch), -31.86).unwrap();
        assert_eq!(placement.heading, 0.0);
        assert!((placement.elevation - 100.64).abs() < 1e-5);
        let (x, y, z) = bevy_3d_tiles::geodesy::geodetic_to_ecef(38.8297, -77.3075, 100.64);
        assert!(
            placement
                .world_from_ecef()
                .transform_point3(bevy::math::DVec3::new(x, y, z))
                .length()
                < 1e-6
        );
        let mut app = App::new();
        app.insert_resource(config)
            .insert_resource(patch)
            .insert_resource(Assets::<Mesh>::default())
            .insert_resource(Assets::<StandardMaterial>::default())
            .insert_resource(TilesetCredits {
                google_visible: true,
                ..default()
            })
            .add_systems(Startup, crate::geo::spawn_patch)
            .add_systems(Update, sync_simulation_surfaces);
        app.update();
        let world = app.world_mut();
        let obstacle = world
            .query_filtered::<Entity, With<crate::geo::GeoObstacle>>()
            .single(world)
            .unwrap();
        let entity = world.entity(obstacle);
        assert!(entity.contains::<Collider>());
        assert!(entity.contains::<RigidBody>());
        let layers = entity.get::<RenderLayers>().unwrap();
        assert!(!layers.intersects(&spectator_layers()));
        assert!(layers.intersects(&RenderLayers::from_layers(&[0, 2])));
        assert_eq!(
            world
                .query::<&crate::next_competition::NextField>()
                .iter(world)
                .count(),
            0
        );
    }
}
