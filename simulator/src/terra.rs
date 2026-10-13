//! Physics-driven differential drive: metres, seconds and radians; +Y is up,
//! local -Z is forward, and positive yaw turns left. Wheel odometry is ideal.
use crate::physics::{RoverBody, RoverPhysicsConfig};
use avian3d::prelude::*;
use bevy::prelude::*;
use zorvane_vehicle::VehicleBody;

#[derive(Default)]
pub struct TerraPlugin {
    pub config: DriveConfig,
}

impl Plugin for TerraPlugin {
    fn build(&self, app: &mut App) {
        self.config
            .validate()
            .expect("invalid differential drive geometry or limits");
        app.insert_resource(self.config.clone())
            .init_resource::<RoverFleet>()
            .add_systems(PreUpdate, reconcile_fleet)
            .add_systems(FixedUpdate, keyboard_command)
            .add_systems(
                FixedPostUpdate,
                drive
                    .run_if(crate::velocity_controller::ideal_drive_enabled)
                    .after(PhysicsSystems::Prepare)
                    .before(PhysicsSystems::StepSimulation),
            );
    }
}

#[derive(Resource, Clone)]
pub struct DriveConfig {
    /// Wheel radius in metres; replace defaults with measured rover geometry.
    pub wheel_radius: f32,
    /// Distance between the left and right wheel contact lines in metres.
    pub track_width: f32,
    /// Maximum wheel angular velocity in radians/second.
    pub max_wheel_speed: f32,
    pub keyboard_linear_speed: f32,
    pub keyboard_angular_speed: f32,
    pub model_path: String,
}

impl Default for DriveConfig {
    fn default() -> Self {
        Self::from_body(&zorvane_vehicle::TerraGround).expect("Terra ground drive is valid")
    }
}

impl DriveConfig {
    pub fn from_body(body: &dyn VehicleBody) -> Result<Self, &'static str> {
        let locomotion = body.locomotion();
        let drive = locomotion.differential()?;
        let config = Self {
            wheel_radius: drive.wheel_radius_m,
            track_width: drive.track_width_m,
            max_wheel_speed: drive.max_wheel_speed_rad_s,
            keyboard_linear_speed: drive.keyboard_linear_speed,
            keyboard_angular_speed: drive.keyboard_angular_speed,
            model_path: body.visual().asset_path.to_owned(),
        };
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        if [self.wheel_radius, self.track_width, self.max_wheel_speed]
            .iter()
            .any(|v| !v.is_finite() || *v <= 0.0)
        {
            return Err("wheel radius, track width and wheel limit must be finite and positive");
        }
        if [self.keyboard_linear_speed, self.keyboard_angular_speed]
            .iter()
            .any(|v| !v.is_finite() || *v < 0.0)
        {
            return Err("keyboard speeds must be finite and nonnegative");
        }
        Ok(())
    }

    pub fn wheels(&self, command: DriveCommand) -> WheelSpeeds {
        if self.validate().is_err() || !command.linear.is_finite() || !command.angular.is_finite() {
            return WheelSpeeds::default();
        }
        // Calculate in f64 to avoid overflow from large but finite commands.
        let half_track = self.track_width as f64 / 2.0;
        let left = (command.linear as f64 - command.angular as f64 * half_track)
            / self.wheel_radius as f64;
        let right = (command.linear as f64 + command.angular as f64 * half_track)
            / self.wheel_radius as f64;
        let scale = (left.abs().max(right.abs()) / self.max_wheel_speed as f64).max(1.0);
        WheelSpeeds {
            left: (left / scale) as f32,
            right: (right / scale) as f32,
        }
    }

    pub fn twist(&self, wheels: WheelSpeeds) -> (f32, f32) {
        (
            self.wheel_radius * (wheels.left + wheels.right) / 2.0,
            self.wheel_radius * (wheels.right - wheels.left) / self.track_width,
        )
    }
}

#[derive(Component)]
#[require(Transform, Visibility)]
pub struct Rover;

/// Remove this marker to control DriveCommand from another system.
#[derive(Component)]
pub struct KeyboardControlled;

/// Body velocity request: forward m/s and left-turn rad/s. Zero means stop.
#[derive(Component, Default, Clone, Copy)]
pub struct DriveCommand {
    pub linear: f32,
    pub angular: f32,
}

