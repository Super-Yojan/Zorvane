//! The world camera a person looks through: follow a rover, or fly freely.
//!
//! V switches between following and free flight. Tab follows the next rover and
//! 0 to 7 follow rover 0 to 7. O jumps to the overview of the whole field.
//! In free flight WASD or the arrows move, Q and E go down and up, Shift is
//! faster, the scroll wheel sets the speed and holding the right mouse button
//! looks around. While following, the right button orbits and the wheel zooms.
//! H hides the hint.
//!
//! The camera never drives a rover. In free flight the keyboard leaves rover 0
//! alone, and `ZORVANE_SPECTATOR=1` starts in free flight with keyboard driving
//! off, for watching while agents or remote players drive.
use crate::{
    terra::{Rover, RoverSlot},
    world::{OVERVIEW_EYE, WorldCamera, WorldConfig},
};
use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit};
use bevy::prelude::*;

pub struct SpectatorPlugin;

impl Plugin for SpectatorPlugin {
    fn build(&self, app: &mut App) {
        let follow = app
            .world()
            .get_resource::<WorldConfig>()
            .is_none_or(|config| config.follow_rover);
        app.insert_resource(SpectatorCamera::new(follow, spectator_env()))
            .add_systems(
                PostUpdate,
                (switch_mode, place_camera)
                    .chain()
                    .before(TransformSystems::Propagate),
            )
            .add_systems(Update, update_hint);
    }
}

