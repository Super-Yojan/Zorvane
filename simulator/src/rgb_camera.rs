//! RGB camera colocated with the front depth sensor, with asynchronous readback.
use crate::depth_camera::{DepthCamera, DepthCameraConfig};
use bevy::{
    asset::RenderAssetUsages,
    camera::RenderTarget,
    prelude::*,
    render::{
        gpu_readback::{Readback, ReadbackComplete},
        render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages},
    },
};

pub struct TerraRgbCameraPlugin;
impl Plugin for TerraRgbCameraPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(RgbPreviewConfig {
            enabled: std::env::var("ZORVANE_RGB_PREVIEW")
                .is_ok_and(|value| value == "1" || value.eq_ignore_ascii_case("true")),
        })
        .add_systems(
            Update,
            (
                mount_rgb_cameras,
                sync_rgb_previews.after(mount_rgb_cameras),
            ),
        );
    }
}
#[derive(Resource)]
struct RgbPreviewConfig {
    enabled: bool,
}
#[derive(Component)]
struct RgbMounted;
#[derive(Component)]
struct RgbPreview(Entity);
#[derive(Component)]
pub struct RgbCamera {
    pub config: DepthCameraConfig,
    pub target: Handle<Image>,
}
#[derive(Component, Default)]
pub struct RgbFrame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub sequence: u64,
    pub received_at: f64,
    pub(crate) last_observation: f64,
}
fn mount_rgb_cameras(
    mut commands: Commands,
    sensors: Query<(Entity, &DepthCamera, &crate::terra::RoverId), Without<RgbMounted>>,
    mut images: ResMut<Assets<Image>>,
) {
    for (entity, sensor, id) in &sensors {
        let config = &sensor.config;
        let mut image = Image::new_fill(
            Extent3d {
                width: config.resolution.x,
                height: config.resolution.y,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            &[0, 0, 0, 255],
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::default(),
        );
        image.texture_descriptor.usage |=
            TextureUsages::COPY_SRC | TextureUsages::RENDER_ATTACHMENT;
        let target = images.add(image);
        commands.entity(entity).insert(RgbMounted);
        commands
            .spawn((
                Name::new("Terra front RGB camera"),
                *id,
                RgbCamera {
                    config: config.clone(),
                    target: target.clone(),
                },
                RgbFrame {
                    width: config.resolution.x,
                    height: config.resolution.y,
                    ..default()
                },
                Camera3d::default(),
                bevy::camera::visibility::RenderLayers::from_layers(&[0, 2]),
                Camera {
                    order: -2,
                    ..default()
                },
                RenderTarget::Image(target.clone().into()),
                Projection::Perspective(PerspectiveProjection {
                    fov: config.vertical_fov,
                    near: config.near,
                    far: config.far,
                    aspect_ratio: config.resolution.x as f32 / config.resolution.y as f32,
                    ..default()
                }),
                Msaa::Off,
                Transform::IDENTITY,
                ChildOf(entity),
                Readback::texture(target),
            ))
            .observe(receive_rgb);
    }
}

fn sync_rgb_previews(
    mut commands: Commands,
    config: Res<RgbPreviewConfig>,
    cameras: Query<(Entity, &RgbCamera, &crate::terra::RoverId)>,
    previews: Query<(Entity, &RgbPreview)>,
) {
    for (entity, preview) in &previews {
        if !config.enabled || cameras.get(preview.0).is_err() {
            commands.entity(entity).despawn();
        }
    }
    if !config.enabled {
        return;
    }
    for (camera_entity, camera, id) in &cameras {
        if id.0 != 0
            || previews
                .iter()
                .any(|(_, preview)| preview.0 == camera_entity)
        {
            continue;
        }
        commands
            .spawn((
                Name::new("Front RGB preview"),
                RgbPreview(camera_entity),
                Node {
                    position_type: PositionType::Absolute,
                    left: px(16.0),
                    bottom: px(16.0),
                    flex_direction: FlexDirection::Column,
                    padding: UiRect::all(px(8.0)),
                    row_gap: px(6.0),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.02, 0.02, 0.02, 0.9)),
            ))
            .with_children(|parent| {
                parent.spawn((
                    Text::new("Front RGB · rover 0"),
                    TextFont {
                        font_size: FontSize::Px(14.0),
                        ..default()
                    },
                ));
                parent.spawn((
                    ImageNode::new(camera.target.clone()),
                    Node {
                        width: px(256.0),
                        height: px(256.0 * camera.config.resolution.y as f32
                            / camera.config.resolution.x as f32),
                        ..default()
                    },
                ));
            });
    }
}
pub(crate) fn strip_padding(data: &[u8], width: u32, height: u32) -> Option<Vec<u8>> {
    if width == 0 || height == 0 {
        return None;
    }
    let row = (width as usize).checked_mul(4)?;
    let stride = row.div_ceil(256).checked_mul(256)?;
    if data.len() != stride.checked_mul(height as usize)? {
        return None;
    }
    Some(
        data.chunks_exact(stride)
            .flat_map(|row_data| row_data[..row].iter().copied())
            .collect(),
    )
}
fn receive_rgb(
    event: On<ReadbackComplete>,
    time: Res<Time>,
    mut frames: Query<(&mut RgbFrame, &crate::terra::RoverId)>,
) {
    let Ok((mut frame, id)) = frames.get_mut(event.entity) else {
        return;
    };
    let Some(rgba) = strip_padding(&event.data, frame.width, frame.height) else {
        return;
    };
    frame.rgba = rgba;
    frame.sequence += 1;
    frame.received_at = time.elapsed_secs_f64();
    if frame.received_at - frame.last_observation >= crate::observation_capture::evidence_period() {
        crate::observation_capture::capture_rgb(id.0, &frame);
        frame.last_observation = frame.received_at;
    }
}

#[cfg(test)]
mod preview_tests {
    use super::*;

    #[test]
    fn rgb_preview_follows_flag() {
        let mut app = App::new();
        app.insert_resource(RgbPreviewConfig { enabled: true })
            .add_systems(Update, sync_rgb_previews);
        app.world_mut().spawn((
            RgbCamera {
                config: DepthCameraConfig::default(),
                target: default(),
            },
            crate::terra::RoverId(0),
        ));
        let count = |app: &mut App| {
            let mut query = app.world_mut().query::<&RgbPreview>();
            query.iter(app.world()).count()
        };
        app.update();
        assert_eq!(count(&mut app), 1);
        app.world_mut().resource_mut::<RgbPreviewConfig>().enabled = false;
        app.update();
        assert_eq!(count(&mut app), 0);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn removes_gpu_row_padding_without_flipping_or_changing_channels() {
        let mut padded = vec![0; 512];
        padded[..8].copy_from_slice(&[1, 2, 3, 255, 4, 5, 6, 255]);
        padded[256..264].copy_from_slice(&[7, 8, 9, 255, 10, 11, 12, 255]);
        assert_eq!(
            strip_padding(&padded, 2, 2).unwrap(),
            [1, 2, 3, 255, 4, 5, 6, 255, 7, 8, 9, 255, 10, 11, 12, 255]
        );
        assert!(strip_padding(&padded[..260], 2, 2).is_none());
    }
}
