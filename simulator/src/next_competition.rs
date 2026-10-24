//! NEXT competition field for Zatara.
//!
//! A full soccer pitch at the FIFA Laws of the Game recommended international
//! size, with tennis balls and open-top deposit buckets. The primary rover
//! is named Zatara. It is spawn slot 0 of the active body, which defaults to
//! `terra-ground` (Terra's chassis and differential drive). Zatara is not a
//! separate vehicle type. Rovers line up on the goal lines, facing into the field.

use avian3d::prelude::*;
use bevy::prelude::*;

/// NEXT Competition 2026 Rules 2.1: a regulation soccer field, approximately
/// 110 × 70 yards (100 m × 64 m).
pub const PITCH_LENGTH: f32 = 100.0;
/// NEXT 2026 Rules 2.1.
pub const PITCH_WIDTH: f32 = 64.0;
/// Centre circle radius, metres (10 yards).
pub const CENTER_CIRCLE_RADIUS: f32 = 9.15;
/// Penalty area depth from the goal line, metres (18 yards).
pub const PENALTY_AREA_DEPTH: f32 = 16.5;
/// Penalty area width, metres (44 yards: 16.5 + 7.32 + 16.5).
pub const PENALTY_AREA_WIDTH: f32 = 40.32;
/// Goal area depth from the goal line, metres (6 yards).
pub const GOAL_AREA_DEPTH: f32 = 5.5;
/// Goal area width, metres (20 yards: 5.5 + 7.32 + 5.5).
pub const GOAL_AREA_WIDTH: f32 = 18.32;
/// Square ground that contains Field 3's turf (about 74 × 123 m), the alliance
/// Bases and the perimeter fence.
pub const WORLD_SIZE: f32 = 140.0;
/// Pitch half-width along Bevy X, metres.
pub const PITCH_HALF_WIDTH: f32 = PITCH_WIDTH * 0.5;
/// South goal line. The pitch is centred on the origin.
pub const PITCH_SOUTH: f32 = PITCH_LENGTH * 0.5;
/// North goal line.
pub const PITCH_NORTH: f32 = -PITCH_LENGTH * 0.5;
/// FIFA Law 1 goal width, metres (8 yards). Implied by the 40.32 m penalty area.
pub const GOAL_WIDTH: f32 = 7.32;
/// FIFA Law 1 goal height, metres (8 feet).
pub const GOAL_HEIGHT: f32 = 2.44;
/// Post and crossbar width. Law 1 allows at most 12 cm; the section is otherwise unspecified.
pub const GOAL_POST_WIDTH: f32 = 0.12;
const BOARD_HEIGHT: f32 = 0.28;
const BOARD_THICKNESS: f32 = 0.12;
const LINE_WIDTH: f32 = 0.08;
const MARKING_Y: f32 = 0.008;

/// Regulation tennis ball, metres and kilograms.
pub const TENNIS_BALL_RADIUS: f32 = 0.0335;
pub const TENNIS_BALL_MASS: f32 = 0.058;

/// 5-gallon bucket, from the user-supplied product diagram (inches × 0.0254).
/// Open top. The diagram’s lid is not part of the model.
pub const BUCKET_TOP_DIAMETER: f32 = 0.3048;
pub const BUCKET_BASE_DIAMETER: f32 = 0.3023;
/// Body height. There is no lid.
pub const BUCKET_BODY_HEIGHT: f32 = 0.3399;
/// Shell thickness so the open top has an interior. Not on the product diagram.
const BUCKET_WALL: f32 = 0.0025;
/// NEXT 2026 Rules 3.2: 10 orange and 10 blue buckets, one colour per half.
pub const BUCKETS_PER_HALF: usize = 10;
/// Rules 3.2a: a bucket stands at ground level or is elevated to 1 m.
pub const BUCKET_STAND_HEIGHT: f32 = 1.0;
/// How many of each half's buckets stand on a 1 m stand. The rules allow
/// either height but do not fix the mix; sim choice.
pub const ELEVATED_PER_HALF: usize = 4;
/// Rules 3.1: 99 tennis balls in each penalty box at the start.
pub const BALLS_PER_PENALTY_BOX: usize = 99;
/// How far inside each goal line a bucket centre must stay. Sim choice.
pub const SPAWN_CLEARANCE: f32 = 1.6;
pub const DEFAULT_BUCKET_SEED: u64 = 42;

pub fn enabled() -> bool {
    #[cfg(all(feature = "gmu-viewer", not(target_arch = "wasm32")))]
    if crate::gmu::requested() {
        return true;
    }
    // The NEXT competition is the default world. An explicit TERRA_NEXT
    // decides; otherwise asking for the elevation map (TERRA_TILES=1) opts out.
    let flag = |name| match std::env::var(name).as_deref() {
        Ok("1") | Ok("true") | Ok("TRUE") => Some(true),
        Ok("0") | Ok("false") | Ok("FALSE") => Some(false),
        _ => None,
    };
    flag("TERRA_NEXT").unwrap_or(flag("TERRA_TILES") != Some(true))
}

pub fn install(app: &mut App) {
    let menu = app
        .world()
        .get_resource::<crate::session::SimSession>()
        .is_some_and(|session| session.phase == crate::session::Phase::Menu);
    app.add_systems(Update, (name_zatara, setup_crab_traps))
        .add_systems(FixedUpdate, (settle_deposits, collect_crab_trap_balls));
    if enabled() && !menu {
        app.add_systems(
            Startup,
            (spawn_field, crate::mason_field::spawn_surroundings),
        );
    }
}

pub fn on_pitch(x: f32, z: f32, margin: f32) -> bool {
    x.abs() <= PITCH_HALF_WIDTH - margin && z <= PITCH_SOUTH - margin && z >= PITCH_NORTH + margin
}

#[derive(Clone, Copy, Debug)]
pub struct Wall {
    pub center: Vec3,
    pub size: Vec3,
}

/// One open-top bucket on the pitch, at ground level or on a 1 m stand.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BucketPlace {
    pub x: f32,
    pub z: f32,
    pub elevated: bool,
}

impl BucketPlace {
    pub fn open(x: f32, z: f32) -> Self {
        Self {
            x,
            z,
            elevated: false,
        }
    }

    /// Height of the bucket's base above the turf.
    pub fn base_y(self) -> f32 {
        if self.elevated { BUCKET_STAND_HEIGHT } else { 0.0 }
    }

    pub fn volume(self) -> Bucket {
        let inner = BUCKET_TOP_DIAMETER * 0.5 - BUCKET_WALL;
        let base = self.base_y();
        Bucket {
            center_x: self.x,
            center_z: self.z,
            inner_radius: inner,
            floor_y: base + BUCKET_WALL,
            rim_y: base + BUCKET_BODY_HEIGHT,
            rest: Vec3::new(self.x, base + BUCKET_WALL + TENNIS_BALL_RADIUS, self.z),
        }
    }
}

#[derive(Component, Clone, Copy, Debug)]
pub struct Bucket {
    pub center_x: f32,
    pub center_z: f32,
    pub inner_radius: f32,
    pub floor_y: f32,
    pub rim_y: f32,
    pub rest: Vec3,
}