/// Achieved wheel angular velocities, rad/s, after saturation.
#[derive(Component, Default, Clone, Copy)]
pub struct WheelSpeeds {
    pub left: f32,
    pub right: f32,
}

/// Ideal wheel odometry relative to the initial rover pose.
#[derive(Component, Default)]
pub struct Odometry {
    pub x: f32,
    pub z: f32,
    pub yaw: f32,
    pub linear_velocity: f32,
    pub angular_velocity: f32,
}

impl Odometry {
    pub(crate) fn integrate(&mut self, v: f32, w: f32, dt: f32) {
        let angle = w * dt;
        // Exact constant-twist arc, stable for straight and near-straight travel.
        let half = angle / 2.0;
        let sinc = if half.abs() < 1e-4 {
            1.0 - half * half / 6.0
        } else {
            half.sin() / half
        };
        let distance = v * dt * sinc;
        self.x -= distance * (self.yaw + half).sin();
        self.z -= distance * (self.yaw + half).cos();
        self.yaw = (self.yaw + angle + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU)
            - std::f32::consts::PI;
        self.linear_velocity = v;
        self.angular_velocity = w;
    }
}

/// Limit the number of simultaneously rendered rover camera pairs.
pub const MAX_ROVERS: usize = 32;
/// Stable within a run; retired IDs are never reassigned.
#[derive(Component, Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct RoverId(pub u64);
#[derive(Component, Clone, Copy)]
pub struct RoverSlot(pub usize);
#[derive(Resource)]
pub struct RoverFleet {
    enabled: Vec<bool>,
    next_id: u64,
}
impl Default for RoverFleet {
    fn default() -> Self {
        Self {
            enabled: vec![true],
            next_id: 0,
        }
    }
}
impl RoverFleet {
    pub fn count(&self) -> usize {
        self.enabled.iter().filter(|enabled| **enabled).count()
    }
    pub fn set_count(&mut self, count: usize) -> Result<(), &'static str> {
        if count > MAX_ROVERS {
            return Err("rover count must be between 0 and 32");
        }
        self.enabled.resize(MAX_ROVERS, false);
        for (slot, enabled) in self.enabled.iter_mut().enumerate() {
            *enabled = slot < count;
        }
        Ok(())
    }
    pub fn set_enabled(&mut self, slot: usize, enabled: bool) -> Result<(), &'static str> {
        if slot >= MAX_ROVERS {
            return Err("rover slot must be between 1 and 32");
        }
        self.enabled.resize(MAX_ROVERS, false);
        self.enabled[slot] = enabled;
        Ok(())
    }
    pub fn enabled_slots(&self) -> impl Iterator<Item = usize> + '_ {
        self.enabled
            .iter()
            .enumerate()
            .filter_map(|(slot, enabled)| enabled.then_some(slot))
    }
    pub fn from_roster(roster: &crate::roster::RobotRoster) -> Self {
        let mut enabled: Vec<_> = roster.0.iter().map(|robot| robot.enabled).collect();
        enabled.resize(MAX_ROVERS, false);
        Self {
            enabled,
            next_id: 0,
        }
    }
}
pub(crate) fn reconcile_fleet(
    mut commands: Commands,
    assets: Option<Res<AssetServer>>,
    physics: Res<RoverPhysicsConfig>,
    mut fleet: ResMut<RoverFleet>,
    world: Option<Res<crate::world::WorldConfig>>,
    vehicles: Option<Res<crate::vehicle::Vehicles>>,
    roster: Option<Res<crate::roster::RobotRoster>>,
    mut meshes: Option<ResMut<Assets<Mesh>>>,
    mut materials: Option<ResMut<Assets<StandardMaterial>>>,
    rovers: Query<(Entity, &RoverId, &RoverSlot, &Transform), With<Rover>>,
) {
    let mut existing: Vec<_> = rovers.iter().collect();
    existing.sort_by_key(|(_, id, _, _)| id.0);
    let enabled_slots: std::collections::BTreeSet<_> = fleet.enabled_slots().collect();
    for (entity, _, slot, _) in &existing {
        if !enabled_slots.contains(&slot.0) {
            commands.entity(*entity).despawn();
        }
    }
    let mut occupied: std::collections::BTreeSet<_> = existing
        .iter()
        .filter(|(_, _, slot, _)| enabled_slots.contains(&slot.0))
        .map(|(_, _, slot, _)| slot.0)
        .collect();
    let mut fleet_chassis = physics.chassis_size;
    if let Some(roster) = roster.as_deref()
        && let Some(vehicles) = vehicles.as_deref()
    {
        for robot in roster.0.iter().filter(|robot| robot.enabled) {
            if let Some(body) = vehicles.0.body(&robot.vehicle) {
                let chassis = body.chassis();
                fleet_chassis = fleet_chassis.max(Vec3::new(
                    chassis.size_xyz[0],
                    chassis.size_xyz[1],
                    chassis.size_xyz[2],
                ));
            }
        }
    }
    let spacing = fleet_chassis.x.max(fleet_chassis.z) + 0.6;
    let mut positions: Vec<_> = existing
        .iter()
        .filter(|(_, _, slot, _)| enabled_slots.contains(&slot.0))
        .map(|(_, _, _, pose)| pose.translation.with_y(0.0))
        .collect();
    let offset_for = |slot: usize| {
        slot_spawn_offset(
            slot,
            spacing,
            fleet_chassis,
            world.as_ref().map_or(100.0, |world| world.size),
            crate::next_competition::enabled(),
        )
    };
    for slot in enabled_slots.iter().copied() {
        if occupied.contains(&slot) {
            continue;
        }
        let Some(offset) = offset_for(slot) else {
            continue;
        };
        // Leave a delayed spawn pending if another moving rover currently occupies it.
        if positions
            .iter()
            .any(|position| position.distance(offset) < spacing - 0.1)
        {
            continue;
        }
        occupied.insert(slot);
        positions.push(offset);
        let id = RoverId(fleet.next_id);
        fleet.next_id = fleet
            .next_id
            .checked_add(1)
            .expect("rover ID space exhausted");
        let fallback_label = vehicles
            .as_ref()
            .map(|vehicles| vehicles.0.active().display_name())
            .unwrap_or(zorvane_vehicle::TerraGround.display_name());
        let definition = roster.as_ref().and_then(|roster| roster.slot(slot));
        let body = definition
            .and_then(|robot| vehicles.as_ref()?.0.body(&robot.vehicle))
            .or_else(|| vehicles.as_ref().map(|vehicles| vehicles.0.active()))
            .unwrap_or(&zorvane_vehicle::TerraGround);
        let rover_drive = DriveConfig::from_body(body)
            .expect("registered vehicle must have supported differential drive");
        let rover_physics = RoverPhysicsConfig::from_body(body)
            .expect("registered vehicle must have a valid chassis");
        let label =
            definition.map_or_else(|| format!("robot{}", slot + 1), |robot| robot.name.clone());
        let topic =
            definition.map_or_else(|| format!("robot{}", slot + 1), |robot| robot.topic.clone());
        let description = definition.map_or_else(
            || fallback_label.to_owned(),
            |robot| robot.description.clone(),
        );
        let vehicle =
            definition.map_or_else(|| "terra-ground".to_owned(), |robot| robot.vehicle.clone());
        let color = definition
            .and_then(|robot| robot.color)
            .map_or_else(|| slot_color(slot), |[r, g, b]| Color::srgb(r, g, b));
        let mut rover = commands.spawn((
            Name::new(label.clone()),
            RoverIdentity {
                name: label,
                description,
                topic,
                vehicle,
            },
            Rover,
            id,
            RoverSlot(slot),
            DriveCommand::default(),
            WheelSpeeds::default(),
            Odometry::default(),
            RoverDriveConfig(rover_drive.clone()),
            RoverBody::new(&rover_physics),
            Transform::from_translation(
                offset + Vec3::Y * (rover_physics.chassis_size.y / 2.0 + 0.02),
            )
            .with_rotation(if slot % 2 == 0 {
                Quat::IDENTITY
            } else {
                Quat::from_rotation_y(std::f32::consts::PI)
            }),
        ));
        if slot == 0 {
            rover.insert(KeyboardControlled);
        }
        if let Some(assets) = &assets {
            rover.with_children(|parent| {
                parent.spawn((
                    WorldAssetRoot(
                        assets.load(
                            GltfAssetLabel::Scene(0).from_asset(rover_drive.model_path.clone()),
                        ),
                    ),
                    Transform::from_scale(Vec3::splat(rover_physics.model_scale)),
                ));
            });
        }
        if let (Some(meshes), Some(materials)) = (meshes.as_deref_mut(), materials.as_deref_mut()) {
            let marker_mesh =
                meshes.add(Sphere::new(0.22).mesh().ico(2).expect("valid marker mesh"));
            let marker_material = materials.add(StandardMaterial {
                base_color: color,
                emissive: color.into(),
                ..default()
            });
            rover.with_children(|parent| {
                parent.spawn((
                    Mesh3d(marker_mesh),
                    MeshMaterial3d(marker_material),
                    Transform::from_xyz(0.0, rover_physics.chassis_size.y * 0.5 + 0.30, 0.0),
                ));
            });
        }
    }
}

