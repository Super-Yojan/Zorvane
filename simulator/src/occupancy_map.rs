//! Exposure-aligned depth frames update an independent rolling map per rover.
use crate::{
    depth_camera::{DepthCamera, DepthFrame},
    terra::Rover,
};
use bevy::prelude::*;
use terra_mapping::{CameraIntrinsics, CameraPose, MapConfig};
use terra_types::{Quaternion, Vector3};
#[derive(Default)]
pub struct TerraOccupancyMapPlugin;
impl Plugin for TerraOccupancyMapPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            update_maps.after(crate::depth_camera::receive_depth),
        );
    }
}
#[derive(Component)]
pub struct RoverOccupancyMap {
    pub map: terra_runtime::mapping::MappingState,
    pub config: MapConfig,
    pub last_sequence: u64,
    pub last_exposure: Option<f64>,
    ground: Option<terra_runtime::ground::GroundPolicy>,
}
/// Convert optical X-right/Y-down/Z-forward into Bevy camera X-right/Y-up/-Z-forward,
/// then Bevy world into robotics world (-Z, -X, Y).
pub(crate) fn camera_pose(transform: &GlobalTransform) -> CameraPose {
    let t = transform.compute_transform();
    let world_from_bevy = Quat::from_mat3(&Mat3::from_cols(
        Vec3::new(0.0, -1.0, 0.0),
        Vec3::new(0.0, 0.0, 1.0),
        Vec3::new(-1.0, 0.0, 0.0),
    ));
    let q = world_from_bevy * t.rotation * Quat::from_rotation_x(std::f32::consts::PI);
    CameraPose {
        position: Vector3 {
            x: -t.translation.z as f64,
            y: -t.translation.x as f64,
            z: t.translation.y as f64,
        },
        orientation: Quaternion {
            x: q.x as f64,
            y: q.y as f64,
            z: q.z as f64,
            w: q.w as f64,
        },
    }
}
/// Rover origin in robotics XY, and yaw of body forward (Bevy −Z) about world up.
pub(crate) fn body_pose(transform: &GlobalTransform) -> (f64, f64, f64) {
    let t = transform.compute_transform();
    let forward = t.rotation * -Vec3::Z;
    let x = -forward.z as f64;
    let y = -forward.x as f64;
    (-t.translation.z as f64, -t.translation.x as f64, y.atan2(x))
}
fn update_maps(
    mut commands: Commands,
    sensors: Query<(&DepthCamera, &DepthFrame, &ChildOf)>,
    physics: Option<Res<crate::physics::RoverPhysicsConfig>>,
    mut rovers: Query<
        (
            Entity,
            &GlobalTransform,
            Option<&mut RoverOccupancyMap>,
            Option<&crate::terra::RoverId>,
            Option<&crate::velocity_controller::VelocityControlState>,
        ),
        With<Rover>,
    >,
) {
    for (entity, transform, state, rover_id, control) in &mut rovers {
        if state.is_none() {
            commands.entity(entity).insert(RoverOccupancyMap {
                map: control
                    .map(|c| c.runtime.mapping_handle())
                    .unwrap_or_else(|| {
                        terra_runtime::mapping::MappingState::new(MapConfig::default()).unwrap()
                    }),
                config: MapConfig::default(),
                last_sequence: 0,
                last_exposure: None,
                ground: None,
            });
            continue;
        }
        let mut state = state.unwrap();
        for (camera, frame, parent) in &sensors {
            if parent.parent() != entity || frame.sequence <= state.last_sequence {
                continue;
            }
            let Some(exposure) = frame.exposure else {
                continue;
            };
            state.last_sequence = frame.sequence;
            if state.last_exposure.is_some_and(|t| exposure.timestamp <= t) {
                continue;
            }
            let body = control
                .and_then(|c| c.runtime.pose.map(|p| (p.0.x, p.0.y)))
                .unwrap_or_else(|| {
                    let p = transform.translation();
                    (-p.z as f64, -p.x as f64)
                });
            if state.map.recenter(body.0, body.1).is_err() {
                continue;
            }
            let fy = frame.height as f64 / (2.0 * (camera.config.vertical_fov as f64 / 2.0).tan());
            let intrinsics = CameraIntrinsics {
                width: frame.width,
                height: frame.height,
                fx: fy,
                fy,
                cx: frame.width as f64 / 2.0 - 0.5,
                cy: frame.height as f64 / 2.0 - 0.5,
            };
            let camera_measurement = camera_pose(&exposure.camera_transform);
            let up = exposure.body_transform.compute_transform().rotation * Vec3::Y;
            let up = Vector3 {
                x: -up.z as f64,
                y: -up.x as f64,
                z: up.y as f64,
            };
            if state.ground.is_none() {
                let body = exposure.body_transform.compute_transform();
                let camera = exposure.camera_transform.translation();
                let normal = body.rotation * Vec3::Y;
                // The cuboid body's bottom is its supporting surface; mount clearance
                // is measured along body up and is independent of world elevation.
                let half_height = physics.as_ref().map_or_else(
                    || crate::physics::RoverPhysicsConfig::default().chassis_size.y * 0.5,
                    |p| p.chassis_size.y * 0.5,
                );
                let clearance = (camera - body.translation).dot(normal) + half_height;
                state.ground = terra_runtime::ground::GroundPolicy::calibrated(clearance as f64);
            }
            let Some(ground) = state
                .ground
                .as_ref()
                .and_then(|g| g.plane(camera_measurement.position, up))
            else {
                continue;
            };
            if state
                .map
                .integrate_depth_with_ground_plane(
                    exposure.timestamp,
                    intrinsics,
                    camera_measurement,
                    &frame.depth_metres,
                    ground,
                )
                .is_ok()
            {
                state.last_exposure = Some(exposure.timestamp);
                crate::observation_capture::capture_depth(
                    rover_id.map(|id| id.0).unwrap_or(entity.to_bits()),
                    frame,
                    &intrinsics,
                    &state.map,
                    &state.config,
                );
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn translated_terrain_preserves_depth_map() {
        fn run(elevation: f32) -> Vec<i8> {
            let mut app = App::new();
            app.add_plugins(MinimalPlugins)
                .add_systems(Update, update_maps);
            let rover = app
                .world_mut()
                .spawn((
                    Rover,
                    GlobalTransform::from(Transform::from_xyz(0.0, elevation, 0.0)),
                ))
                .id();
            app.world_mut().spawn((
                DepthCamera {
                    config: crate::depth_camera::DepthCameraConfig {
                        resolution: UVec2::ONE,
                        ..default()
                    },
                    depth_texture: default(),
                    preview_texture: default(),
                },
                DepthFrame {
                    width: 1,
                    height: 1,
                    depth_metres: vec![2.0],
                    sequence: 1,
                    received_at: 1.0,
                    exposure: Some(crate::depth_camera::DepthExposure {
                        sensor: Entity::PLACEHOLDER,
                        timestamp: 1.0,
                        camera_transform: GlobalTransform::from(Transform::from_xyz(
                            0.0,
                            elevation + 0.5,
                            0.0,
                        )),
                        body_transform: GlobalTransform::from(Transform::from_xyz(
                            0.0, elevation, 0.0,
                        )),
                    }),
                },
                ChildOf(rover),
            ));
            app.update();
            app.update();
            let state = app.world().get::<RoverOccupancyMap>(rover).unwrap();
            assert_eq!(state.last_exposure, Some(1.0));
            state.map.snapshot().occupancy
        }
        assert_eq!(run(0.0), run(5.0));
        assert_eq!(run(0.0), run(-5.0));
    }
    #[test]
    fn optical_axes_and_camera_mount_convert_to_robotics_world() {
        let p = camera_pose(&GlobalTransform::from(Transform::from_xyz(2.0, 0.5, -3.0)));
        assert_eq!(
            p.position,
            Vector3 {
                x: 3.0,
                y: -2.0,
                z: 0.5
            }
        );
        let forward = p.orientation.rotate(Vector3 {
            x: 0.0,
            y: 0.0,
            z: 1.0,
        });
        assert!((forward.x - 1.0).abs() < 1e-6);
        let down = p.orientation.rotate(Vector3 {
            x: 0.0,
            y: 1.0,
            z: 0.0,
        });
        assert!((down.z + 1.0).abs() < 1e-6);
    }
    #[test]
    fn body_pose_uses_robotics_axes_and_bevy_heading() {
        let straight = body_pose(&GlobalTransform::from(Transform::from_xyz(2.0, 0.4, -3.0)));
        assert!((straight.0 - 3.0).abs() < 1e-6);
        assert!((straight.1 + 2.0).abs() < 1e-6);
        assert!(straight.2.abs() < 1e-5);
        let turned = body_pose(&GlobalTransform::from(Transform::from_rotation(
            Quat::from_rotation_y(std::f32::consts::FRAC_PI_2),
        )));
        assert!((turned.2 - std::f64::consts::FRAC_PI_2).abs() < 1e-5);
    }
    #[test]
    fn late_frame_uses_exposure_pose_and_maps_are_independent() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_systems(Update, update_maps);
        let rover = app
            .world_mut()
            .spawn((
                Rover,
                GlobalTransform::from(Transform::from_xyz(0.0, 0.0, -1.0)),
            ))
            .id();
        let other = app
            .world_mut()
            .spawn((Rover, GlobalTransform::default()))
            .id();
        app.world_mut().spawn((
            DepthCamera {
                config: crate::depth_camera::DepthCameraConfig {
                    resolution: UVec2::ONE,
                    ..default()
                },
                depth_texture: default(),
                preview_texture: default(),
            },
            DepthFrame {
                width: 1,
                height: 1,
                depth_metres: vec![2.0],
                sequence: 1,
                received_at: 10.0,
                exposure: Some(crate::depth_camera::DepthExposure {
                    sensor: Entity::PLACEHOLDER,
                    timestamp: 1.0,
                    camera_transform: GlobalTransform::from(Transform::from_xyz(-0.25, 0.5, -0.25)),
                    body_transform: GlobalTransform::from(Transform::from_xyz(0.0, 0.0, -1.0)),
                }),
            },
            ChildOf(rover),
        ));
        app.update();
        app.update();
        let mapped = app.world().get::<RoverOccupancyMap>(rover).unwrap();
        assert_eq!(mapped.last_exposure, Some(1.0));
        let grid = mapped.map.snapshot();
        let col = ((2.25 - grid.origin_x) / grid.resolution).floor() as usize;
        let row = ((0.25 - grid.origin_y) / grid.resolution).floor() as usize;
        assert!(grid.occupancy[row * grid.width as usize + col] > 50);
        assert!(
            app.world()
                .get::<RoverOccupancyMap>(other)
                .unwrap()
                .map
                .snapshot()
                .occupancy
                .iter()
                .all(|v| *v == -1)
        );
    }
}