impl Bucket {
    /// Open top: a ball is inside once it has dropped through the rim.
    pub fn contains(self, point: Vec3) -> bool {
        let dx = point.x - self.center_x;
        let dz = point.z - self.center_z;
        dx * dx + dz * dz <= self.inner_radius * self.inner_radius
            && point.y <= self.rim_y + 0.02
            && point.y >= self.floor_y - 0.05
    }
}

pub fn bucket_seed() -> u64 {
    std::env::var("TERRA_NEXT_SEED")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(DEFAULT_BUCKET_SEED)
}

/// Ten buckets on each half. Same seed, same places. Not a competition rule.
pub fn buckets(seed: u64) -> Vec<BucketPlace> {
    let mut rng = fastrand::Rng::with_seed(seed);
    let mut places = place_half(&mut rng, true);
    places.extend(place_half(&mut rng, false));
    places
}

fn place_half(rng: &mut fastrand::Rng, south: bool) -> Vec<BucketPlace> {
    let margin = BUCKET_TOP_DIAMETER * 0.5 + 0.2;
    let x_min = -PITCH_HALF_WIDTH + margin;
    let x_max = PITCH_HALF_WIDTH - margin;
    let (z_min, z_max) = if south {
        (0.8, PITCH_SOUTH - SPAWN_CLEARANCE - margin)
    } else {
        (PITCH_NORTH + SPAWN_CLEARANCE + margin, -0.8)
    };
    let min_sep = 1.5_f32;
    let balls: Vec<Vec2> = ball_positions()
        .into_iter()
        .map(|ball| Vec2::new(ball.x, ball.z))
        .collect();
    let mut points = Vec::with_capacity(BUCKETS_PER_HALF);
    for _ in 0..8_000 {
        if points.len() == BUCKETS_PER_HALF {
            break;
        }
        let point = Vec2::new(
            x_min + rng.f32() * (x_max - x_min),
            z_min + rng.f32() * (z_max - z_min),
        );
        let clear = points
            .iter()
            .all(|other: &Vec2| other.distance(point) >= min_sep)
            && balls.iter().all(|ball| ball.distance(point) >= 0.8);
        if clear {
            points.push(point);
        }
    }
    assert_eq!(
        points.len(),
        BUCKETS_PER_HALF,
        "bucket layout could not place {BUCKETS_PER_HALF} buckets"
    );
    points.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    // Elevate a seeded subset, so stands appear at every depth.
    let mut order: Vec<usize> = (0..points.len()).collect();
    rng.shuffle(&mut order);
    let raised = &order[..ELEVATED_PER_HALF.min(order.len())];
    points
        .into_iter()
        .enumerate()
        .map(|(index, point)| BucketPlace {
            elevated: raised.contains(&index),
            ..BucketPlace::open(point.x, point.y)
        })
        .collect()
}

/// Goal-line spawn. Even slots take the south goal, odd slots the north goal.
/// Forward is local −Z, so the south side uses identity and the north side yaws 180°.
pub fn rover_spawn(slot: usize, count: usize, chassis: Vec3) -> (Vec3, Quat) {
    let south = slot % 2 == 0;
    let index = slot / 2;
    let on_side = if south { count.div_ceil(2) } else { count / 2 };
    let spacing = chassis.x + 0.25;
    let xs = goal_line_xs(on_side.max(1), spacing, chassis.x);
    let x = xs[index.min(xs.len() - 1)];
    let inset = chassis.z * 0.5 + 0.05;
    let z = if south {
        PITCH_SOUTH - inset
    } else {
        PITCH_NORTH + inset
    };
    let yaw = if south {
        Quat::IDENTITY
    } else {
        Quat::from_rotation_y(std::f32::consts::PI)
    };
    (Vec3::new(x, chassis.y * 0.5 + 0.02, z), yaw)
}

fn goal_line_xs(count: usize, spacing: f32, chassis_x: f32) -> Vec<f32> {
    let inner = GOAL_WIDTH * 0.5 - GOAL_POST_WIDTH - chassis_x * 0.5 - 0.08;
    let mouth = ((inner * 2.0 / spacing).floor() as usize)
        .saturating_add(1)
        .max(1);
    let in_mouth = count.min(mouth);
    let span = (in_mouth - 1) as f32 * spacing;
    let mut xs: Vec<f32> = (0..in_mouth)
        .map(|index| -span * 0.5 + index as f32 * spacing)
        .collect();
    let mut wing = 0usize;
    while xs.len() < count {
        let side = if wing % 2 == 0 { -1.0 } else { 1.0 };
        let step = (wing / 2) as f32;
        xs.push(
            side * (GOAL_WIDTH * 0.5 + GOAL_POST_WIDTH + chassis_x * 0.5 + 0.15 + step * spacing),
        );
        wing += 1;
    }
    xs
}

