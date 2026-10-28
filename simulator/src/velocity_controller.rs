//! Phone-like IMU/VIO feedback into the portable controller, actuated through Avian forces.
use crate::terra::{DriveCommand, DriveConfig, KeyboardControlled, Odometry, Rover, WheelSpeeds};
use avian3d::prelude::*;
use bevy::prelude::*;
use terra_control::ControllerConfig;

use terra_types::{
    ImuSample, MotorOutput, Quaternion, StopReason, Vector3, VelocityTarget, VioSample,
};

use crate::bridge::AutonomyMotorGate;

#[derive(Resource, Clone)]
pub struct VelocitySimulationConfig {
    pub enabled: bool,
    pub imu_enabled: bool,
    pub vio_enabled: bool,
    pub vio_rate: f64,
    pub wheel_force: f32,
    pub linear_drag: f32,
    pub yaw_drag: f32,
    pub lateral_drag: f32,
}
impl Default for VelocitySimulationConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            imu_enabled: true,
            vio_enabled: true,
            vio_rate: 20.0,
            wheel_force: 30.0,
            linear_drag: 20.0,
            yaw_drag: 3.6,
            lateral_drag: 40.0,
        }
    }
}
pub struct TerraVelocityControlPlugin;
impl Plugin for TerraVelocityControlPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<VelocitySimulationConfig>()
            .insert_resource(Time::<Fixed>::from_hz(100.0))
            .add_systems(FixedPreUpdate, sample_sensors)
            .add_systems(
                PreUpdate,
                mount_controllers.after(crate::terra::reconcile_fleet),
            )
            .add_systems(
                FixedPostUpdate,
                control_and_apply
                    .after(PhysicsSystems::Prepare)
                    .before(PhysicsSystems::StepSimulation),
            );
    }
}
pub(crate) fn ideal_drive_enabled(config: Option<Res<VelocitySimulationConfig>>) -> bool {
    config.is_none_or(|config| !config.enabled)
}
pub(crate) fn keyboard_ideal_drive_enabled(config: Option<Res<VelocitySimulationConfig>>) -> bool {
    config.is_some_and(|config| config.enabled)
}
#[derive(Component)]
pub struct VelocityControlState {
    pub runtime: terra_runtime::Runtime,
    last_velocity: Option<Vec3>,
    last_vio: Option<f64>,
    pub output: MotorOutput,
}
#[cfg(test)]
impl VelocityControlState {
    pub(crate) fn new(runtime: terra_runtime::Runtime) -> Self {
        Self {
            runtime,
            last_velocity: None,
            last_vio: None,
            output: MotorOutput::stopped(StopReason::SensorNotReady),
        }
    }
}
fn mount_controllers(
    mut commands: Commands,
    config: Res<VelocitySimulationConfig>,
    drive: Res<DriveConfig>,
    physics: Option<Res<crate::physics::RoverPhysicsConfig>>,
    rovers: Query<Entity, Added<Rover>>,
) {
    for entity in &rovers {
        let controller_config = ControllerConfig {
            linear_feedforward: (config.linear_drag / (2.0 * config.wheel_force)) as f64,
            yaw_feedforward: (config.yaw_drag / (config.wheel_force * drive.track_width)) as f64,
            ..Default::default()
        };
        commands.entity(entity).insert((
            VelocityControlState {
                runtime: terra_runtime::Runtime::new(
                    terra_runtime::RuntimeConfig {
                        controller: controller_config,
                        mode: if std::env::var("TERRA_ACTUATION_MODE").as_deref()
                            == Ok("direct_effort")
                        {
                            terra_runtime::ActuationMode::DirectEffort
                        } else {
                            terra_runtime::ActuationMode::VelocityFeedback
                        },
                        outdoor_supported: true,
                        ..Default::default()
                    },
                    format!("sim-{}", entity.to_bits()),
                )
                .expect("invalid simulated runtime parameters")
                .with_dock_outlet(physics.as_deref()),
                last_velocity: None,
                last_vio: None,
                output: MotorOutput::stopped(StopReason::SensorNotReady),
            },
            ConstantForce::default(),
            ConstantTorque::default(),
        ));
    }
}
/// Line the bucket dock up with the NEXT dumper outlet of this chassis.
trait WithDockOutlet {
    fn with_dock_outlet(self, physics: Option<&crate::physics::RoverPhysicsConfig>) -> Self;
}
impl WithDockOutlet for terra_runtime::Runtime {
    fn with_dock_outlet(mut self, physics: Option<&crate::physics::RoverPhysicsConfig>) -> Self {
        if let Some(physics) = physics {
            let mut dock = self.autonomy.dock_config().clone();
            dock.outlet_ahead = f64::from(crate::next_competition::dumper_outlet_ahead(
                physics.chassis_size,
            ));
            dock.blind_range = dock.outlet_ahead + 0.35;
            self.autonomy.configure_dock(dock);
        }
        self
    }
}
fn world_vector(value: Vec3) -> Vector3 {
    Vector3 {
        x: -value.z as f64,
        y: -value.x as f64,
        z: value.y as f64,
    }
}
fn sample_sensors(
    time: Res<Time<Fixed>>,
    config: Res<VelocitySimulationConfig>,
    mut rovers: Query<
        (
            &Rotation,
            &Position,
            &LinearVelocity,
            &AngularVelocity,
            &mut VelocityControlState,
        ),
        With<Rover>,
    >,
) {
    let t = time.elapsed_secs_f64();
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }
    for (rotation, position, velocity, angular, mut brain) in &mut rovers {
        let basis = Quat::from_mat3(&Mat3::from_cols(
            Vec3::new(0., -1., 0.),
            Vec3::new(0., 0., 1.),
            Vec3::new(-1., 0., 0.),
        ));
        let q = basis * rotation.0 * basis.inverse();
        let orientation = Quaternion {
            x: q.x as f64,
            y: q.y as f64,
            z: q.z as f64,
            w: q.w as f64,
        };
        let acceleration = brain
            .last_velocity
            .map_or(Vec3::ZERO, |last| (velocity.0 - last) / dt);
        brain.last_velocity = Some(velocity.0);
        if config.imu_enabled {
            let sample = ImuSample {
                timestamp: t,
                acceleration: orientation.conjugate().rotate(world_vector(acceleration)),
                angular_velocity: orientation.conjugate().rotate(world_vector(angular.0)),
            };
            let _ = brain.runtime.ingest(terra_runtime::RuntimeEvent::Imu {
                sample,
                received_at: t,
            });
        }
        if config.vio_enabled
            && brain
                .last_vio
                .is_none_or(|last| t - last >= 1.0 / config.vio_rate)
        {
            let sample = VioSample {
                timestamp: t,
                position: world_vector(position.0),
                orientation,
                velocity: world_vector(velocity.0),
                tracked: true,
            };
            let _ = brain.runtime.ingest(terra_runtime::RuntimeEvent::Vio {
                sample,
                received_at: t,
            });
            brain.last_vio = Some(t);
        }
    }
}
type ControlledRovers<'w, 's> = Query<
    'w,
    's,
    (
        &'static DriveCommand,
        Option<&'static AutonomyMotorGate>,
        &'static Rotation,
        &'static Position,
        &'static LinearVelocity,
        &'static AngularVelocity,
        &'static mut VelocityControlState,
        &'static mut ConstantForce,
        &'static mut ConstantTorque,
        &'static mut WheelSpeeds,
        &'static mut Odometry,
    ),
    (With<Rover>, Without<KeyboardControlled>),