fn slot_spawn_offset(
    slot: usize,
    spacing: f32,
    chassis: Vec3,
    world_size: f32,
    next_field: bool,
) -> Option<Vec3> {
    let x = (slot / 2) as f32 * spacing;
    let z = if next_field {
        if slot % 2 == 0 {
            crate::next_competition::PITCH_SOUTH - chassis.z * 0.5 - 0.5
        } else {
            crate::next_competition::PITCH_NORTH + chassis.z * 0.5 + 0.5
        }
    } else if slot % 2 == 0 {
        spacing * 0.5
    } else {
        -spacing * 0.5
    };
    if x.abs() + chassis.x * 0.5 > world_size * 0.5 || z.abs() + chassis.z * 0.5 > world_size * 0.5
    {
        return None;
    }
    if next_field && !crate::next_competition::on_pitch(x, z, chassis.x.max(chassis.z) * 0.5) {
        return None;
    }
    Some(Vec3::new(x, 0.0, z))
}

fn slot_color(slot: usize) -> Color {
    const COLORS: [[f32; 3]; 8] = [
        [0.92, 0.18, 0.12],
        [0.12, 0.42, 0.96],
        [0.98, 0.68, 0.08],
        [0.12, 0.78, 0.40],
        [0.72, 0.28, 0.90],
        [0.05, 0.75, 0.78],
        [0.96, 0.36, 0.62],
        [0.78, 0.82, 0.18],
    ];
    let [r, g, b] = COLORS[slot % COLORS.len()];
    Color::srgb(r, g, b)
}