/// `TERRA_NEXT_BALLS` adds this many tennis balls scattered over the pitch,
/// as on a NEXT2026 practice field, on top of the six competition balls.
/// More balls give object detection more examples per frame.
pub fn extra_ball_count() -> usize {
    std::env::var("TERRA_NEXT_BALLS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(0)
        .min(400)
}

/// Deterministic scatter: on the pitch, clear of buckets and the goal mouths.
pub fn scattered_balls(count: usize, buckets: &[BucketPlace]) -> Vec<Vec3> {
    let mut rng = fastrand::Rng::with_seed(0x6e65_7874);
    let mut balls = Vec::with_capacity(count);
    for _ in 0..count * 50 {
        if balls.len() == count {
            break;
        }
        let x = (rng.f32() * 2.0 - 1.0) * (PITCH_HALF_WIDTH - 0.5);
        let z = PITCH_NORTH + 0.5 + rng.f32() * (PITCH_SOUTH - PITCH_NORTH - 1.0);
        let clear = buckets
            .iter()
            .all(|bucket| Vec2::new(bucket.x - x, bucket.z - z).length() > 0.6)
            && (x.abs() > GOAL_WIDTH || z.abs() < PITCH_SOUTH - 2.0);
        if clear {
            balls.push(Vec3::new(x, TENNIS_BALL_RADIUS, z));
        }
    }
    balls
}

/// Rules 3.1: 99 balls in each penalty box. They are spread evenly with a
/// seeded jitter and kept out of the goal area, where rovers start.
pub fn ball_positions() -> Vec<Vec3> {
    let mut rng = fastrand::Rng::with_seed(0x6e65_7874_3331);
    let mut balls = Vec::with_capacity(BALLS_PER_PENALTY_BOX * 2);
    for goal_z in [PITCH_SOUTH, PITCH_NORTH] {
        let inward = -goal_z.signum();
        let mut side = Vec::with_capacity(BALLS_PER_PENALTY_BOX);
        for _ in 0..BALLS_PER_PENALTY_BOX * 400 {
            if side.len() == BALLS_PER_PENALTY_BOX {
                break;
            }
            let x = (rng.f32() * 2.0 - 1.0) * (PENALTY_AREA_WIDTH * 0.5 - 0.3);
            let depth = 0.3 + rng.f32() * (PENALTY_AREA_DEPTH - 0.6);
            let in_goal_area = x.abs() < GOAL_AREA_WIDTH * 0.5 + 0.5 && depth < GOAL_AREA_DEPTH + 0.5;
            let point = Vec3::new(x, TENNIS_BALL_RADIUS, goal_z + inward * depth);
            if !in_goal_area && side.iter().all(|other: &Vec3| other.distance(point) > 0.9) {
                side.push(point);
            }
        }
        assert_eq!(side.len(), BALLS_PER_PENALTY_BOX, "penalty box ball layout");
        balls.extend(side);
    }
    balls
}

pub fn pitch_boards() -> Vec<Wall> {
    let y = BOARD_HEIGHT * 0.5;
    let length = PITCH_SOUTH - PITCH_NORTH;
    let width = PITCH_HALF_WIDTH * 2.0;
    let mid_z = (PITCH_SOUTH + PITCH_NORTH) * 0.5;
    let t = BOARD_THICKNESS;
    let opening = GOAL_WIDTH + GOAL_POST_WIDTH;
    let side = (width - opening) * 0.5;
    let mut walls = vec![
        Wall {
            center: Vec3::new(-PITCH_HALF_WIDTH - t * 0.5, y, mid_z),
            size: Vec3::new(t, BOARD_HEIGHT, length),
        },
        Wall {
            center: Vec3::new(PITCH_HALF_WIDTH + t * 0.5, y, mid_z),
            size: Vec3::new(t, BOARD_HEIGHT, length),
        },
    ];
    for goal_z in [PITCH_SOUTH, PITCH_NORTH] {
        let z = if goal_z > 0.0 {
            goal_z + t * 0.5
        } else {
            goal_z - t * 0.5
        };
        for side_sign in [-1.0_f32, 1.0] {
            walls.push(Wall {
                center: Vec3::new(side_sign * (opening * 0.5 + side * 0.5), y, z),
                size: Vec3::new(side, BOARD_HEIGHT, t),
            });
        }
    }
    walls
}

#[derive(Component)]
pub struct NextField;
/// Authored turf and markings; kept in robot views when spectators see GMU imagery.
#[derive(Component)]
pub struct PitchSurface;
#[derive(Component)]
pub struct TennisBall;
/// Number of NEXT balls held behind a CrabTrapBot's one-way entrance.
#[derive(Component, Default)]
pub struct CarriedBalls(pub usize);
#[derive(Component)]
struct TrapInstalled;
#[derive(Component)]
struct TrapFlap {
    side: f32,
}
#[derive(Component)]
struct CarriedBall;
#[derive(Component)]
pub struct PitchBoard;
/// Primary rover in the NEXT scene.
#[derive(Component)]
pub struct Zatara;

pub(crate) fn spawn_field(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    assets: Option<Res<AssetServer>>,
    config: Option<Res<crate::world::WorldConfig>>,
) {
    if config.is_some_and(|config| config.scene != crate::world::SceneKind::Next) {
        return;
    }
    spawn_field_contents(&mut commands, &mut meshes, &mut materials, assets.as_deref());
}

/// Optic yellow felt and Home Depot orange (#F96302), used when the
/// textures and the Blender bucket are not available (headless tests).
const BALL_FELT: Color = Color::srgb(0.875, 1.0, 0.31);
const BUCKET_ORANGE: Color = Color::srgb(0.976, 0.388, 0.008);
/// Blue buckets (#1D5DB2), matched by eye to the NEXT2026 footage.
const BUCKET_BLUE: Color = Color::srgb(0.114, 0.365, 0.698);
/// Turf lines in yellow, as on the multi-sport NEXT2026 field.
const LINE_YELLOW: Color = Color::srgb(0.95, 0.78, 0.12);
/// Mowing stripes across the pitch, as on a cut natural-grass field.
const MOWING_STRIPES: usize = 14;

/// Loads a texture that tiles: the pitch and apron repeat it every few metres.
pub(crate) fn repeating(assets: &AssetServer, path: &'static str) -> Handle<Image> {
    use bevy::image::{ImageAddressMode, ImageLoaderSettings, ImageSampler, ImageSamplerDescriptor};
    assets
        .load_builder()
        .with_settings(|settings: &mut ImageLoaderSettings| {
            settings.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
                address_mode_u: ImageAddressMode::Repeat,
                address_mode_v: ImageAddressMode::Repeat,
                ..ImageSamplerDescriptor::linear()
            });
        })
        .load(path)
}

fn turf_material(
    materials: &mut Assets<StandardMaterial>,
    turf: Option<&Handle<Image>>,
    width: f32,
    length: f32,
    shade: f32,
) -> Handle<StandardMaterial> {
    let repeat = crate::mason_field::TURF_REPEAT;
    materials.add(StandardMaterial {
        base_color: match turf {
            Some(_) => Color::srgb(shade, shade, shade),
            None => Color::srgb(0.18 * shade, 0.52 * shade, 0.22 * shade),
        },
        base_color_texture: turf.cloned(),
        uv_transform: bevy::math::Affine2::from_scale(Vec2::new(width / repeat, length / repeat)),
        perceptual_roughness: 0.95,
        ..default()
    })
}