fn spectator_env() -> bool {
    matches!(
        std::env::var("ZORVANE_SPECTATOR").as_deref(),
        Ok("1") | Ok("true")
    )
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CameraMode {
    /// Chase the rover in this fleet slot.
    Follow(usize),
    /// Fly anywhere, bound to no rover.
    Free,
}

const MIN_SPEED: f32 = 0.5;
const MAX_SPEED: f32 = 80.0;
const LOOK_RATE: f32 = 0.003;
const MIN_ZOOM: f32 = 0.25;
const MAX_ZOOM: f32 = 8.0;
/// The free camera stays this far above the ground.
const MIN_HEIGHT: f32 = 0.15;

#[derive(Resource, Clone, Debug)]
pub struct SpectatorCamera {
    pub mode: CameraMode,
    /// Free flight speed, m/s.
    pub speed: f32,
    /// The keyboard never drives a rover (`ZORVANE_SPECTATOR=1`).
    pub spectator_only: bool,
    pub show_hint: bool,
    yaw: f32,
    pitch: f32,
    /// Follow camera's turn about the rover, radians, and distance scale.
    orbit: f32,
    zoom: f32,
    last_followed: usize,
}

impl SpectatorCamera {
    pub fn new(follow: bool, spectator_only: bool) -> Self {
        Self {
            mode: if follow && !spectator_only {
                CameraMode::Follow(0)
            } else {
                CameraMode::Free
            },
            speed: 8.0,
            spectator_only,
            show_hint: true,
            yaw: 0.0,
            pitch: 0.0,
            orbit: 0.0,
            zoom: 1.0,
            last_followed: 0,
        }
    }

    /// Whether WASD may drive the keyboard rover.
    pub fn keyboard_drives(&self) -> bool {
        !self.spectator_only && self.mode != CameraMode::Free
    }

    fn fly_from(&mut self, camera: &Transform) {
        let (yaw, pitch, _) = camera.rotation.to_euler(EulerRot::YXZ);
        self.yaw = yaw;
        self.pitch = pitch;
        self.mode = CameraMode::Free;
    }

    fn follow(&mut self, slot: usize) {
        self.mode = CameraMode::Follow(slot);
        self.last_followed = slot;
    }
}

/// Fleet slots present, in order.
fn slots(
    rovers: &Query<(&Transform, Option<&RoverSlot>), (With<Rover>, Without<WorldCamera>)>,
) -> Vec<usize> {
    let mut slots: Vec<usize> = rovers
        .iter()
        .filter_map(|(_, slot)| slot.map(|s| s.0))
        .collect();
    slots.sort_unstable();
    slots
}

fn switch_mode(
    keys: Option<Res<ButtonInput<KeyCode>>>,
    mut state: ResMut<SpectatorCamera>,
    rovers: Query<(&Transform, Option<&RoverSlot>), (With<Rover>, Without<WorldCamera>)>,
    cameras: Query<&mut Transform, (With<WorldCamera>, Without<Rover>)>,
    cutscene: Option<Res<crate::cutscene::CutsceneActive>>,
) {
    let Some(keys) = keys else { return };
    if cutscene.is_some() {
        return;
    }
    let mut cameras = cameras;
    let Ok(mut camera) = cameras.single_mut() else {
        return;
    };
    if keys.just_pressed(KeyCode::KeyH) {
        state.show_hint = !state.show_hint;
    }
    if keys.just_pressed(KeyCode::KeyO) {
        *camera = Transform::from_translation(OVERVIEW_EYE).looking_at(Vec3::ZERO, Vec3::Y);
        state.fly_from(&camera);
        return;
    }
    if keys.just_pressed(KeyCode::KeyV) {
        match state.mode {
            CameraMode::Follow(_) => state.fly_from(&camera),
            CameraMode::Free => {
                let slot = state.last_followed;
                state.follow(slot);
            }
        }
        return;
    }
    let present = slots(&rovers);
    if keys.just_pressed(KeyCode::Tab) && !present.is_empty() {
        let next = match state.mode {
            CameraMode::Follow(slot) => present
                .iter()
                .copied()
                .find(|&s| s > slot)
                .unwrap_or(present[0]),
            CameraMode::Free => state.last_followed,
        };
        state.follow(next);
        return;
    }
    const DIGITS: [KeyCode; 8] = [
        KeyCode::Digit0,
        KeyCode::Digit1,
        KeyCode::Digit2,
        KeyCode::Digit3,
        KeyCode::Digit4,
        KeyCode::Digit5,
        KeyCode::Digit6,
        KeyCode::Digit7,
    ];
    for (slot, key) in DIGITS.into_iter().enumerate() {
        if keys.just_pressed(key) && present.contains(&slot) {
            state.follow(slot);
        }
    }
}

/// Mouse wheel steps since last frame; trackpad pixels count as a fraction of a step.
fn scroll_steps(scroll: Option<&AccumulatedMouseScroll>) -> f32 {
    scroll.map_or(0.0, |scroll| match scroll.unit {
        MouseScrollUnit::Line => scroll.delta.y,
        MouseScrollUnit::Pixel => scroll.delta.y / 40.0,
    })
}

#[allow(clippy::too_many_arguments)]
fn place_camera(
    config: Res<WorldConfig>,
    time: Res<Time>,
    real: Option<Res<Time<bevy::time::Real>>>,
    keys: Option<Res<ButtonInput<KeyCode>>>,
    buttons: Option<Res<ButtonInput<MouseButton>>>,
    motion: Option<Res<AccumulatedMouseMotion>>,
    scroll: Option<Res<AccumulatedMouseScroll>>,
    mut state: ResMut<SpectatorCamera>,
    rovers: Query<(&Transform, Option<&RoverSlot>), (With<Rover>, Without<WorldCamera>)>,
    mut cameras: Query<&mut Transform, (With<WorldCamera>, Without<Rover>)>,
    cutscene: Option<Res<crate::cutscene::CutsceneActive>>,
) {
    // The NEXT intro cutscene drives the camera while it plays.
    if cutscene.is_some() {
        return;
    }
    let looking = buttons
        .as_ref()
        .is_some_and(|buttons| buttons.pressed(MouseButton::Right));
    let drag = match (&motion, looking) {
        (Some(motion), true) => motion.delta,
        _ => Vec2::ZERO,
    };
    let steps = scroll_steps(scroll.as_deref());
    match state.mode {
        CameraMode::Follow(slot) => {
            // A missing slot (fleet shrank) falls back to slot zero, then to a lone rover.
            let Some((rover, _)) = rovers
                .iter()
                .find(|(_, s)| s.is_some_and(|s| s.0 == slot))
                .or_else(|| rovers.iter().find(|(_, s)| s.is_some_and(|s| s.0 == 0)))
                .or_else(|| rovers.single().ok())
            else {
                return;
            };
            state.orbit -= drag.x * LOOK_RATE;
            state.zoom = (state.zoom * 0.9_f32.powf(steps)).clamp(MIN_ZOOM, MAX_ZOOM);
            let offset = Quat::from_rotation_y(state.orbit) * config.camera_offset * state.zoom;
            let blend = 1.0 - (-config.camera_response * time.delta_secs()).exp();
            for mut camera in &mut cameras {
                camera.translation = camera.translation.lerp(rover.translation + offset, blend);
                camera.look_at(rover.translation, Vec3::Y);
            }
        }
        CameraMode::Free => {
            // Real time, so a paused match can still be looked around.
            let dt = real.map_or(time.delta_secs(), |real| real.delta_secs());
            state.speed = (state.speed * 1.2_f32.powf(steps)).clamp(MIN_SPEED, MAX_SPEED);
            state.yaw -= drag.x * LOOK_RATE;
            state.pitch = (state.pitch - drag.y * LOOK_RATE).clamp(-1.54, 1.54);
            let rotation = Quat::from_euler(EulerRot::YXZ, state.yaw, state.pitch, 0.0);
            let wish = keys.as_deref().map_or(Vec3::ZERO, |keys| {
                let axis = |pos: [KeyCode; 2], neg: [KeyCode; 2]| {
                    keys.any_pressed(pos) as i32 as f32 - keys.any_pressed(neg) as i32 as f32
                };
                let forward = (rotation * Vec3::NEG_Z).with_y(0.0).normalize_or_zero();
                let right = (rotation * Vec3::X).with_y(0.0).normalize_or_zero();
                let fast = if keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]) {
                    4.0
                } else {
                    1.0
                };
                (forward
                    * axis(
                        [KeyCode::KeyW, KeyCode::ArrowUp],
                        [KeyCode::KeyS, KeyCode::ArrowDown],
                    )
                    + right
                        * axis(
                            [KeyCode::KeyD, KeyCode::ArrowRight],
                            [KeyCode::KeyA, KeyCode::ArrowLeft],
                        )
                    + Vec3::Y
                        * axis(
                            [KeyCode::KeyE, KeyCode::PageUp],
                            [KeyCode::KeyQ, KeyCode::PageDown],
                        ))
                .normalize_or_zero()
                    * fast
            });
            let reach = config.size * 0.5 + 50.0;
            for mut camera in &mut cameras {
                let moved = camera.translation + wish * state.speed * dt;
                camera.translation = Vec3::new(
                    moved.x.clamp(-reach, reach),
                    moved.y.clamp(MIN_HEIGHT, reach),
                    moved.z.clamp(-reach, reach),
                );
                camera.rotation = rotation;
            }
        }
    }
}

