//! FPV video the way TerraPhone sends it, so a dashboard built against the
//! simulator works unchanged on a real rover: each rover answers a dashboard's
//! WebRTC call (Terra `fpv-signaling` on `<prefix>/<id>/webrtc/*`) with H.264
//! from a camera at its front sensor mount. Video only; driving, LOAD and DUMP
//! stay on their own topics.
//!
//! The camera renders and is read back only while a call is live.

mod link;
mod peer;
mod sender;

use crate::depth_camera::DepthCamera;
use crate::terra::RoverId;
use bevy::{
    asset::RenderAssetUsages,
    camera::RenderTarget,
    prelude::*,
    render::{
        gpu_readback::{Readback, ReadbackComplete},
        render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages},
    },
};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// FPV frame size and rate: 4:3 VGA at 15 fps, about what the phone sends.
pub const WIDTH: u32 = 640;
pub const HEIGHT: u32 = 480;
pub const FPS: f32 = 15.0;
/// Vertical field of view of a phone's main camera in landscape, roughly.
const VERTICAL_FOV_DEGREES: f32 = 50.0;

/// The latest camera frame for one rover, and whether a call wants frames.
#[derive(Default)]
pub(crate) struct Feed {
    pub active: AtomicBool,
    pub latest: Mutex<Option<Arc<Vec<u8>>>>,
}

pub struct TerraFpvPlugin {
    /// Zenoh endpoint of the simulator's own router.
    pub endpoint: String,
    pub prefix: String,
    pub user: Option<String>,
    pub password: Option<String>,
}

#[derive(Resource)]
struct Senders {
    runtime: tokio::runtime::Runtime,
    feeds: BTreeMap<u64, Arc<Feed>>,
    endpoint: String,
    prefix: String,
    user: Option<String>,
    password: Option<String>,
    stun: Vec<String>,
    cancel: Arc<AtomicBool>,
}

impl Drop for Senders {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

impl Plugin for TerraFpvPlugin {
    fn build(&self, app: &mut App) {
        let stun = match peer::stun_from_env() {
            Ok(stun) => stun,
            Err(error) => {
                eprintln!("Zorvane FPV disabled: {error}");
                return;
            }
        };
        let runtime = match tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .worker_threads(2)
            .thread_name("zorvane-fpv")
            .build()
        {
            Ok(runtime) => runtime,
            Err(error) => {
                eprintln!("Zorvane FPV disabled: {error}");
                return;
            }
        };
        app.insert_resource(Senders {
            runtime,
            feeds: BTreeMap::new(),
            endpoint: self.endpoint.clone(),
            prefix: self.prefix.clone(),
            user: self.user.clone(),
            password: self.password.clone(),
            stun,
            cancel: Arc::new(AtomicBool::new(false)),
        })
        .add_systems(Update, (mount_fpv_cameras, follow_calls).chain());
    }
}

#[derive(Component)]
struct FpvMounted;

#[derive(Component)]
struct FpvCamera {
    feed: Arc<Feed>,
    target: Handle<Image>,
}

fn mount_fpv_cameras(
    mut commands: Commands,
    mut senders: ResMut<Senders>,
    sensors: Query<(Entity, &DepthCamera, &RoverId), Without<FpvMounted>>,
    mut images: ResMut<Assets<Image>>,
) {
    for (entity, sensor, id) in &sensors {
        let feed = senders.feeds.get(&id.0).cloned().unwrap_or_else(|| {
            let feed = Arc::new(Feed::default());
            senders.feeds.insert(id.0, feed.clone());
            let options = sender::SenderOptions {
                endpoint: senders.endpoint.clone(),
                prefix: senders.prefix.clone(),
                rover: id.0,
                user: senders.user.clone(),
                password: senders.password.clone(),
                stun: senders.stun.clone(),
                feed: feed.clone(),
                cancel: senders.cancel.clone(),
            };
            senders.runtime.spawn(sender::run_sender(options));
            feed
        });
        let mut image = Image::new_fill(
            Extent3d {
                width: WIDTH,
                height: HEIGHT,
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
        commands.entity(entity).insert(FpvMounted);
        commands
            .spawn((
                Name::new("Terra FPV camera"),
                *id,
                FpvCamera {
                    feed,
                    target: target.clone(),
                },
                Camera3d::default(),
                // Robot cameras see simulator geometry, never spectator-only scenery.
                bevy::camera::visibility::RenderLayers::from_layers(&[0, 2]),
                Camera {
                    order: -3,
                    is_active: false,
                    ..default()
                },
                RenderTarget::Image(target.into()),
                Projection::Perspective(PerspectiveProjection {
                    fov: VERTICAL_FOV_DEGREES.to_radians(),
                    near: sensor.config.near,
                    far: 400.0,
                    aspect_ratio: WIDTH as f32 / HEIGHT as f32,
                    ..default()
                }),
                Transform::IDENTITY,
                ChildOf(entity),
            ))
            .observe(receive_frame);
    }
}

/// Renders and reads back a rover's FPV camera only while its call is live.
fn follow_calls(
    mut commands: Commands,
    mut cameras: Query<(Entity, &FpvCamera, &mut Camera, Has<Readback>)>,
) {
    for (entity, fpv, mut camera, reading) in &mut cameras {
        let active = fpv.feed.active.load(Ordering::Relaxed);
        if camera.is_active != active {
            camera.is_active = active;
        }
        if active && !reading {
            commands
                .entity(entity)
                .insert(Readback::texture(fpv.target.clone()));
        } else if !active && reading {
            commands.entity(entity).remove::<Readback>();
            *fpv.feed.latest.lock().unwrap() = None;
        }
    }
}

fn receive_frame(event: On<ReadbackComplete>, cameras: Query<&FpvCamera>) {
    let Ok(fpv) = cameras.get(event.entity) else {
        return;
    };
    if let Some(rgba) = crate::rgb_camera::strip_padding(&event.data, WIDTH, HEIGHT) {
        *fpv.feed.latest.lock().unwrap() = Some(Arc::new(rgba));
    }
}