#[derive(Component, Clone, Debug)]
pub struct RoverIdentity {
    pub name: String,
    pub description: String,
    pub topic: String,
    pub vehicle: String,
}

#[derive(Component, Clone)]
pub struct RoverDriveConfig(pub DriveConfig);

fn keyboard_command(
    keys: Res<ButtonInput<KeyCode>>,
    fallback: Res<DriveConfig>,
    mut rovers: Query<
        (&mut DriveCommand, Option<&RoverDriveConfig>),
        (With<Rover>, With<KeyboardControlled>),
    >,
) {
    let axis = |positive, negative| {
        keys.pressed(positive) as i32 as f32 - keys.pressed(negative) as i32 as f32
    };
    for (mut command, rover_config) in &mut rovers {
        let config = rover_config.map_or(fallback.as_ref(), |config| &config.0);
        *command = if keys.pressed(KeyCode::Space) {
            DriveCommand::default()
        } else {
            DriveCommand {
                linear: axis(KeyCode::KeyW, KeyCode::KeyS) * config.keyboard_linear_speed,
                angular: axis(KeyCode::KeyA, KeyCode::KeyD) * config.keyboard_angular_speed,
            }
        };
    }
}

type RoverDriveQuery<'w, 's> = Query<
    'w,
    's,
    (
        &'static DriveCommand,
        Option<&'static RoverDriveConfig>,
        &'static mut WheelSpeeds,
        &'static mut Odometry,
        &'static Rotation,
        &'static mut LinearVelocity,
        &'static mut AngularVelocity,
    ),
    With<Rover>,
>;