fn spawn_field_contents(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    assets: Option<&AssetServer>,
) {
    let cube = meshes.add(Cuboid::new(1.0, 1.0, 1.0));
    let material = |materials: &mut Assets<StandardMaterial>, color, roughness| {
        materials.add(StandardMaterial {
            base_color: color,
            perceptual_roughness: roughness,
            ..default()
        })
    };
    let length = PITCH_SOUTH - PITCH_NORTH;
    let width = PITCH_HALF_WIDTH * 2.0;
    let turf = assets.map(|assets| repeating(assets, crate::mason_field::TURF_TEXTURE));
    let grass = turf_material(materials, turf.as_ref(), width, length, 1.0);
    let line = material(materials, Color::srgb(0.95, 0.95, 0.92), 0.7);
    let board = material(materials, Color::srgb(0.90, 0.92, 0.94), 0.55);
    // Grey: the rules reserve orange, blue and yellow for game pieces.
    let stand_color = material(materials, Color::srgb(0.54, 0.55, 0.57), 0.6);
    let bucket_colors = [
        material(materials, BUCKET_ORANGE, 0.42),
        material(materials, BUCKET_BLUE, 0.42),
    ];
    let goal_color = material(materials, Color::srgb(0.95, 0.95, 0.96), 0.4);
    // Felt is matte; the texture carries the white seam.
    let ball_color = materials.add(StandardMaterial {
        base_color: if assets.is_some() { Color::WHITE } else { BALL_FELT },
        base_color_texture: assets
            .map(|assets| assets.load(crate::mason_field::TENNIS_BALL_TEXTURE)),
        perceptual_roughness: 0.95,
        ..default()
    });
    let ball_mesh = meshes.add(Sphere::new(TENNIS_BALL_RADIUS).mesh().uv(32, 18));
    assert!(
        ball_positions()
            .iter()
            .all(|ball| on_pitch(ball.x, ball.z, 0.4)),
        "NEXT tennis balls must sit on the pitch"
    );

    let mid_z = (PITCH_SOUTH + PITCH_NORTH) * 0.5;
    commands.spawn((
        Name::new("NEXT pitch"),
        PitchSurface,
        NextField,
        Mesh3d(cube.clone()),
        MeshMaterial3d(grass),
        Transform::from_xyz(0.0, 0.003, mid_z).with_scale(Vec3::new(width, 0.004, length)),
    ));
    // Every other band is darker, laid just above the pitch and under the lines.
    let band = length / MOWING_STRIPES as f32;
    // Synthetic turf rolls in panels; the banding is faint, not mown stripes.
    let mown = turf_material(materials, turf.as_ref(), width, band, 0.93);
    for index in (0..MOWING_STRIPES).step_by(2) {
        commands.spawn((
            Name::new("NEXT mowing stripe"),
            PitchSurface,
            NextField,
            Mesh3d(cube.clone()),
            MeshMaterial3d(mown.clone()),
            Transform::from_xyz(0.0, 0.0052, PITCH_NORTH + band * (index as f32 + 0.5))
                .with_scale(Vec3::new(width, 0.0004, band)),
        ));
    }

    paint_lines(commands, &cube, &line);
    let yellow = material(materials, LINE_YELLOW, 0.7);
    // Yellow restraining lines across the field, 30 yards from each end line,
    // where the NEXT2026 field's lacrosse markings run.
    for z in [PITCH_SOUTH - 27.43, PITCH_NORTH + 27.43] {
        commands.spawn((
            Name::new("NEXT yellow line"),
            PitchSurface,
            NextField,
            Mesh3d(cube.clone()),
            MeshMaterial3d(yellow.clone()),
            Transform::from_xyz(0.0, MARKING_Y, z).with_scale(Vec3::new(
                PITCH_WIDTH,
                0.006,
                LINE_WIDTH,
            )),
        ));
    }
    for wall in pitch_boards() {
        commands.spawn((
            Name::new("NEXT board"),
            NextField,
            PitchBoard,
            RigidBody::Static,
            Collider::cuboid(1.0, 1.0, 1.0),
            Friction::new(0.6),
            Restitution::new(0.15),
            Mesh3d(cube.clone()),
            MeshMaterial3d(board.clone()),
            Transform::from_translation(wall.center).with_scale(wall.size),
        ));
    }
    spawn_goals(commands, meshes, &goal_color);
    let shell = meshes.add(bucket_shell_mesh());
    let places = buckets(bucket_seed());
    for (index, place) in places.iter().copied().enumerate() {
        let bucket = place.volume();
        if place.elevated {
            spawn_stand(commands, &cube, &stand_color, place);
        }
        let mut entity = commands.spawn((
            Name::new(format!("NEXT bucket {}", index + 1)),
            NextField,
            bucket,
            RigidBody::Static,
            bucket_collider(),
            Friction::new(0.7),
            Restitution::new(0.05),
            // Buckets sit at random angles, as they would when set down.
            Visibility::default(),
            Transform::from_xyz(place.x, place.base_y(), place.z)
                .with_rotation(Quat::from_rotation_y(index as f32 * 2.399_963)),
        ));
        // Rules 3.2b: each alliance's buckets stand in the opposing half. Red
        // (south Base) scores in the orange buckets on the north half; Blue
        // scores in the blue buckets on the south half.
        let blue = index < BUCKETS_PER_HALF;
        match assets {
            Some(assets) => {
                let model = if blue {
                    crate::mason_field::BLUE_BUCKET_MODEL
                } else {
                    crate::mason_field::BUCKET_MODEL
                };
                entity.with_child(WorldAssetRoot(
                    assets.load(GltfAssetLabel::Scene(0).from_asset(model)),
                ));
            }
            None => {
                entity.insert((
                    Mesh3d(shell.clone()),
                    MeshMaterial3d(bucket_colors[usize::from(blue)].clone()),
                ));
            }
        }
    }
    let balls = ball_positions()
        .into_iter()
        .chain(scattered_balls(extra_ball_count(), &places));
    for (index, position) in balls.enumerate() {
        commands.spawn((
            Name::new(format!("Tennis ball {}", index + 1)),
            NextField,
            TennisBall,
            RigidBody::Dynamic,
            Collider::sphere(TENNIS_BALL_RADIUS),
            Mass(TENNIS_BALL_MASS),
            Friction::new(0.65),
            Restitution::new(0.55),
            LinearDamping(0.45),
            AngularDamping(0.55),
            SweptCcd::default(),
            Mesh3d(ball_mesh.clone()),
            MeshMaterial3d(ball_color.clone()),
            Transform::from_translation(position),
        ));
    }
    eprintln!(
        "Terra NEXT: Zatara on a {PITCH_LENGTH} m by {PITCH_WIDTH} m pitch, {} tennis balls, {} deposit buckets. W/A/S/D drives the primary rover when Zenoh is off.",
        ball_positions().len() + extra_ball_count(),
        buckets(bucket_seed()).len()
    );
}

/// A 1 m stand for an elevated bucket (rules 3.2a): a 0.5 m square top on
/// four legs, static, so balls and rovers collide with it.
fn spawn_stand(
    commands: &mut Commands,
    cube: &Handle<Mesh>,
    material: &Handle<StandardMaterial>,
    place: BucketPlace,
) {
    const TOP: f32 = 0.5;
    const THICKNESS: f32 = 0.04;
    const LEG: f32 = 0.045;
    let leg_height = BUCKET_STAND_HEIGHT - THICKNESS;
    let inset = TOP * 0.5 - LEG;
    commands
        .spawn((
            Name::new("NEXT bucket stand"),
            NextField,
            RigidBody::Static,
            Collider::compound(
                std::iter::once((
                    Position::from_xyz(0.0, BUCKET_STAND_HEIGHT - THICKNESS * 0.5, 0.0),
                    Rotation::default(),
                    Collider::cuboid(TOP, THICKNESS, TOP),
                ))
                .chain([(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)].map(|(sx, sz)| {
                    (
                        Position::from_xyz(sx * inset, leg_height * 0.5, sz * inset),
                        Rotation::default(),
                        Collider::cuboid(LEG, leg_height, LEG),
                    )
                }))
                .collect(),
            ),
            Visibility::default(),
            Transform::from_xyz(place.x, 0.0, place.z),
        ))
        .with_children(|stand| {
            stand.spawn((
                Mesh3d(cube.clone()),
                MeshMaterial3d(material.clone()),
                Transform::from_xyz(0.0, BUCKET_STAND_HEIGHT - THICKNESS * 0.5, 0.0)
                    .with_scale(Vec3::new(TOP, THICKNESS, TOP)),
            ));
            for (sx, sz) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                stand.spawn((
                    Mesh3d(cube.clone()),
                    MeshMaterial3d(material.clone()),
                    Transform::from_xyz(sx * inset, leg_height * 0.5, sz * inset)
                        .with_scale(Vec3::new(LEG, leg_height, LEG)),
                ));
            }
        });
}

fn spawn_goals(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    material: &Handle<StandardMaterial>,
) {
    let post = meshes.add(Cylinder::new(GOAL_POST_WIDTH * 0.5, GOAL_HEIGHT));
    let bar = meshes.add(Cylinder::new(GOAL_POST_WIDTH * 0.5, GOAL_WIDTH));
    for goal_z in [PITCH_SOUTH, PITCH_NORTH] {
        for side in [-1.0_f32, 1.0] {
            commands.spawn((
                Name::new("NEXT goal post"),
                NextField,
                RigidBody::Static,
                Collider::cylinder(GOAL_POST_WIDTH * 0.5, GOAL_HEIGHT),
                Mesh3d(post.clone()),
                MeshMaterial3d(material.clone()),
                Transform::from_xyz(side * GOAL_WIDTH * 0.5, GOAL_HEIGHT * 0.5, goal_z),
            ));
        }
        commands.spawn((
            Name::new("NEXT crossbar"),
            NextField,
            RigidBody::Static,
            Collider::cylinder(GOAL_POST_WIDTH * 0.5, GOAL_WIDTH),
            Mesh3d(bar.clone()),
            MeshMaterial3d(material.clone()),
            Transform::from_xyz(0.0, GOAL_HEIGHT, goal_z)
                .with_rotation(Quat::from_rotation_z(std::f32::consts::FRAC_PI_2)),
        ));
    }
}