>;
fn control_and_apply(
    time: Res<Time<Fixed>>,
    config: Res<VelocitySimulationConfig>,
    drive: Res<DriveConfig>,
    mut rovers: ControlledRovers<'_, '_>,
) {
    if !config.enabled {
        for (_, _, _, _, _, _, _, mut force, mut torque, _, _) in &mut rovers {
            force.0 = Vec3::ZERO;
            torque.0 = Vec3::ZERO;
        }
        return;
    }
    let t = time.elapsed_secs_f64();
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }
    for (
        request,
        gate,
        rotation,
        _position,
        velocity,
        angular,
        mut brain,
        mut force,
        mut torque,
        mut wheels,
        mut odometry,
    ) in &mut rovers
    {
        if gate.is_none() {
            let target = VelocityTarget {
                timestamp: t,
                forward: request.linear as f64,
                yaw_rate: request.angular as f64,
            };
            let _ = brain
                .runtime
                .ingest(terra_runtime::RuntimeEvent::LocalVelocity {
                    target,
                    received_at: t,
                });
            if let Ok(out) = brain.runtime.step(t) {
                brain.output = out.motor;
            }
        }
        let output = brain.output;
        let forward = (rotation.0 * Vec3::NEG_Z).with_y(0.0).normalize_or_zero();
        let speed = velocity.0.dot(forward);
        let lateral = velocity.0.with_y(0.0) - forward * speed;
        force.0 = forward
            * ((output.left + output.right) as f32 * config.wheel_force
                - config.linear_drag * speed)
            - lateral * config.lateral_drag;
        torque.0 = Vec3::Y
            * ((output.right - output.left) as f32 * config.wheel_force * drive.track_width * 0.5
                - config.yaw_drag * angular.y);
        *wheels = drive.wheels(DriveCommand {
            linear: speed,
            angular: angular.y,
        });
        odometry.integrate(speed, angular.y, dt);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        physics::{RoverBody, RoverPhysicsConfig, TerraPhysicsPlugin},
        terra::{DriveCommand, DriveConfig, Odometry, Rover, WheelSpeeds},
    };
    #[test]
    fn simulator_sensor_adapter_matches_portable_runtime_replay() {
        for mode in [
            terra_runtime::ActuationMode::DirectEffort,
            terra_runtime::ActuationMode::VelocityFeedback,
        ] {
            let config = terra_runtime::RuntimeConfig {
                mode,
                ..Default::default()
            };
            let mut reference =
                terra_runtime::Runtime::new(config.clone(), "replay".into()).unwrap();
            let mut app = App::new();
            app.add_plugins(MinimalPlugins)
                .insert_resource(VelocitySimulationConfig {
                    vio_rate: 1000.0,
                    ..Default::default()
                })
                .add_systems(FixedPreUpdate, sample_sensors);
            let entity = app
                .world_mut()
                .spawn((
                    Rover,
                    Rotation::IDENTITY,
                    Position(Vec3::new(-2.0, 0.0, -1.0)),
                    LinearVelocity::ZERO,
                    AngularVelocity::ZERO,
                    VelocityControlState::new(
                        terra_runtime::Runtime::new(config, "replay".into()).unwrap(),
                    ),
                ))
                .id();
            for tick in 1..=100 {
                let t = tick as f64 * 0.01;
                app.world_mut()
                    .resource_mut::<Time<Fixed>>()
                    .advance_by(std::time::Duration::from_millis(10));
                app.world_mut().run_schedule(FixedPreUpdate);
                reference
                    .ingest(terra_runtime::RuntimeEvent::Imu {
                        sample: ImuSample {
                            timestamp: t,
                            acceleration: Vector3::ZERO,
                            angular_velocity: Vector3::ZERO,
                        },
                        received_at: t,
                    })
                    .unwrap();
                reference
                    .ingest(terra_runtime::RuntimeEvent::Vio {
                        sample: VioSample {
                            timestamp: t,
                            position: Vector3 {
                                x: 1.0,
                                y: 2.0,
                                z: 0.0,
                            },
                            orientation: Quaternion::default(),
                            velocity: Vector3::ZERO,
                            tracked: true,
                        },
                        received_at: t,
                    })
                    .unwrap();
                let target = VelocityTarget {
                    timestamp: t,
                    forward: 0.2,
                    yaw_rate: 0.1,
                };
                reference
                    .ingest(terra_runtime::RuntimeEvent::LocalVelocity {
                        target,
                        received_at: t,
                    })
                    .unwrap();
                let expected = reference.step(t).unwrap();
                let mut state = app
                    .world_mut()
                    .get_mut::<VelocityControlState>(entity)
                    .unwrap();
                state
                    .runtime
                    .ingest(terra_runtime::RuntimeEvent::LocalVelocity {
                        target,
                        received_at: t,
                    })
                    .unwrap();
                let actual = state.runtime.step(t).unwrap();
                assert!((actual.motor.left - expected.motor.left).abs() <= 1e-6);
                assert!((actual.motor.right - expected.motor.right).abs() <= 1e-6);
                assert_eq!(
                    actual.snapshot.value["status"],
                    expected.snapshot.value["status"]
                );
                assert_eq!(
                    actual.autonomy.command_origin,
                    expected.autonomy.command_origin
                );
            }
        }
    }
    #[test]
    fn feedback_controller_tracks_velocity_using_motor_forces_and_stops() {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            TransformPlugin,
            bevy::asset::AssetPlugin::default(),
            bevy::mesh::MeshPlugin,
            TerraPhysicsPlugin,
            TerraVelocityControlPlugin,
        ))
        .insert_resource(DriveConfig::default())
        .insert_resource(Time::<Fixed>::from_hz(100.0))
        .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_millis(10),
        ));
        app.finish();
        app.cleanup();
        app.world_mut().spawn((
            RigidBody::Static,
            Collider::cuboid(100.0, 0.2, 100.0),
            Transform::from_xyz(0.0, -0.1, 0.0),
        ));
        let rover = app
            .world_mut()
            .spawn((
                Rover,
                RoverBody::default(),
                DriveCommand {
                    linear: 1.0,
                    angular: 0.3,
                },
                WheelSpeeds::default(),
                Odometry::default(),
                Transform::from_xyz(
                    0.0,
                    RoverPhysicsConfig::default().chassis_size.y * 0.5 + 0.02,
                    0.0,
                ),
            ))
            .id();
        for _ in 0..800 {
            app.update();
        }
        let world = app.world();
        let rotation = world.get::<Rotation>(rover).unwrap().0;
        let velocity = world.get::<LinearVelocity>(rover).unwrap().0;
        let forward = velocity.dot(rotation * Vec3::NEG_Z);
        let yaw = world.get::<AngularVelocity>(rover).unwrap().y;
        assert!(
            (forward - 1.0).abs() < 0.08,
            "forward={forward}, output={:?}, yaw={yaw}, time={}",
            world.get::<VelocityControlState>(rover).unwrap().output,
            world.resource::<Time<Fixed>>().elapsed_secs_f64()
        );
        assert!((yaw - 0.3).abs() < 0.08, "yaw={yaw}");
        app.world_mut()
            .get_mut::<DriveCommand>(rover)
            .unwrap()
            .linear = 0.0;
        app.world_mut()
            .get_mut::<DriveCommand>(rover)
            .unwrap()
            .angular = 0.0;
        for _ in 0..500 {
            app.update();
        }
        assert!(
            app.world()
                .get::<LinearVelocity>(rover)
                .unwrap()
                .0
                .with_y(0.0)
                .length()
                < 0.04
        );

        app.world_mut()
            .entity_mut(rover)
            .insert(crate::next_competition::RoverBallLoad {
                collected: 0,
                waiting: 2,
                dumper: 1,
            });
        app.world_mut()
            .get_mut::<DriveCommand>(rover)
            .unwrap()
            .linear = 1.0;
        for _ in 0..500 {
            app.update();
        }
        let world = app.world();
        let rotation = world.get::<Rotation>(rover).unwrap().0;
        let forward = world
            .get::<LinearVelocity>(rover)
            .unwrap()
            .0
            .dot(rotation * Vec3::NEG_Z);
        assert!(forward > 0.8, "loaded dumper stopped the rover: {forward}");
    }

    #[test]
    fn keyboard_teleop_tracks_straight_and_stops_without_reversing() {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            TransformPlugin,
            bevy::asset::AssetPlugin::default(),
            bevy::mesh::MeshPlugin,
            TerraPhysicsPlugin,
            TerraVelocityControlPlugin,
        ))
        .insert_resource(DriveConfig::default())
        .insert_resource(Time::<Fixed>::from_hz(100.0))
        .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_millis(10),
        ))
        .add_systems(
            FixedPostUpdate,
            crate::terra::drive_keyboard
                .after(PhysicsSystems::Prepare)
                .before(PhysicsSystems::StepSimulation),
        );
        app.finish();
        app.cleanup();
        app.world_mut().spawn((
            RigidBody::Static,
            Collider::cuboid(100.0, 0.2, 100.0),
            Transform::from_xyz(0.0, -0.1, 0.0),
        ));
        let rover = app
            .world_mut()
            .spawn((
                Rover,
                KeyboardControlled,
                RoverBody::default(),
                DriveCommand {
                    linear: 1.0,
                    angular: 0.0,
                },
                WheelSpeeds::default(),
                Odometry::default(),
                Transform::from_xyz(
                    0.0,
                    RoverPhysicsConfig::default().chassis_size.y * 0.5 + 0.02,
                    0.0,
                ),
            ))
            .id();
        for _ in 0..3 {
            app.update();
        }
        let first_speed = -app.world().get::<LinearVelocity>(rover).unwrap().z;
        assert!(
            first_speed > 0.0 && first_speed < 1.0,
            "acceleration skipped: {first_speed}"
        );
        for _ in 0..200 {
            app.update();
        }
        let moving = app.world().get::<Position>(rover).unwrap().0;
        assert!(moving.z < -1.5, "rover did not drive forward: {moving:?}");
        assert!(moving.x.abs() < 0.02, "rover drifted sideways: {moving:?}");

        *app.world_mut().get_mut::<DriveCommand>(rover).unwrap() = DriveCommand::default();
        for _ in 0..30 {
            app.update();
        }
        let stopped = app.world().get::<Position>(rover).unwrap().0;
        let velocity = app.world().get::<LinearVelocity>(rover).unwrap().0;
        assert!(
            velocity.x.abs() < 0.01 && velocity.z.abs() < 0.01,
            "{velocity:?}"
        );
        assert!(
            stopped.z < moving.z && stopped.z > moving.z - 0.15,
            "unexpected stopping motion: {moving:?} -> {stopped:?}"
        );
    }
}