pub(crate) fn drive(
    time: Res<Time<Fixed>>,
    fallback: Res<DriveConfig>,
    mut rovers: RoverDriveQuery,
) {
    for (command, config, mut wheels, mut pose, rotation, mut linear, mut angular) in &mut rovers {
        let config = config.map_or(fallback.as_ref(), |config| &config.0);
        *wheels = config.wheels(*command);
        let (v, w) = if config.validate().is_ok() {
            config.twist(*wheels)
        } else {
            (0.0, 0.0)
        };
        pose.integrate(v, w, time.delta_secs());
        let forward = (rotation.0 * Vec3::NEG_Z).with_y(0.0).normalize_or_zero();
        // Leave vertical velocity to gravity and the contact solver.
        linear.x = forward.x * v;
        linear.z = forward.z * v;
        angular.0 = Vec3::Y * w;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn near(a: f32, b: f32) {
        assert!((a - b).abs() < 1e-5, "{a} != {b}");
    }
    #[test]
    fn controller_preserves_gravity_and_does_not_teleport() {
        let mut app = App::new();
        let mut time = Time::<Fixed>::default();
        time.advance_by(std::time::Duration::from_secs(1));
        app.insert_resource(time)
            .insert_resource(DriveConfig::default())
            .insert_resource(ButtonInput::<KeyCode>::default())
            .add_systems(FixedUpdate, (keyboard_command, drive).chain());
        let entity = app
            .world_mut()
            .spawn((
                Rover,
                KeyboardControlled,
                DriveCommand::default(),
                WheelSpeeds::default(),
                Odometry::default(),
                Transform::from_xyz(3.0, 2.0, 4.0),
                Rotation(Quat::from_rotation_y(std::f32::consts::FRAC_PI_2)),
                LinearVelocity(Vec3::new(0.0, -2.0, 0.0)),
                AngularVelocity::ZERO,
            ))
            .id();
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyW);
        app.world_mut().run_schedule(FixedUpdate);
        let velocity = app.world().get::<LinearVelocity>(entity).unwrap().0;
        near(velocity.x, -1.5);
        near(velocity.y, -2.0);
        near(velocity.z, 0.0);
        assert_eq!(
            app.world().get::<Transform>(entity).unwrap().translation,
            Vec3::new(3.0, 2.0, 4.0)
        );
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .release(KeyCode::KeyW);
        app.world_mut().run_schedule(FixedUpdate);
        let velocity = app.world().get::<LinearVelocity>(entity).unwrap().0;
        near(velocity.x, 0.0);
        near(velocity.y, -2.0);
    }

    #[test]
    fn two_headless_rovers_apply_different_drive_commands_independently() {
        let mut time = Time::<Fixed>::default();
        time.advance_by(std::time::Duration::from_secs(1));
        let mut app = App::new();
        app.insert_resource(time)
            .insert_resource(DriveConfig::default())
            .add_systems(FixedUpdate, drive);
        let moving = app
            .world_mut()
            .spawn((
                Rover,
                DriveCommand {
                    linear: 1.0,
                    angular: 0.0,
                },
                WheelSpeeds::default(),
                Odometry::default(),
                Rotation(Quat::IDENTITY),
                LinearVelocity::ZERO,
                AngularVelocity::ZERO,
            ))
            .id();
        let stopped = app
            .world_mut()
            .spawn((
                Rover,
                DriveCommand::default(),
                WheelSpeeds::default(),
                Odometry::default(),
                Rotation(Quat::IDENTITY),
                LinearVelocity::ZERO,
                AngularVelocity::ZERO,
            ))
            .id();
        app.world_mut().run_schedule(FixedUpdate);
        assert!(app.world().get::<Odometry>(moving).unwrap().z < -0.9);
        assert_eq!(app.world().get::<Odometry>(stopped).unwrap().z, 0.0);
        assert!(app.world().get::<LinearVelocity>(moving).unwrap().z < -0.9);
        assert_eq!(
            app.world().get::<LinearVelocity>(stopped).unwrap().length(),
            0.0
        );
    }

    #[test]
    fn straight_reverse_and_stop() {
        let config = DriveConfig::default();
        for speed in [1.0, -1.0, 0.0] {
            let wheels = config.wheels(DriveCommand {
                linear: speed,
                angular: 0.0,
            });
            let (v, w) = config.twist(wheels);
            near(v, speed);
            near(w, 0.0);
            let mut pose = Odometry::default();
            pose.integrate(v, w, 1.0);
            near(pose.x, 0.0);
            near(pose.z, -speed);
        }
    }
    #[test]
    fn spin_and_quarter_circle() {
        let mut pose = Odometry::default();
        pose.integrate(0.0, 1.0, 1.0);
        near(pose.x, 0.0);
        near(pose.z, 0.0);
        near(pose.yaw, 1.0);
        let mut pose = Odometry::default();
        pose.integrate(1.0, 1.0, std::f32::consts::FRAC_PI_2);
        near(pose.x, -1.0);
        near(pose.z, -1.0);
    }
    #[test]
    fn wheel_limit_preserves_curvature_and_invalid_input_stops() {
        let config = DriveConfig::default();
        let wheels = config.wheels(DriveCommand {
            linear: 100.0,
            angular: 50.0,
        });
        assert!(wheels.left.abs() <= config.max_wheel_speed);
        assert!(wheels.right.abs() <= config.max_wheel_speed);
        let (v, w) = config.twist(wheels);
        near(w / v, 0.5);
        let wheels = config.wheels(DriveCommand {
            linear: f32::NAN,
            angular: 0.0,
        });
        near(wheels.left, 0.0);
        near(wheels.right, 0.0);
        assert!(
            DriveConfig {
                track_width: 0.0,
                ..config
            }
            .validate()
            .is_err()
        );
    }
    #[test]
    fn fleet_grows_shrinks_and_does_not_reuse_retired_ids() {
        let mut app = App::new();
        app.insert_resource(DriveConfig::default())
            .insert_resource(RoverPhysicsConfig::default())
            .insert_resource(RoverFleet::default())
            .add_systems(Update, reconcile_fleet);
        app.update();
        let world = app.world_mut();
        let original = world
            .query_filtered::<(Entity, &RoverId), With<Rover>>()
            .single(world)
            .unwrap();
        let first = original.0;
        assert_eq!(original.1.0, 0);
        world.resource_mut::<RoverFleet>().set_count(3).unwrap();
        app.update();
        let world = app.world_mut();
        assert_eq!(
            world
                .query_filtered::<&RoverId, With<Rover>>()
                .iter(world)
                .count(),
            3
        );
        world.resource_mut::<RoverFleet>().set_count(1).unwrap();
        app.update();
        assert!(app.world().get::<Rover>(first).is_some());
        app.world_mut()
            .resource_mut::<RoverFleet>()
            .set_count(3)
            .unwrap();
        app.update();
        let world = app.world_mut();
        let mut ids: Vec<_> = world
            .query_filtered::<&RoverId, With<Rover>>()
            .iter(world)
            .map(|id| id.0)
            .collect();
        ids.sort();
        assert_eq!(ids, [0, 3, 4]);
        assert!(
            world
                .resource_mut::<RoverFleet>()
                .set_count(MAX_ROVERS + 1)
                .is_err()
        );
        world.resource_mut::<RoverFleet>().set_count(0).unwrap();
        app.update();
        let world = app.world_mut();
        assert_eq!(
            world
                .query_filtered::<&RoverId, With<Rover>>()
                .iter(world)
                .count(),
            0
        );
    }
    #[test]
    fn fleet_slots_can_be_toggled_individually() {
        let mut fleet = RoverFleet::default();
        fleet.set_enabled(0, false).unwrap();
        fleet.set_enabled(1, true).unwrap();
        assert_eq!(fleet.enabled_slots().collect::<Vec<_>>(), [1]);
        assert_eq!(fleet.count(), 1);
        fleet.set_enabled(1, false).unwrap();
        assert_eq!(fleet.enabled_slots().count(), 0);
    }

    #[test]
    fn roster_enabled_slots_spawn_and_toggle_without_changing_the_body() {
        let roster = crate::roster::RobotRoster(vec![
            crate::roster::RobotDefinition {
                enabled: true,
                name: "robot1".into(),
                description: "first Terra".into(),
                vehicle: "terra-ground".into(),
                topic: "robot1".into(),
                color: None,
            },
            crate::roster::RobotDefinition {
                enabled: false,
                name: "robot2".into(),
                description: "second Terra".into(),
                vehicle: "terra-ground".into(),
                topic: "robot2".into(),
                color: None,
            },
        ]);
        let mut app = App::new();
        app.insert_resource(DriveConfig::default())
            .insert_resource(RoverPhysicsConfig::default())
            .insert_resource(RoverFleet::from_roster(&roster))
            .insert_resource(roster)
            .add_systems(Update, reconcile_fleet);
        app.update();
        let world = app.world_mut();
        let mut query = world.query::<(&RoverSlot, &RoverIdentity)>();
        let first: Vec<_> = query
            .iter(world)
            .map(|(slot, identity)| (slot.0, identity.name.as_str()))
            .collect();
        assert_eq!(first, [(0, "robot1")]);
        app.world_mut()
            .resource_mut::<RoverFleet>()
            .set_enabled(1, true)
            .unwrap();
        app.update();
        let world = app.world_mut();
        let mut query = world.query::<(
            &RoverSlot,
            &RoverIdentity,
            Option<&KeyboardControlled>,
        )>();
        let mut second: Vec<_> = query
            .iter(world)
            .map(|(slot, identity, keyboard)| {
                (slot.0, identity.topic.as_str(), keyboard.is_some())
            })
            .collect();
        second.sort_by_key(|(slot, _, _)| *slot);
        assert_eq!(second, [(0, "robot1", true), (1, "robot2", false)]);
        app.world_mut()
            .resource_mut::<RoverFleet>()
            .set_enabled(0, false)
            .unwrap();
        app.update();
        let world = app.world_mut();
        let slots: Vec<_> = world
            .query::<&RoverSlot>()
            .iter(world)
            .map(|slot| slot.0)
            .collect();
        assert_eq!(slots, [1]);
    }
    #[test]
    fn live_growth_avoids_a_rover_occupying_a_spawn_location() {
        let mut app = App::new();
        app.insert_resource(DriveConfig::default())
            .insert_resource(RoverPhysicsConfig::default())
            .init_resource::<RoverFleet>()
            .add_systems(Update, reconcile_fleet);
        app.update();
        let world = app.world_mut();
        let first = world
            .query_filtered::<Entity, With<Rover>>()
            .single(world)
            .unwrap();
        world.get_mut::<Transform>(first).unwrap().translation.x = 1.5;
        world.resource_mut::<RoverFleet>().set_count(2).unwrap();
        app.update();
        let world = app.world_mut();
        let positions: Vec<_> = world
            .query_filtered::<&Transform, With<Rover>>()
            .iter(world)
            .map(|pose| pose.translation)
            .collect();
        assert_eq!(positions.len(), 2);
        assert!(positions[0].distance(positions[1]) >= 1.4);
    }

    #[test]
    fn next_slots_alternate_pitch_ends_and_remain_inside_without_colliding() {
        let chassis = Vec3::new(0.9, 0.16, 0.8);
        let spacing = chassis.x.max(chassis.z) + 0.6;
        let south = slot_spawn_offset(0, spacing, chassis, 32.0, true).unwrap();
        let north = slot_spawn_offset(1, spacing, chassis, 32.0, true).unwrap();
        let south_again = slot_spawn_offset(2, spacing, chassis, 32.0, true).unwrap();
        for pose in [south, north, south_again] {
            assert!(crate::next_competition::on_pitch(
                pose.x,
                pose.z,
                chassis.x.max(chassis.z) * 0.5
            ));
        }
        assert!(south.z > 0.0 && north.z < 0.0 && south_again.z > 0.0);
        assert!(south.distance(north) >= spacing);
        assert!(south.distance(south_again) >= spacing);
    }

    struct NotARover;

    impl zorvane_vehicle::VehicleBody for NotARover {
        fn id(&self) -> &'static str {
            "not-a-rover"
        }
        fn display_name(&self) -> &'static str {
            "Not a rover"
        }
        fn visual(&self) -> zorvane_vehicle::VehicleVisual {
            zorvane_vehicle::VehicleVisual {
                asset_path: "models/missing.glb",
            }
        }
        fn chassis(&self) -> zorvane_vehicle::ChassisSpec {
            zorvane_vehicle::TerraGround.chassis()
        }
        fn locomotion(&self) -> zorvane_vehicle::Locomotion {
            zorvane_vehicle::Locomotion::Unsupported {
                reason: "no actuators yet",
            }
        }
    }

    #[test]
    fn only_differential_drive_bodies_become_a_drive_config() {
        assert!(DriveConfig::from_body(&NotARover).is_err());
        assert!(DriveConfig::from_body(&zorvane_vehicle::TerraGround).is_ok());
    }
}