#[derive(Component)]
struct CameraHint;

fn hint_text(state: &SpectatorCamera, keyboard_rover: bool) -> String {
    let drive = if keyboard_rover && state.keyboard_drives() {
        "WASD drives rover 0"
    } else {
        "keyboard does not drive"
    };
    match state.mode {
        CameraMode::Follow(slot) => format!(
            "Camera: following rover {}  ({drive})\n\
             V free camera · Tab next rover · 0-7 pick rover · O overview\n\
             Right-drag orbit · wheel zoom · H hide",
            slot
        ),
        CameraMode::Free => format!(
            "Camera: free, {:.0} m/s  ({drive})\n\
             WASD move · Q/E down/up · Shift fast · right-drag look · wheel speed\n\
             V follow · Tab/0-7 follow a rover · O overview · H hide",
            state.speed
        ),
    }
}

fn update_hint(
    mut commands: Commands,
    state: Res<SpectatorCamera>,
    session: Option<Res<crate::session::SimSession>>,
    cutscene: Option<Res<crate::cutscene::CutsceneActive>>,
    cameras: Query<(), With<WorldCamera>>,
    keyboard_rovers: Query<(), With<crate::terra::KeyboardControlled>>,
    mut hints: Query<(&mut Text, &mut Visibility), With<CameraHint>>,
) {
    let shown = state.show_hint
        && cutscene.is_none()
        && !cameras.is_empty()
        && session.is_none_or(|session| session.phase != crate::session::Phase::Menu);
    let text = hint_text(&state, !keyboard_rovers.is_empty());
    let Ok((mut current, mut visibility)) = hints.single_mut() else {
        commands.spawn((
            Name::new("Camera hint"),
            CameraHint,
            Text::new(text),
            TextFont {
                font_size: bevy::text::FontSize::Px(13.0),
                ..default()
            },
            TextColor(Color::srgba(1.0, 1.0, 1.0, 0.85)),
            BackgroundColor(Color::srgba(0.05, 0.07, 0.09, 0.6)),
            Node {
                position_type: PositionType::Absolute,
                bottom: Val::Px(12.0),
                left: Val::Px(12.0),
                padding: UiRect::all(Val::Px(6.0)),
                ..default()
            },
            if shown {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            },
        ));
        return;
    };
    if current.0 != text {
        current.0 = text;
    }
    let wanted = if shown {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    if *visibility != wanted {
        *visibility = wanted;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> App {
        let mut app = App::new();
        app.insert_resource(Time::<()>::default())
            .insert_resource(ButtonInput::<KeyCode>::default())
            .insert_resource(WorldConfig::default())
            .add_plugins(SpectatorPlugin);
        app.world_mut().spawn((
            WorldCamera,
            Transform::from_xyz(0.0, 3.0, 5.0).looking_at(Vec3::ZERO, Vec3::Y),
        ));
        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(std::time::Duration::from_millis(500));
        app
    }

    fn press(app: &mut App, key: KeyCode) {
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(key);
        app.update();
        let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
        keys.release(key);
        keys.clear();
    }

    fn camera(app: &mut App) -> Transform {
        let world = app.world_mut();
        *world
            .query_filtered::<&Transform, With<WorldCamera>>()
            .single(world)
            .unwrap()
    }

    #[test]
    fn spectator_env_starts_free_without_keyboard_drive() {
        let state = SpectatorCamera::new(true, true);
        assert_eq!(state.mode, CameraMode::Free);
        assert!(!state.keyboard_drives());
        let mut state = SpectatorCamera::new(true, false);
        assert!(state.keyboard_drives());
        state.mode = CameraMode::Free;
        assert!(!state.keyboard_drives());
    }

    #[test]
    fn v_toggles_free_flight_and_wasd_moves_the_camera() {
        let mut app = app();
        app.world_mut()
            .spawn((Rover, RoverSlot(0), Transform::from_xyz(10.0, 0.0, -10.0)));
        app.update();
        press(&mut app, KeyCode::KeyV);
        assert_eq!(
            app.world().resource::<SpectatorCamera>().mode,
            CameraMode::Free
        );
        let before = camera(&mut app).translation;
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyW);
        app.update();
        let after = camera(&mut app).translation;
        assert!(
            after.distance(before) > 0.5,
            "camera should fly: {before} -> {after}"
        );
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .release(KeyCode::KeyW);
        press(&mut app, KeyCode::KeyV);
        assert_eq!(
            app.world().resource::<SpectatorCamera>().mode,
            CameraMode::Follow(0)
        );
    }

    #[test]
    fn tab_and_digits_follow_present_rovers() {
        let mut app = app();
        for slot in [0, 1, 3] {
            app.world_mut().spawn((
                Rover,
                RoverSlot(slot),
                Transform::from_xyz(slot as f32 * 20.0, 0.0, 0.0),
            ));
        }
        press(&mut app, KeyCode::Tab);
        assert_eq!(
            app.world().resource::<SpectatorCamera>().mode,
            CameraMode::Follow(1)
        );
        press(&mut app, KeyCode::Tab);
        assert_eq!(
            app.world().resource::<SpectatorCamera>().mode,
            CameraMode::Follow(3)
        );
        press(&mut app, KeyCode::Tab);
        assert_eq!(
            app.world().resource::<SpectatorCamera>().mode,
            CameraMode::Follow(0)
        );
        // Rover 2 is absent, so the camera stays put.
        press(&mut app, KeyCode::Digit2);
        assert_eq!(
            app.world().resource::<SpectatorCamera>().mode,
            CameraMode::Follow(0)
        );
        press(&mut app, KeyCode::Digit3);
        assert_eq!(
            app.world().resource::<SpectatorCamera>().mode,
            CameraMode::Follow(3)
        );
        for _ in 0..20 {
            app.update();
        }
        assert!(camera(&mut app).translation.x > 50.0);
    }

    #[test]
    fn overview_key_frees_the_camera_high_over_the_field() {
        let mut app = app();
        press(&mut app, KeyCode::KeyO);
        assert_eq!(
            app.world().resource::<SpectatorCamera>().mode,
            CameraMode::Free
        );
        assert!(camera(&mut app).translation.y > 30.0);
    }
}