fn bucket_shell_mesh() -> Mesh {
    let bottom = BUCKET_BASE_DIAMETER * 0.5;
    let top = BUCKET_TOP_DIAMETER * 0.5;
    let height = BUCKET_BODY_HEIGHT;
    let segments = 28u32;
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut indices = Vec::new();
    let ring = |radius: f32, y: f32, index: u32| -> Vec3 {
        let angle = index as f32 / segments as f32 * std::f32::consts::TAU;
        Vec3::new(angle.cos() * radius, y, angle.sin() * radius)
    };
    let mut quad = |a: Vec3, b: Vec3, c: Vec3, d: Vec3| {
        let start = positions.len() as u32;
        let normal = (b - a).cross(c - a).normalize_or_zero();
        for point in [a, b, c, d] {
            positions.push(point.to_array());
            normals.push(normal.to_array());
        }
        indices.extend([start, start + 1, start + 2, start, start + 2, start + 3]);
    };
    for index in 0..segments {
        let next = index + 1;
        let outer_bottom = ring(bottom, 0.0, index);
        let outer_bottom_next = ring(bottom, 0.0, next);
        let outer_top = ring(top, height, index);
        let outer_top_next = ring(top, height, next);
        quad(outer_bottom, outer_bottom_next, outer_top_next, outer_top);
        let inner_bottom = ring((bottom - BUCKET_WALL).max(0.02), BUCKET_WALL, index);
        let inner_bottom_next = ring((bottom - BUCKET_WALL).max(0.02), BUCKET_WALL, next);
        let inner_top = ring((top - BUCKET_WALL).max(0.02), height, index);
        let inner_top_next = ring((top - BUCKET_WALL).max(0.02), height, next);
        quad(inner_top, inner_top_next, inner_bottom_next, inner_bottom);
        quad(
            inner_bottom,
            inner_bottom_next,
            outer_bottom_next,
            outer_bottom,
        );
    }
    let mut mesh = Mesh::new(
        bevy::render::render_resource::PrimitiveTopology::TriangleList,
        bevy::asset::RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_indices(bevy::mesh::Indices::U32(indices));
    mesh
}

fn bucket_collider() -> Collider {
    let bottom = BUCKET_BASE_DIAMETER * 0.5 - BUCKET_WALL * 0.5;
    let top = BUCKET_TOP_DIAMETER * 0.5 - BUCKET_WALL * 0.5;
    let wall_r = (bottom + top) * 0.5;
    let segments = 16;
    let arc = std::f32::consts::TAU * wall_r / segments as f32;
    let mut shapes = vec![(
        Position::from_xyz(0.0, BUCKET_WALL * 0.5, 0.0),
        Rotation::default(),
        Collider::cylinder((bottom - BUCKET_WALL).max(0.02), BUCKET_WALL),
    )];
    for index in 0..segments {
        let angle = index as f32 / segments as f32 * std::f32::consts::TAU;
        shapes.push((
            Position::from_xyz(
                angle.cos() * wall_r,
                BUCKET_BODY_HEIGHT * 0.5,
                angle.sin() * wall_r,
            ),
            Rotation::from(Quat::from_rotation_y(-angle)),
            Collider::cuboid(BUCKET_WALL * 2.0, BUCKET_BODY_HEIGHT, arc * 1.15),
        ));
    }
    Collider::compound(shapes)
}

fn paint_lines(commands: &mut Commands, cube: &Handle<Mesh>, line: &Handle<StandardMaterial>) {
    let length = PITCH_LENGTH;
    let width = PITCH_WIDTH;
    let mid_z = (PITCH_SOUTH + PITCH_NORTH) * 0.5;
    let y = MARKING_Y;
    let stripe = |commands: &mut Commands, center: Vec3, size: Vec3| {
        commands.spawn((
            Name::new("NEXT line"),
            PitchSurface,
            NextField,
            Mesh3d(cube.clone()),
            MeshMaterial3d(line.clone()),
            Transform::from_translation(center).with_scale(size),
        ));
    };
    let area = |commands: &mut Commands, goal_z: f32, depth: f32, half_width: f32| {
        // Positive Z is south. The box runs from the goal line into the field.
        let sign = if goal_z > 0.0 { -1.0 } else { 1.0 };
        stripe(
            commands,
            Vec3::new(0.0, y, goal_z + sign * depth),
            Vec3::new(half_width * 2.0, 0.006, LINE_WIDTH),
        );
        stripe(
            commands,
            Vec3::new(-half_width, y, goal_z + sign * depth * 0.5),
            Vec3::new(LINE_WIDTH, 0.006, depth),
        );
        stripe(
            commands,
            Vec3::new(half_width, y, goal_z + sign * depth * 0.5),
            Vec3::new(LINE_WIDTH, 0.006, depth),
        );
    };
    // Touchlines, goal lines, and the halfway line.
    stripe(
        commands,
        Vec3::new(0.0, y, PITCH_SOUTH),
        Vec3::new(width, 0.006, LINE_WIDTH),
    );
    stripe(
        commands,
        Vec3::new(0.0, y, PITCH_NORTH),
        Vec3::new(width, 0.006, LINE_WIDTH),
    );
    stripe(
        commands,
        Vec3::new(-PITCH_HALF_WIDTH, y, mid_z),
        Vec3::new(LINE_WIDTH, 0.006, length),
    );
    stripe(
        commands,
        Vec3::new(PITCH_HALF_WIDTH, y, mid_z),
        Vec3::new(LINE_WIDTH, 0.006, length),
    );
    stripe(
        commands,
        Vec3::new(0.0, y, mid_z),
        Vec3::new(width, 0.006, LINE_WIDTH),
    );
    for goal_z in [PITCH_SOUTH, PITCH_NORTH] {
        area(
            commands,
            goal_z,
            PENALTY_AREA_DEPTH,
            PENALTY_AREA_WIDTH * 0.5,
        );
        area(commands, goal_z, GOAL_AREA_DEPTH, GOAL_AREA_WIDTH * 0.5);
    }
    // Centre circle. Each dash is rotated along the tangent so the 9.15 m
    // radius reads as a line instead of a ring of axis-aligned blobs.
    let segments = 96;
    let arc = std::f32::consts::TAU * CENTER_CIRCLE_RADIUS / segments as f32;
    for index in 0..segments {
        let angle = index as f32 * std::f32::consts::TAU / segments as f32;
        commands.spawn((
            Name::new("NEXT line"),
            PitchSurface,
            NextField,
            Mesh3d(cube.clone()),
            MeshMaterial3d(line.clone()),
            Transform::from_translation(Vec3::new(
                angle.cos() * CENTER_CIRCLE_RADIUS,
                y,
                mid_z + angle.sin() * CENTER_CIRCLE_RADIUS,
            ))
            .with_rotation(Quat::from_rotation_y(-angle))
            .with_scale(Vec3::new(LINE_WIDTH, 0.006, arc * 1.08)),
        ));
    }
    stripe(
        commands,
        Vec3::new(0.0, y, mid_z),
        Vec3::new(0.22, 0.006, 0.22),
    );
}

fn name_zatara(
    mut commands: Commands,
    rovers: Query<(Entity, &crate::terra::RoverSlot), (With<crate::terra::Rover>, Without<Zatara>)>,
) {
    for (entity, slot) in &rovers {
        if slot.0 == 0 {
            commands
                .entity(entity)
                .insert((Name::new("Zatara"), Zatara));
        }
    }
}

/// A visible pair of vertical hinges at the intake. Their travel is limited
/// to the rover interior (+Z); the collection rule below holds balls there.
fn setup_crab_traps(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    rovers: Query<(Entity, &CarriedBalls), Without<TrapInstalled>>,
) {
    if rovers.is_empty() {
        return;
    }
    let flap_mesh = meshes.add(Cuboid::new(0.205, 0.12, 0.012));
    let flap_material = materials.add(Color::srgb(0.18, 0.22, 0.25));
    for (rover, _) in &rovers {
        commands
            .entity(rover)
            .insert(TrapInstalled)
            .with_children(|parent| {
                for side in [-1.0_f32, 1.0] {
                    parent
                        .spawn((
                            Name::new(if side < 0.0 {
                                "Left intake hinge"
                            } else {
                                "Right intake hinge"
                            }),
                            TrapFlap { side },
                            Visibility::default(),
                            Transform::from_xyz(side * 0.205, -0.237, -0.298),
                        ))
                        .with_children(|hinge| {
                            hinge.spawn((
                                Mesh3d(flap_mesh.clone()),
                                MeshMaterial3d(flap_material.clone()),
                                Transform::from_xyz(-side * 0.1025, 0.0, 0.0),
                            ));
                        });
                }
            });
    }
}

fn in_front_intake(local: Vec3) -> bool {
    local.x.abs() <= 0.17
        && local.y >= -0.31
        && local.y <= -0.19
        && local.z >= -0.33
        && local.z <= -0.19
}

fn collect_crab_trap_balls(
    mut commands: Commands,
    mut rovers: Query<(Entity, &Position, &Rotation, &mut CarriedBalls), With<crate::terra::Rover>>,
    balls: Query<
        (
            Entity,
            &Position,
            &Mesh3d,
            &MeshMaterial3d<StandardMaterial>,
        ),
        With<TennisBall>,
    >,
    mut flaps: Query<(&TrapFlap, &ChildOf, &mut Transform)>,
) {
    let mut claimed = std::collections::HashSet::new();
    for (rover, position, rotation, mut carried) in &mut rovers {
        let mut open = false;
        for (ball, ball_position, mesh, material) in &balls {
            let local = rotation.0.inverse() * (ball_position.0 - position.0);
            if local.x.abs() < 0.22
                && local.y >= -0.33
                && local.y < -0.17
                && local.z >= -0.43
                && local.z <= -0.18
            {
                open = true;
            }
            if !in_front_intake(local) || !claimed.insert(ball) {
                continue;
            }
            let slot = carried.0;
            carried.0 += 1;
            commands.entity(ball).despawn();
            commands.entity(rover).with_children(|parent| {
                parent.spawn((
                    Name::new(format!("Collected tennis ball {}", slot + 1)),
                    CarriedBall,
                    Mesh3d(mesh.0.clone()),
                    MeshMaterial3d(material.0.clone()),
                    Transform::from_xyz(
                        (slot % 4) as f32 * 0.075 - 0.1125,
                        -0.267 + (slot / 16) as f32 * 0.07,
                        -0.08 + ((slot / 4) % 4) as f32 * 0.075,
                    ),
                ));
            });
        }
        for (flap, parent, mut transform) in &mut flaps {
            if parent.parent() == rover {
                let target = if open { flap.side * 1.05 } else { 0.0 };
                let current = transform.rotation.to_euler(EulerRot::YXZ).0;
                let next = current + (target - current).clamp(-0.15, 0.15);
                transform.rotation = Quat::from_rotation_y(next);
            }
        }
    }
}

/// Draws a ball that has crossed a bucket mouth toward the back wall so it stays deposited.
fn settle_deposits(
    time: Res<Time<Fixed>>,
    mut balls: Query<(&Position, &mut LinearVelocity), With<TennisBall>>,
    bins: Query<&Bucket>,
) {
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }
    let damp = (-5.0 * dt).exp();
    for (position, mut velocity) in &mut balls {
        for bucket in &bins {
            if !bucket.contains(position.0) {
                continue;
            }
            let to_rest = Vec3::new(bucket.rest.x - position.x, 0.0, bucket.rest.z - position.z);
            let pull = to_rest.clamp_length_max(0.25) * 8.0;
            velocity.x += pull.x * dt;
            velocity.z += pull.z * dt;
            velocity.x *= damp;
            velocity.z *= damp;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        physics::{RoverBody, RoverPhysicsConfig, TerraPhysicsPlugin},
        terra::{
            DriveCommand, DriveConfig, Odometry, Rover, RoverFleet, TerraPlugin, WheelSpeeds, drive,
        },
        velocity_controller::TerraVelocityControlPlugin,
        world::{TerraWorldPlugin, WorldConfig},
    };
    use bevy::{asset::AssetPlugin, mesh::MeshPlugin, time::TimeUpdateStrategy};
    use std::time::Duration;
    use zorvane_vehicle::VehicleBody;

    #[test]
    fn trap_accepts_ground_ball_only_through_front_opening() {
        assert!(in_front_intake(Vec3::new(0.0, -0.287, -0.25)));
        assert!(!in_front_intake(Vec3::new(0.20, -0.287, -0.25)));
        assert!(!in_front_intake(Vec3::new(0.0, -0.287, 0.25)));
        assert!(!in_front_intake(Vec3::new(0.0, 0.0, -0.25)));
    }

    #[test]
    fn crab_trap_carries_front_ball_but_leaves_rear_ball_on_pitch() {
        let mut app = App::new();
        app.add_systems(FixedUpdate, collect_crab_trap_balls);
        let rover = app
            .world_mut()
            .spawn((
                Rover,
                Position(Vec3::new(0.0, 0.32, 0.0)),
                Rotation::default(),
                CarriedBalls::default(),
            ))
            .id();
        let spawn_ball = |app: &mut App, z| {
            app.world_mut()
                .spawn((
                    TennisBall,
                    Position(Vec3::new(0.0, TENNIS_BALL_RADIUS, z)),
                    Mesh3d(Handle::<Mesh>::default()),
                    MeshMaterial3d(Handle::<StandardMaterial>::default()),
                ))
                .id()
        };
        let front = spawn_ball(&mut app, -0.25);
        let rear = spawn_ball(&mut app, 0.25);
        app.world_mut().run_schedule(FixedUpdate);
        assert!(app.world().get_entity(front).is_err());
        assert!(app.world().get_entity(rear).is_ok());
        assert_eq!(app.world().get::<CarriedBalls>(rover).unwrap().0, 1);
        let mut carried = app.world_mut().query::<&CarriedBall>();
        assert_eq!(carried.iter(app.world()).count(), 1);
    }

    #[test]
    fn layout_fits_the_pitch_and_leaves_the_spawn_clear() {
        let spawn = Vec2::ZERO;
        let mut balls = ball_positions();
        assert_eq!(balls.len(), BALLS_PER_PENALTY_BOX * 2);
        for ball in &balls {
            assert!(on_pitch(ball.x, ball.z, 0.2), "{ball:?}");
            assert!((ball.y - TENNIS_BALL_RADIUS).abs() < 1e-4);
            assert!(Vec2::new(ball.x, ball.z).distance(spawn) > 1.5);
            // Rules 3.1: inside a penalty box, outside the goal area.
            let depth = PITCH_SOUTH - ball.z.abs();
            assert!(ball.x.abs() <= PENALTY_AREA_WIDTH * 0.5 && depth <= PENALTY_AREA_DEPTH, "{ball:?}");
            assert!(ball.x.abs() > GOAL_AREA_WIDTH * 0.5 || depth > GOAL_AREA_DEPTH, "{ball:?}");
        }
        assert_eq!(
            balls.iter().filter(|ball| ball.z > 0.0).count(),
            BALLS_PER_PENALTY_BOX
        );
        balls.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.z.total_cmp(&b.z)));
        for pair in balls.windows(2) {
            assert!(pair[0].distance(pair[1]) > 0.5, "{pair:?}");
        }
        assert!(WORLD_SIZE * 0.5 > PITCH_HALF_WIDTH + 1.0);
        assert!(WORLD_SIZE * 0.5 > PITCH_SOUTH.abs().max(PITCH_NORTH.abs()) + 1.0);
        assert_eq!(PITCH_LENGTH, 100.0);
        assert_eq!(PITCH_WIDTH, 64.0);
        assert_eq!(CENTER_CIRCLE_RADIUS, 9.15);
        assert_eq!(PENALTY_AREA_DEPTH, 16.5);
        assert_eq!(PENALTY_AREA_WIDTH, 40.32);
        assert_eq!(GOAL_AREA_DEPTH, 5.5);
        assert_eq!(GOAL_AREA_WIDTH, 18.32);
        assert!(GOAL_AREA_DEPTH < PENALTY_AREA_DEPTH);
        assert!(GOAL_AREA_WIDTH < PENALTY_AREA_WIDTH);
        assert!(PENALTY_AREA_WIDTH < PITCH_WIDTH);
        assert!(PENALTY_AREA_DEPTH * 2.0 < PITCH_LENGTH);
        assert_eq!(BUCKET_TOP_DIAMETER, 0.3048);
        assert_eq!(BUCKET_BASE_DIAMETER, 0.3023);
        assert_eq!(BUCKET_BODY_HEIGHT, 0.3399);
        assert!(BUCKET_TOP_DIAMETER > BUCKET_BASE_DIAMETER);
        let open = BucketPlace::open(0.0, 0.0);
        assert!((open.volume().rim_y - BUCKET_BODY_HEIGHT).abs() < 1e-6);
        let placed = buckets(DEFAULT_BUCKET_SEED);
        assert_eq!(placed.len(), BUCKETS_PER_HALF * 2);
        assert_eq!(buckets(DEFAULT_BUCKET_SEED), placed);
        assert_ne!(buckets(DEFAULT_BUCKET_SEED + 1), placed);
        let mut halves = [0usize, 0];
        for place in &placed {
            let bucket = place.volume();
            assert!(bucket.contains(bucket.rest), "{place:?}");
            assert!(on_pitch(place.x, place.z, BUCKET_TOP_DIAMETER * 0.5));
            assert!(
                place.z < PITCH_SOUTH - SPAWN_CLEARANCE && place.z > PITCH_NORTH + SPAWN_CLEARANCE,
                "bucket overlaps a goal-line spawn: {place:?}"
            );
            assert!(place.z.abs() > 0.5, "bucket sits on the halfway line");
            halves[usize::from(place.z > 0.0)] += 1;
            for ball in &ball_positions() {
                assert!(Vec2::new(place.x, place.z).distance(Vec2::new(ball.x, ball.z)) > 0.7);
            }
        }
        assert_eq!(halves, [BUCKETS_PER_HALF, BUCKETS_PER_HALF]);
        for south in [true, false] {
            let raised: Vec<_> = placed
                .iter()
                .filter(|place| (place.z > 0.0) == south && place.elevated)
                .collect();
            assert_eq!(raised.len(), ELEVATED_PER_HALF);
            for place in raised {
                let bucket = place.volume();
                assert!((bucket.floor_y - (BUCKET_STAND_HEIGHT + BUCKET_WALL)).abs() < 1e-6);
                assert!(bucket.contains(bucket.rest));
            }
        }
        for pair in placed.windows(2) {
            let apart = Vec2::new(pair[0].x, pair[0].z).distance(Vec2::new(pair[1].x, pair[1].z));
            if (pair[0].z > 0.0) == (pair[1].z > 0.0) {
                assert!(apart > 1.4, "{pair:?} {apart}");
            }
        }
        let chassis = Vec3::new(0.403, 0.144, 0.384);
        let (south, south_yaw) = rover_spawn(0, 1, chassis);
        assert!(south.x.abs() < 0.05);
        assert!((south.z - (PITCH_SOUTH - chassis.z * 0.5 - 0.05)).abs() < 1e-3);
        assert_eq!(south_yaw, Quat::IDENTITY);
        let (north, north_yaw) = rover_spawn(1, 2, chassis);
        assert!(north.x.abs() < 0.05);
        assert!(north.z < PITCH_NORTH + 0.4);
        assert!((north_yaw * Vec3::NEG_Z).z > 0.99);
        let (left, _) = rover_spawn(0, 4, chassis);
        let (right, _) = rover_spawn(2, 4, chassis);
        assert!((left.x - right.x).abs() > 0.4);
        assert!(left.z > 0.0 && right.z > 0.0);
        assert_eq!(PITCH_SOUTH - PITCH_NORTH, PITCH_LENGTH);
        assert_eq!(PITCH_HALF_WIDTH * 2.0, PITCH_WIDTH);
        for board in pitch_boards() {
            assert!(board.size.min_element() > 0.0);
            assert!(board.center.y > 0.0);
        }
    }

    #[test]
    fn next_world_replaces_the_practice_town() {
        let mut config = WorldConfig::default();
        config.prepare_next_field();
        assert!(!config.landscape.enabled);
        assert!(!config.tiles.enabled);
        assert_eq!(config.size, WORLD_SIZE);
        assert!(config.camera_offset.y < 8.0 && config.camera_offset.z < 10.0);
        assert!(config.follow_rover);
        assert!(config.validate().is_ok());
    }

    #[test]
    fn scene_spawns_balls_buckets_boards_and_zatara() {
        let mut app = App::new();
        app.insert_resource(Assets::<Mesh>::default())
            .insert_resource(Assets::<StandardMaterial>::default())
            .insert_resource(Time::<()>::default())
            .insert_resource(RoverPhysicsConfig::default())
            .insert_resource(ButtonInput::<KeyCode>::default())
            .add_plugins((
                TerraWorldPlugin {
                    config: {
                        let mut config = WorldConfig::default();
                        config.prepare_next_field();
                        config
                    },
                },
                TerraPlugin::default(),
            ))
            .add_systems(Startup, spawn_field)
            .add_systems(Update, name_zatara);
        app.update();
        let world = app.world_mut();
        assert_eq!(
            world.query::<&TennisBall>().iter(world).count(),
            ball_positions().len()
        );
        assert_eq!(
            world.query::<&Bucket>().iter(world).count(),
            buckets(bucket_seed()).len()
        );
        assert_eq!(
            world.query::<&PitchBoard>().iter(world).count(),
            pitch_boards().len()
        );
        let chassis_now = world.resource::<RoverPhysicsConfig>().chassis_size;
        let zatara = *world
            .query_filtered::<&Transform, With<Zatara>>()
            .single(world)
            .expect("primary rover");
        let (expected, _) = rover_spawn(0, 1, chassis_now);
        assert!((zatara.translation.x - expected.x).abs() < 0.05);
        assert!((zatara.translation.z - expected.z).abs() < 0.05);
        assert!((zatara.rotation * Vec3::NEG_Z).z < -0.99);
        let ground = zorvane_vehicle::TerraGround;
        let chassis = ground.chassis();
        let physics = world.resource::<RoverPhysicsConfig>();
        assert_eq!(
            physics.chassis_size,
            Vec3::new(
                chassis.size_xyz[0],
                chassis.size_xyz[1],
                chassis.size_xyz[2]
            ),
            "Zatara uses the terra-ground chassis"
        );
        assert_eq!(physics.mass_kg, chassis.mass_kg);
        let drive = world.resource::<DriveConfig>();
        let locomotion = ground.locomotion();
        let spec = locomotion
            .differential()
            .expect("terra-ground differential drive");
        assert_eq!(drive.wheel_radius, spec.wheel_radius_m);
        assert_eq!(drive.track_width, spec.track_width_m);
        assert_eq!(drive.model_path, ground.visual().asset_path);
        assert_eq!(world.resource::<RoverFleet>().count(), 1);
        let elements = world.query::<&NextField>().iter(world).count();
        app.update();
        let world = app.world_mut();
        assert_eq!(world.query::<&NextField>().iter(world).count(), elements);
        assert_eq!(world.query::<&Zatara>().iter(world).count(), 1);
    }

    fn physics_app() -> App {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            TransformPlugin,
            AssetPlugin::default(),
            MeshPlugin,
            TerraPhysicsPlugin,
        ))
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f64(
            1.0 / 60.0,
        )))
        .insert_resource(DriveConfig::default())
        .add_systems(
            FixedUpdate,
            (settle_deposits, drive)
                .chain()
                .before(PhysicsSystems::StepSimulation),
        );
        app.finish();
        app.cleanup();
        app.world_mut().spawn((
            RigidBody::Static,
            Collider::cuboid(40.0, 0.2, 40.0),
            Transform::from_xyz(0.0, -0.1, 0.0),
        ));
        app
    }

    fn tick(app: &mut App, frames: usize) {
        for _ in 0..frames {
            app.update();
        }
    }

    #[test]
    fn zatara_pushes_a_tennis_ball() {
        let mut app = physics_app();
        let ball = app
            .world_mut()
            .spawn((
                TennisBall,
                RigidBody::Dynamic,
                Collider::sphere(TENNIS_BALL_RADIUS),
                Mass(TENNIS_BALL_MASS),
                Friction::new(0.65),
                Restitution::new(0.2),
                LinearDamping(0.2),
                SweptCcd::default(),
                Transform::from_xyz(0.0, TENNIS_BALL_RADIUS + 0.02, -1.05),
            ))
            .id();
        let rover = app
            .world_mut()
            .spawn((
                Rover,
                RoverBody::default(),
                DriveCommand {
                    linear: 1.2,
                    angular: 0.0,
                },
                WheelSpeeds::default(),
                Odometry::default(),
                Transform::from_xyz(0.0, 0.2, 0.0),
            ))
            .id();
        tick(&mut app, 180);
        let ball_z = app.world().get::<Position>(ball).unwrap().z;
        let rover_z = app.world().get::<Position>(rover).unwrap().z;
        assert!(
            ball_z < -1.4,
            "ball was not pushed north: ball {ball_z}, rover {rover_z}"
        );
        assert!(
            rover_z < -0.2,
            "rover did not drive toward the ball: {rover_z}"
        );
    }

    #[test]
    fn force_drive_pushes_a_tennis_ball() {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            TransformPlugin,
            AssetPlugin::default(),
            MeshPlugin,
            TerraPhysicsPlugin,
            TerraVelocityControlPlugin,
        ))
        .insert_resource(DriveConfig::default())
        .insert_resource(Time::<Fixed>::from_hz(100.0))
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
            10,
        )));
        app.finish();
        app.cleanup();
        app.world_mut().spawn((
            RigidBody::Static,
            Collider::cuboid(40.0, 0.2, 40.0),
            Transform::from_xyz(0.0, -0.1, 0.0),
        ));
        let ball = app
            .world_mut()
            .spawn((
                TennisBall,
                RigidBody::Dynamic,
                Collider::sphere(TENNIS_BALL_RADIUS),
                Mass(TENNIS_BALL_MASS),
                Friction::new(0.65),
                Restitution::new(0.2),
                LinearDamping(0.2),
                SweptCcd::default(),
                Transform::from_xyz(0.0, TENNIS_BALL_RADIUS + 0.02, -1.4),
            ))
            .id();
        app.world_mut().spawn((
            Rover,
            RoverBody::default(),
            DriveCommand {
                linear: 1.0,
                angular: 0.0,
            },
            WheelSpeeds::default(),
            Odometry::default(),
            Transform::from_xyz(0.0, 0.1, 0.0),
        ));
        tick(&mut app, 500);
        let ball_z = app.world().get::<Position>(ball).unwrap().z;
        assert!(
            ball_z < -1.8,
            "force-driven rover did not push the ball: {ball_z}"
        );
    }

    #[test]
    fn bucket_keeps_a_deposited_ball() {
        let place = BucketPlace::open(0.0, 0.0);
        let bucket = place.volume();
        let mut app = physics_app();
        app.world_mut().spawn((
            bucket,
            RigidBody::Static,
            bucket_collider(),
            Friction::new(0.7),
            Restitution::new(0.05),
        ));
        let start = Vec3::new(bucket.inner_radius * 0.45, bucket.rest.y + 0.05, 0.0);
        assert!(bucket.contains(start), "{start:?} vs {bucket:?}");
        let ball = app
            .world_mut()
            .spawn((
                TennisBall,
                RigidBody::Dynamic,
                Collider::sphere(TENNIS_BALL_RADIUS),
                Mass(TENNIS_BALL_MASS),
                Friction::new(0.65),
                Restitution::new(0.2),
                LinearDamping(0.2),
                SweptCcd::default(),
                LinearVelocity(Vec3::new(0.4, 0.0, 0.0)),
                Transform::from_translation(start),
            ))
            .id();
        tick(&mut app, 180);
        let position = app.world().get::<Position>(ball).unwrap().0;
        assert!(
            bucket.contains(position),
            "ball left the bucket: {position:?}"
        );
        let started = Vec2::new(start.x, start.z).length();
        let ended = Vec2::new(position.x, position.z).length();
        assert!(
            ended < started,
            "ball was not drawn toward the middle: start {start:?} now {position:?}"
        );
    }
}
