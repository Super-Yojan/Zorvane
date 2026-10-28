//! NEXT intro cutscene: a narrated camera tour of the field that explains the
//! NEXT Competition 2026 rules, then hands the camera back to the rover.
//!
//! The script (captions, narration, clip timings) is
//! `assets/cutscene/next_intro.json`; `tools/cutscene/make_voiceover.py`
//! regenerates the voice clips from it. Each beat names a shot, and the shot's
//! camera path is computed from the live field layout. Enter skips. Headless
//! runs and `ZORVANE_CUTSCENE=0` never play it.

use crate::next_competition::{
    BUCKETS_PER_HALF, PENALTY_AREA_DEPTH, PITCH_SOUTH, TennisBall, bucket_seed, buckets,
};
use crate::world::{WorldCamera, WorldConfig};
use bevy::prelude::*;
use serde::Deserialize;

const SCRIPT: &str = include_str!("../assets/cutscene/next_intro.json");
/// Red alliance Base centre (south end, west of the goal), as authored in
/// `tools/blender/build_next_assets.py` (`BASE_CENTRES`), in Bevy coordinates.
const RED_BASE: Vec3 = Vec3::new(-18.0, 0.0, PITCH_SOUTH + 6.0);

#[derive(Deserialize, Clone, Debug)]
struct Script {
    title: String,
    beats: Vec<BeatScript>,
}

#[derive(Deserialize, Clone, Debug)]
struct BeatScript {
    shot: Shot,
    caption: String,
    narration: String,
    audio: String,
    seconds: f32,
}

#[derive(Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Shot {
    Aerial,
    Sweep,
    RedBase,
    Balls,
    OrangeBucket,
    ElevatedBucket,
    Tag,
    Penalty,
    Rover,
}

/// A camera move: eye and look target, each eased from start to end.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Path {
    eye: (Vec3, Vec3),
    look: (Vec3, Vec3),
}

impl Path {
    fn at(self, t: f32) -> Transform {
        let s = t.clamp(0.0, 1.0);
        let eased = s * s * (3.0 - 2.0 * s);
        let eye = self.eye.0.lerp(self.eye.1, eased);
        let look = self.look.0.lerp(self.look.1, eased);
        Transform::from_translation(eye).looking_at(look, Vec3::Y)
    }
}

/// Where the shots point, from the field as built.
#[derive(Clone, Copy, Debug)]
struct Layout {
    orange_ground: Vec3,
    orange_elevated: Vec3,
    blue_ground: Vec3,
    rover: Vec3,
}

impl Layout {
    fn current(rover: Option<Vec3>) -> Self {
        let places = buckets(bucket_seed());
        // The first ten are the south half (blue), the rest the north (orange).
        let pick = |orange: bool, elevated: bool| {
            places
                .iter()
                .enumerate()
                .filter(|(index, place)| (*index >= BUCKETS_PER_HALF) == orange && place.elevated == elevated)
                // The one nearest midfield keeps the shot inside the field.
                .min_by(|(_, a), (_, b)| a.z.abs().total_cmp(&b.z.abs()))
                .map(|(_, place)| Vec3::new(place.x, place.base_y(), place.z))
                .unwrap_or(Vec3::ZERO)
        };
        Self {
            orange_ground: pick(true, false),
            orange_elevated: pick(true, true),
            blue_ground: pick(false, false),
            rover: rover.unwrap_or(Vec3::new(0.0, 0.3, PITCH_SOUTH - 0.4)),
        }
    }
}

