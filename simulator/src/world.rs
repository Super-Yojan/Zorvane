//! Seeded practice world with roads, buildings, forests, water and voxel hills.
use crate::{landscape::LandscapeConfig, session::SimSession, terra::Rover};
use avian3d::prelude::*;
use bevy::prelude::*;

/// Which authored scene `WorldConfig` describes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SceneKind {
    #[default]
    Town,
    Tiles,
    Next,
    Mission,
}

#[derive(Default)]
pub struct TerraWorldPlugin {
    pub config: WorldConfig,
}

impl Plugin for TerraWorldPlugin {
    fn build(&self, app: &mut App) {
        let menu = app
            .world()
            .get_resource::<SimSession>()
            .is_some_and(|session| session.phase == crate::session::Phase::Menu);
        let mut config = self.config.clone();
        let tiles_requested = config.tiles.enabled && !menu;
        if tiles_requested {
            // The practice town is replaced only after a tile patch actually loads.
            config.landscape.enabled = false;
        }
        config
            .validate()
            .expect("invalid Terra world configuration");
        let patch = if tiles_requested {
            match crate::geo::load_patch(&config.tiles, config.size) {
                Ok(patch) => {
                    eprintln!(
                        "Terra tiles: {} cells around {:.5}, {:.5} zoom {} ({} obstacles, origin elevation {:.1} m)",
                        patch.cells.len(),
                        patch.anchor.latitude,
                        patch.anchor.longitude,
                        patch.anchor.zoom,
                        patch.cells.iter().filter(|cell| cell.obstacle).count(),
                        patch.origin_elevation
                    );
                    Some(patch)
                }
                Err(error) => {
                    eprintln!("Terra tiles: {error}. Continuing with the flat practice world.");
                    config.tiles.enabled = false;
                    config.scene = SceneKind::Town;
                    config.landscape = self.config.landscape.clone();
                    if config.validate().is_err() {
                        config.landscape.enabled = false;
                    }
                    None
                }
            }
        } else {
            None
        };
        crate::landscape::install(app, &config, menu);
        if patch.is_some() {
            app.add_systems(Startup, crate::geo::spawn_patch);
        }
        if menu {
            app.add_systems(
                Update,
                (
                    apply_session,
                    generate_world,
                    crate::landscape::generate_landscape,
                    crate::next_competition::spawn_field,
                    crate::geo::spawn_patch,
                    finish_rebuild,
                )
                    .chain()
                    .run_if(resource_exists::<crate::session::RebuildWorld>),
            );
        } else {
            app.add_systems(Startup, generate_world);
        }
        app.insert_resource(config)
            .insert_resource(ClearColor(Color::srgb(0.65, 0.78, 0.88)))
            .insert_resource(GlobalAmbientLight {
                brightness: 220.0,
                ..default()
            })
            .add_systems(PostUpdate, follow_rover.before(TransformSystems::Propagate));
        if let Some(patch) = patch {
            app.insert_resource(patch);
        }
    }
}

#[derive(Resource, Clone)]
pub struct WorldConfig {
    /// Square ground width in metres, centred at the rover's spawn origin.
    pub size: f32,
    /// Distance between grid lines in metres.
    pub grid_spacing: f32,
    /// Offset from the rover in world coordinates; camera does not rotate with it.
    pub camera_offset: Vec3,
    /// Exponential camera response rate per second.
    pub camera_response: f32,
    pub follow_rover: bool,
    pub show_grid: bool,
    pub landscape: LandscapeConfig,
    pub tiles: crate::geo::GeoTileConfig,
    pub scene: SceneKind,
}

impl Default for WorldConfig {
    fn default() -> Self {
        Self {
            size: 100.0,
            grid_spacing: 5.0,
            camera_offset: Vec3::new(12.0, 18.0, 20.0),
            camera_response: 5.0,
            follow_rover: true,
            show_grid: false,
            landscape: LandscapeConfig::default(),
            tiles: crate::geo::GeoTileConfig::default(),
            scene: SceneKind::Town,
        }
    }
}

impl WorldConfig {
    pub fn from_env() -> Result<Self, String> {
        let mut config = Self::default();
        if let Ok(value) = std::env::var("TERRA_WORLD_SIZE") {
            config.size = value
                .parse()
                .map_err(|_| "TERRA_WORLD_SIZE must be a number of metres".to_owned())?;
        }
        config.tiles = crate::geo::GeoTileConfig::from_env()?;
        if crate::next_competition::enabled() {
            config.prepare_next_field();
        } else if std::env::var("TERRA_MISSION").as_deref() == Ok("1") {
            config.tiles.enabled = false;
            config.landscape.enabled = false;
            config.size = 100.;
            config.scene = SceneKind::Mission;
        } else if config.tiles.enabled {
            config.scene = SceneKind::Tiles;
        }
        if let Ok(seed) = std::env::var("TERRA_MISSION_SEED") {
            config.landscape.seed = seed
                .parse()
                .map_err(|_| "invalid mission seed".to_owned())?;
        }
        config.validate().map_err(str::to_owned)?;
        Ok(config)
    }

    /// Flat NEXT pitch: no practice town and no elevation tiles.
    pub(crate) fn prepare_next_field(&mut self) {
        self.tiles.enabled = false;
        self.landscape.enabled = false;
        self.show_grid = false;
        self.size = crate::next_competition::WORLD_SIZE;
        // A ~0.4 m rover disappears if this frames the whole 105 m pitch.
        // Stay close and follow the spawn at the centre spot.
        self.camera_offset = Vec3::new(0.0, 3.0, 4.2);
        self.follow_rover = true;
        self.scene = SceneKind::Next;
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        if !self.size.is_finite()
            || self.size <= 0.0
            || !self.grid_spacing.is_finite()
            || self.grid_spacing <= 0.0
        {
            return Err("ground size and grid spacing must be finite and positive");
        }
        if self.size / self.grid_spacing > 500.0 {
            return Err("grid must have at most 500 intervals across the ground");
        }
        if !self.camera_offset.is_finite()
            || self.camera_offset.length_squared() < 0.01
            || !self.camera_offset.length_squared().is_finite()
            || self.camera_offset.cross(Vec3::Y).length_squared() < 0.01
        {
            return Err("camera offset must be finite and not parallel to its up axis");
        }
        if !self.camera_response.is_finite() || self.camera_response <= 0.0 {
            return Err("camera response must be finite and positive");
        }
        self.landscape.validate(self.size)?;
        self.tiles.validate()?;
        if self.tiles.enabled && !(40.0..=512.0).contains(&self.size) {
            return Err("tile worlds must be 40 to 512 metres wide");
        }
        Ok(())
    }
}

#[derive(Component)]
pub struct WorldElement;
#[derive(Component)]
pub struct Ground;
#[derive(Component)]
pub struct WorldCamera;
#[derive(Component)]
pub struct BoundaryMarker;