fn path(shot: Shot, layout: Layout) -> Path {
    let up = |v: Vec3, y: f32| v + Vec3::Y * y;
    match shot {
        Shot::Aerial => Path {
            eye: (Vec3::new(-150.0, 90.0, 160.0), Vec3::new(-85.0, 52.0, 95.0)),
            look: (Vec3::ZERO, Vec3::new(0.0, 0.0, 5.0)),
        },
        Shot::Sweep => Path {
            eye: (Vec3::new(-42.0, 7.0, 32.0), Vec3::new(42.0, 7.0, -28.0)),
            look: (Vec3::new(0.0, 0.5, -12.0), Vec3::new(0.0, 0.5, 12.0)),
        },
        Shot::RedBase => Path {
            eye: (up(RED_BASE, 3.2) + Vec3::new(12.0, 0.0, -14.0), up(RED_BASE, 2.0) + Vec3::new(6.0, 0.0, -7.5)),
            look: (up(RED_BASE, 1.2), up(RED_BASE, 1.1)),
        },
        Shot::Balls => {
            let box_centre = Vec3::new(0.0, 0.05, PITCH_SOUTH - PENALTY_AREA_DEPTH * 0.6);
            Path {
                eye: (box_centre + Vec3::new(9.0, 2.2, -10.0), box_centre + Vec3::new(3.0, 0.9, -4.0)),
                look: (box_centre, box_centre + Vec3::new(-2.0, 0.0, 2.0)),
            }
        }
        Shot::OrangeBucket => {
            let b = layout.orange_ground;
            Path {
                eye: (b + Vec3::new(3.5, 1.8, 3.5), b + Vec3::new(1.3, 0.75, 1.5)),
                look: (up(b, 0.2), up(b, 0.18)),
            }
        }
        Shot::ElevatedBucket => {
            let b = layout.orange_elevated;
            Path {
                eye: (b + Vec3::new(-3.5, 1.2, 3.5), b + Vec3::new(-1.5, 0.6, 1.6)),
                look: (up(b, 0.2), up(b, 0.15)),
            }
        }
        Shot::Tag => Path {
            eye: (Vec3::new(30.0, 3.5, 8.0), Vec3::new(18.0, 2.5, -8.0)),
            look: (Vec3::new(0.0, 0.5, -2.0), Vec3::new(-4.0, 0.5, -18.0)),
        },
        Shot::Penalty => {
            let b = layout.blue_ground;
            Path {
                eye: (b + Vec3::new(-4.0, 1.6, -3.0), b + Vec3::new(-1.6, 0.8, -1.4)),
                look: (up(b, 0.2), up(b, 0.18)),
            }
        }
        // The rover faces −Z, into the field; film it from the front.
        Shot::Rover => {
            let r = layout.rover;
            Path {
                eye: (r + Vec3::new(0.0, 2.0, -8.0), r + Vec3::new(1.4, 0.9, -2.8)),
                look: (up(r, 0.2), up(r, 0.25)),
            }
        }
    }
}

/// Present while the cutscene owns the world camera.
#[derive(Resource)]
pub struct CutsceneActive;

#[derive(Resource)]
struct Playing {
    beats: Vec<BeatScript>,
    index: usize,
    elapsed: f32,
    path: Path,
}

#[derive(Component)]
struct CutsceneUi;
#[derive(Component)]
struct CaptionText;
#[derive(Component)]
struct NarrationText;
#[derive(Component)]
struct CutsceneVoice;

#[derive(Resource)]
struct IntroScript(Script);

pub fn install(app: &mut App) {
    // ZORVANE_CUTSCENE is checked per field, so the start menu can turn it off.
    if std::env::var("TERRA_HEADLESS").as_deref() == Ok("1") {
        return;
    }
    let script: Script = serde_json::from_str(SCRIPT).expect("invalid NEXT intro script");
    app.insert_resource(IntroScript(script))
        .add_systems(Update, (start, advance).chain());
}

/// Starts each time a NEXT field is spawned (startup, Restart, Load world).
fn start(
    mut commands: Commands,
    script: Res<IntroScript>,
    config: Option<Res<WorldConfig>>,
    new_field: Query<(), Added<TennisBall>>,
    cameras: Query<(), With<WorldCamera>>,
    playing: Option<Res<Playing>>,
) {
    if new_field.is_empty()
        || crate::settings::env("ZORVANE_CUTSCENE").as_deref() == Some("0")
        || playing.is_some()
        || cameras.is_empty()
        || config.is_none()
    {
        return;
    }
    let first = script.0.beats[0].shot;
    commands.insert_resource(Playing {
        beats: script.0.beats.clone(),
        index: usize::MAX,
        elapsed: 0.0,
        path: path(first, Layout::current(None)),
    });
    commands.insert_resource(CutsceneActive);
    spawn_ui(&mut commands, &script.0.title);
}

fn spawn_ui(commands: &mut Commands, title: &str) {
    let text = |value: &str, size: f32, color: Color| {
        (
            Text::new(value),
            TextFont {
                font_size: FontSize::Px(size),
                ..default()
            },
            TextColor(color),
        )
    };
    commands
        .spawn((
            CutsceneUi,
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                ..default()
            },
        ))
        .with_children(|root| {
            // Captions sit at the top: the bottom corner holds the depth inset.
            root.spawn((
                Node {
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    row_gap: Val::Px(8.0),
                    padding: UiRect::axes(Val::Px(48.0), Val::Px(18.0)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.55)),
            ))
            .with_children(|band| {
                band.spawn(Node {
                    width: Val::Percent(100.0),
                    justify_content: JustifyContent::SpaceBetween,
                    ..default()
                })
                .with_children(|row| {
                    row.spawn(text(title, 16.0, Color::srgba(1.0, 1.0, 1.0, 0.8)));
                    row.spawn(text("Enter to skip", 15.0, Color::srgba(1.0, 1.0, 1.0, 0.7)));
                });
                band.spawn((CaptionText, text("", 30.0, Color::WHITE)));
                band.spawn((
                    NarrationText,
                    text("", 18.0, Color::srgb(0.86, 0.88, 0.9)),
                ));
            });
        });
}

#[allow(clippy::too_many_arguments)]
fn advance(
    mut commands: Commands,
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    assets: Option<Res<AssetServer>>,
    playing: Option<ResMut<Playing>>,
    rovers: Query<(&Transform, &crate::terra::RoverSlot), (With<crate::terra::Rover>, Without<WorldCamera>)>,
    mut cameras: Query<&mut Transform, With<WorldCamera>>,
    mut captions: Query<&mut Text, (With<CaptionText>, Without<NarrationText>)>,
    mut narration: Query<&mut Text, (With<NarrationText>, Without<CaptionText>)>,
    ui: Query<Entity, With<CutsceneUi>>,
    voices: Query<Entity, With<CutsceneVoice>>,
) {
    let Some(mut playing) = playing else {
        return;
    };
    let skip = keys.just_pressed(KeyCode::Enter) || keys.just_pressed(KeyCode::NumpadEnter);
    if playing.index != usize::MAX {
        playing.elapsed += time.delta_secs();
    }
    let beat_over = playing.index == usize::MAX
        || playing.elapsed >= playing.beats[playing.index].seconds;
    if beat_over {
        let next = playing.index.wrapping_add(1);
        if skip || next >= playing.beats.len() {
            finish(&mut commands, &ui, &voices);
            return;
        }
        playing.index = next;
        playing.elapsed = 0.0;
        let beat = playing.beats[next].clone();
        let rover = rovers
            .iter()
            .find(|(_, slot)| slot.0 == 0)
            .map(|(transform, _)| transform.translation);
        playing.path = path(beat.shot, Layout::current(rover));
        for entity in &voices {
            commands.entity(entity).despawn();
        }
        if let Some(assets) = &assets {
            commands.spawn((
                CutsceneVoice,
                AudioPlayer::new(assets.load(beat.audio.clone())),
                PlaybackSettings::DESPAWN,
            ));
        }
        for mut text in &mut captions {
            text.0 = beat.caption.clone();
        }
        for mut text in &mut narration {
            text.0 = beat.narration.clone();
        }
    } else if skip {
        finish(&mut commands, &ui, &voices);
        return;
    }
    let beat = &playing.beats[playing.index];
    let pose = playing.path.at(playing.elapsed / beat.seconds.max(0.1));
    for mut camera in &mut cameras {
        *camera = pose;
    }
}

fn finish(
    commands: &mut Commands,
    ui: &Query<Entity, With<CutsceneUi>>,
    voices: &Query<Entity, With<CutsceneVoice>>,
) {
    for entity in ui.iter().chain(voices.iter()) {
        commands.entity(entity).despawn();
    }
    commands.remove_resource::<Playing>();
    commands.remove_resource::<CutsceneActive>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_parses_and_every_beat_has_a_clip_and_a_shot() {
        let script: Script = serde_json::from_str(SCRIPT).unwrap();
        assert_eq!(script.beats.first().map(|beat| beat.shot), Some(Shot::Aerial));
        assert_eq!(script.beats.last().map(|beat| beat.shot), Some(Shot::Rover));
        let assets = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets");
        for beat in &script.beats {
            assert!(beat.seconds > 1.0 && beat.seconds < 20.0, "{beat:?}");
            assert!(!beat.caption.is_empty() && !beat.narration.is_empty());
            assert!(assets.join(&beat.audio).is_file(), "missing {}", beat.audio);
        }
    }

    #[test]
    fn shots_frame_their_subject_inside_the_world() {
        let layout = Layout::current(None);
        let half = crate::next_competition::WORLD_SIZE * 0.5;
        for shot in [
            Shot::Sweep,
            Shot::RedBase,
            Shot::Balls,
            Shot::OrangeBucket,
            Shot::ElevatedBucket,
            Shot::Tag,
            Shot::Penalty,
            Shot::Rover,
        ] {
            let path = path(shot, layout);
            for t in [0.0, 0.5, 1.0] {
                let eye = path.at(t).translation;
                assert!(eye.y > 0.3, "{shot:?} eye below turf: {eye}");
                assert!(eye.x.abs() < half && eye.z.abs() < half, "{shot:?} eye off the field: {eye}");
            }
        }
        // Orange buckets are Red's targets on the north half (rules 3.2b).
        assert!(layout.orange_ground.z < 0.0 && layout.orange_elevated.z < 0.0);
        assert!(layout.blue_ground.z > 0.0);
        assert!((layout.orange_elevated.y - crate::next_competition::BUCKET_STAND_HEIGHT).abs() < 1e-6);
    }
}