fn generate_world(
    mut commands: Commands,
    config: Res<WorldConfig>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let ground_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.28, 0.34, 0.23),
        perceptual_roughness: 1.0,
        ..default()
    });
    let grid_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.46, 0.51, 0.40),
        perceptual_roughness: 1.0,
        ..default()
    });
    let marker_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.95, 0.45, 0.08),
        perceptual_roughness: 0.8,
        ..default()
    });
    let cube = meshes.add(Cuboid::new(1.0, 1.0, 1.0));
    // Top surface at Y=0, matching the rover controller's ground plane.
    commands.spawn((
        Name::new("Terra ground"),
        WorldElement,
        Ground,
        RigidBody::Static,
        Collider::cuboid(1.0, 1.0, 1.0),
        Restitution::ZERO,
        Mesh3d(cube.clone()),
        MeshMaterial3d(ground_material),
        Transform::from_xyz(0.0, -0.1, 0.0).with_scale(Vec3::new(config.size, 0.2, config.size)),
    ));
    let half = config.size / 2.0;
    let steps = (half / config.grid_spacing).floor() as i32;
    let thickness = (config.grid_spacing * 0.01).min(0.04);
    if config.show_grid {
        for index in -steps..=steps {
            let position = index as f32 * config.grid_spacing;
            for (translation, scale) in [
                (
                    Vec3::new(position, 0.002, 0.0),
                    Vec3::new(thickness, 0.004, config.size),
                ),
                (
                    Vec3::new(0.0, 0.002, position),
                    Vec3::new(config.size, 0.004, thickness),
                ),
            ] {
                commands.spawn((
                    Name::new("Distance grid"),
                    WorldElement,
                    Mesh3d(cube.clone()),
                    MeshMaterial3d(grid_material.clone()),
                    Transform::from_translation(translation).with_scale(scale),
                ));
            }
        }
    }
    for x in [-half, half] {
        for z in [-half, half] {
            commands.spawn((
                Name::new("World boundary marker"),
                WorldElement,
                BoundaryMarker,
                RigidBody::Static,
                Collider::cuboid(1.0, 1.0, 1.0),
                Restitution::ZERO,
                Mesh3d(cube.clone()),
                MeshMaterial3d(marker_material.clone()),
                Transform::from_xyz(x, 0.5, z).with_scale(Vec3::new(0.3, 1.0, 0.3)),
            ));
        }
    }
    let camera = commands
        .spawn((
            Name::new("Terra world camera"),
            WorldElement,
            WorldCamera,
            Camera3d::default(),
            Transform::from_translation(config.camera_offset).looking_at(Vec3::ZERO, Vec3::Y),
        ))
        .id();
    #[cfg(not(feature = "native"))]
    let _ = camera;
    #[cfg(feature = "native")]
    commands
        .entity(camera)
        .insert(bevy_voxel_world::prelude::VoxelWorldCamera::<
            crate::voxel_terrain::TerraVoxelTerrain,
        >::default());
    commands.spawn((
        Name::new("Sun"),
        WorldElement,
        DirectionalLight {
            illuminance: 10_000.0,
            // WebGL2 software adapters reject the depth copies shadow maps use.
            shadow_maps_enabled: cfg!(not(target_arch = "wasm32")),
            ..default()
        },
        Transform::from_xyz(4.0, 8.0, 4.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
}

fn apply_session(world: &mut World) {
    let Some(session) = world.get_resource::<SimSession>().cloned() else {
        return;
    };
    let mut config = WorldConfig::for_choice(session.world);
    let mut note = String::new();
    if config.tiles.enabled {
        match crate::geo::load_patch(&config.tiles, config.size) {
            Ok(patch) => {
                eprintln!(
                    "Terra tiles: {} cells around {:.5}, {:.5} zoom {} ({} obstacles, origin elevation {:.1} m)",
                    patch.cells.len(),
                    patch.anchor.latitude,
                    patch.anchor.longitude,
                    patch.anchor.zoom,
                    patch.cells.iter().filter(|cell| cell.obstacle).count(),
                    patch.origin_elevation
                );
                world.insert_resource(patch);
            }
            Err(error) => {
                note = format!("Tiles did not load ({error}). Showing the practice town.");
                eprintln!("Terra tiles: {error}. Continuing with the flat practice world.");
                config = WorldConfig::for_choice(crate::session::WorldChoice::Town);
            }
        }
    } else {
        world.remove_resource::<crate::geo::GeoPatch>();
    }
    if config.validate().is_err() {
        note = "That world configuration was rejected. Showing the practice town.".into();
        config = WorldConfig::for_choice(crate::session::WorldChoice::Town);
    }
    world.insert_resource(config.clone());
    if let Some(mut fleet) = world.get_resource_mut::<crate::terra::RoverFleet>() {
        let _ = fleet.set_count(session.rover_count);
    }
    #[cfg(feature = "native")]
    if let Some(mut terrain) = world.get_resource_mut::<crate::voxel_terrain::TerraVoxelTerrain>() {
        terrain.enabled = config.scene == SceneKind::Town && config.landscape.voxel_hills;
        terrain.size = config.size;
        terrain.seed = config.landscape.seed;
    }
    let mut doomed = Vec::new();
    {
        let mut query = world.query_filtered::<Entity, Or<(
            With<WorldElement>,
            With<Rover>,
            With<crate::next_competition::NextField>,
            With<crate::mission::MissionObstacle>,
        )>>();
        for entity in query.iter(world) {
            doomed.push(entity);
        }
    }
    #[cfg(feature = "native")]
    {
        let mut query =
            world.query_filtered::<Entity, With<crate::voxel_terrain::VoxelTerrainChunk>>();
        for entity in query.iter(world) {
            doomed.push(entity);
        }
    }
    for entity in doomed {
        // A parent despawn already removes its children. Skip ids that are gone
        // so a second despawn does not fail the command queue.
        if world.get_entity(entity).is_ok() {
            world.entity_mut(entity).despawn();
        }
    }
    if let Some(mut session) = world.get_resource_mut::<SimSession>() {
        session.note = note;
        session.world = match config.scene {
            SceneKind::Next => crate::session::WorldChoice::Next,
            SceneKind::Tiles => crate::session::WorldChoice::Tiles,
            _ => crate::session::WorldChoice::Town,
        };
    }
}

fn finish_rebuild(mut commands: Commands) {
    commands.remove_resource::<crate::session::RebuildWorld>();
}

type FollowRovers<'w, 's> = Query<
    'w,
    's,
    (&'static Transform, Option<&'static crate::terra::RoverSlot>),
    (With<Rover>, Without<WorldCamera>),
>;

fn follow_rover(
    config: Res<WorldConfig>,
    time: Res<Time>,
    rovers: FollowRovers<'_, '_>,
    mut cameras: Query<&mut Transform, (With<WorldCamera>, Without<Rover>)>,
) {
    if !config.follow_rover {
        return;
    }
    // Slot zero is the primary rover; standalone single-rover scenes also work.
    let Some((rover, _)) = rovers
        .iter()
        .find(|(_, slot)| slot.is_some_and(|slot| slot.0 == 0))
        .or_else(|| rovers.single().ok())
    else {
        return;
    };
    let blend = 1.0 - (-config.camera_response * time.delta_secs()).exp();
    for mut camera in &mut cameras {
        camera.translation = camera
            .translation
            .lerp(rover.translation + config.camera_offset, blend);
        camera.look_at(rover.translation, Vec3::Y);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn headless() -> App {
        let mut app = App::new();
        app.insert_resource(Assets::<Mesh>::default())
            .insert_resource(Assets::<StandardMaterial>::default())
            .insert_resource(Time::<()>::default());
        app
    }
    #[test]
    fn generates_ground_camera_light_and_markers_once() {
        let mut app = headless();
        app.add_plugins(TerraWorldPlugin::default());
        app.update();
        let world = app.world_mut();
        assert_eq!(
            world
                .query_filtered::<Entity, With<Ground>>()
                .iter(world)
                .count(),
            1
        );
        assert_eq!(
            world
                .query_filtered::<Entity, With<WorldCamera>>()
                .iter(world)
                .count(),
            1
        );
        assert_eq!(
            world
                .query_filtered::<Entity, With<DirectionalLight>>()
                .iter(world)
                .count(),
            1
        );
        assert_eq!(
            world
                .query_filtered::<Entity, With<BoundaryMarker>>()
                .iter(world)
                .count(),
            4
        );
        let count = world
            .query_filtered::<Entity, With<WorldElement>>()
            .iter(world)
            .count();
        app.update();
        let world = app.world_mut();
        assert_eq!(
            world
                .query_filtered::<Entity, With<WorldElement>>()
                .iter(world)
                .count(),
            count
        );
    }
    #[test]
    fn validates_dimensions_and_limits_grid_density() {
        assert!(WorldConfig::default().validate().is_ok());
        assert!(
            WorldConfig {
                size: -1.0,
                ..default()
            }
            .validate()
            .is_err()
        );
        assert!(
            WorldConfig {
                grid_spacing: 0.0,
                ..default()
            }
            .validate()
            .is_err()
        );
        assert!(
            WorldConfig {
                grid_spacing: 0.0001,
                ..default()
            }
            .validate()
            .is_err()
        );
    }
    #[test]
    fn menu_defers_the_world_until_rebuild() {
        let mut app = headless();
        app.insert_resource(crate::session::SimSession::at_menu(
            crate::session::WorldChoice::Next,
            2,
        ))
        .add_plugins(TerraWorldPlugin::default());
        app.update();
        let world = app.world_mut();
        assert_eq!(
            world
                .query_filtered::<Entity, With<Ground>>()
                .iter(world)
                .count(),
            0
        );
        {
            let mut session = world.resource_mut::<crate::session::SimSession>();
            session.phase = crate::session::Phase::Playing;
        }
        app.insert_resource(crate::session::RebuildWorld);
        app.update();
        let world = app.world_mut();
        assert_eq!(
            world
                .query_filtered::<Entity, With<Ground>>()
                .iter(world)
                .count(),
            1
        );
        assert_eq!(
            world
                .query::<&crate::next_competition::TennisBall>()
                .iter(world)
                .count(),
            crate::next_competition::ball_positions().len()
        );
        assert!(
            world
                .query::<&crate::landscape::Building>()
                .iter(world)
                .next()
                .is_none()
        );
    }
    #[test]
    fn choice_configs_select_town_tiles_and_next() {
        let town = WorldConfig::for_choice(crate::session::WorldChoice::Town);
        assert_eq!(town.scene, SceneKind::Town);
        assert!(town.landscape.enabled);
        assert!(!town.tiles.enabled);
        let tiles = WorldConfig::for_choice(crate::session::WorldChoice::Tiles);
        assert_eq!(tiles.scene, SceneKind::Tiles);
        assert!(tiles.tiles.enabled);
        assert!(!tiles.landscape.enabled);
        let next = WorldConfig::for_choice(crate::session::WorldChoice::Next);
        assert_eq!(next.scene, SceneKind::Next);
        assert_eq!(next.size, crate::next_competition::WORLD_SIZE);
        assert!(!next.landscape.enabled);
    }
    #[test]
    fn camera_follows_rover_and_handles_missing_rover() {
        let mut app = headless();
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(std::time::Duration::from_secs(1));
        app.add_plugins(TerraWorldPlugin::default());
        app.update();
        let rover = app
            .world_mut()
            .spawn((Rover, Transform::from_xyz(10.0, 0.0, -10.0)))
            .id();
        app.update();
        let world = app.world_mut();
        let camera = world
            .query_filtered::<&Transform, With<WorldCamera>>()
            .single(world)
            .unwrap();
        assert!(camera.translation.x > 9.0);
        world.despawn(rover);
        app.update();
    }
}
