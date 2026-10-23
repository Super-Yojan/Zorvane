//! NEXT competition field for Zatara.
//!
//! A full soccer pitch at the FIFA Laws of the Game recommended international
//! size, with tennis balls and open-top deposit buckets. The primary rover
//! is named Zatara. It is spawn slot 0 of the active body, which defaults to
//! `terra-ground` (Terra's chassis and differential drive). Zatara is not a
//! separate vehicle type. Rovers line up on the goal lines, facing into the field.

use avian3d::prelude::*;
use bevy::prelude::*;

/// FIFA Laws of the Game, Law 1, recommended size for international matches.
/// User decision: use this size for the NEXT pitch.
pub const PITCH_LENGTH: f32 = 105.0;
/// FIFA Laws of the Game, Law 1, recommended width for international matches.
pub const PITCH_WIDTH: f32 = 68.0;
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
/// Square ground that contains the pitch, boards, and an apron.
pub const WORLD_SIZE: f32 = 120.0;
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
pub const BALLS_PER_GOAL: usize = 36;
pub const ROVER_BALL_CAPACITY: usize = 8;
/// Maximum balls staged in the top-mounted dumper before a dump press.
pub const DUMPER_CAPACITY: usize = 2;
const DUMPER_BALL_LOCAL_Y: f32 = 0.32;

/// 5-gallon bucket, from the user-supplied product diagram (inches × 0.0254).
/// Open top. The diagram’s lid is not part of the model.
pub const BUCKET_TOP_DIAMETER: f32 = 0.3048;
pub const BUCKET_BASE_DIAMETER: f32 = 0.3023;
/// Body height. There is no lid.
pub const BUCKET_BODY_HEIGHT: f32 = 0.3399;
/// Shell thickness so the open top has an interior. Not on the product diagram.
const BUCKET_WALL: f32 = 0.0025;
/// Wider collision shell than the visible bucket wall, for reliable rover contact.
const BUCKET_COLLIDER_WALL: f32 = 0.025;
pub const BUCKETS_PER_HALF: usize = 10;
/// How far inside each goal line a bucket centre must stay. Sim choice.
pub const SPAWN_CLEARANCE: f32 = 1.6;
/// Scoring bucket just ahead of each goal-line rover spawn.
pub const GOAL_BUCKET_INSET: f32 = 0.9;
pub const DEFAULT_BUCKET_SEED: u64 = 42;

pub fn enabled() -> bool {
    #[cfg(all(feature = "gmu-viewer", not(target_arch = "wasm32")))]
    if crate::gmu::requested() {
        return true;
    }
    matches!(
        std::env::var("TERRA_NEXT").as_deref(),
        Ok("1") | Ok("true") | Ok("TRUE")
    )
}

pub fn install(app: &mut App) {
    let menu = app
        .world()
        .get_resource::<crate::session::SimSession>()
        .is_some_and(|session| session.phase == crate::session::Phase::Menu);
    app.init_resource::<NextTeamScore>()
        .add_systems(Update, (name_zatara, setup_crab_traps))
        .add_systems(
            Update,
            (
                load_hud_clicks,
                dump_hud_clicks,
                load_world_clicks,
                sync_waiting_markers,
                update_score_hud,
            )
                .chain(),
        )
        .add_systems(
            FixedUpdate,
            (
                settle_deposits,
                collect_crab_trap_balls,
                queue_balls_at_goal,
                sync_full_crab_traps,
            )
                .chain()
                .before(PhysicsSystems::StepSimulation),
        );
    if enabled() && !menu {
        app.add_systems(Startup, spawn_field);
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

/// One open-top bucket on the pitch.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BucketPlace {
    pub x: f32,
    pub z: f32,
}

impl BucketPlace {
    pub fn open(x: f32, z: f32) -> Self {
        Self { x, z }
    }

    pub fn volume(self) -> Bucket {
        let inner = BUCKET_TOP_DIAMETER * 0.5 - BUCKET_WALL;
        Bucket {
            center_x: self.x,
            center_z: self.z,
            inner_radius: inner,
            floor_y: BUCKET_WALL,
            rim_y: BUCKET_BODY_HEIGHT,
            rest: Vec3::new(self.x, BUCKET_WALL + TENNIS_BALL_RADIUS, self.z),
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

#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub enum BucketSide {
    Orange,
    Blue,
}

impl BucketSide {
    fn at(z: f32) -> Self {
        if z >= 0.0 { Self::Orange } else { Self::Blue }
    }
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

fn goal_bucket_place(south: bool) -> BucketPlace {
    BucketPlace::open(
        0.0,
        if south {
            PITCH_SOUTH - GOAL_BUCKET_INSET
        } else {
            PITCH_NORTH + GOAL_BUCKET_INSET
        },
    )
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
    let goal = goal_bucket_place(south);
    let mut points = vec![Vec2::new(goal.x, goal.z)];
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
    points
        .into_iter()
        .map(|point| BucketPlace::open(point.x, point.y))
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

/// Most balls cluster in each penalty box, with a few along its outer flanks.
pub fn ball_positions() -> [Vec3; BALLS_PER_GOAL * 2] {
    std::array::from_fn(|index| {
        let goal = index / BALLS_PER_GOAL;
        let local = index % BALLS_PER_GOAL;
        let (x, distance_from_center) = if local < 30 {
            let row = local / 10;
            let column = local % 10;
            (
                -18.0 + column as f32 * 4.0 + if row == 1 { 1.0 } else { 0.0 },
                40.0 + row as f32 * 4.0,
            )
        } else {
            let flank = local - 30;
            let rank = flank % 3;
            let side = if flank < 3 { -1.0 } else { 1.0 };
            (side * (24.0 + rank as f32 * 3.0), 41.0 + rank as f32 * 3.0)
        };
        let z = distance_from_center * if goal == 0 { -1.0 } else { 1.0 };
        Vec3::new(x, TENNIS_BALL_RADIUS, z)
    })
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
/// Balls on one NEXT rover. Waiting balls still count as onboard until clicked.
#[derive(Component, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct RoverBallLoad {
    pub collected: usize,
    pub waiting: usize,
    pub dumper: usize,
}

impl RoverBallLoad {
    pub fn count(self) -> usize {
        self.collected + self.waiting + self.dumper
    }
    pub fn needs_operator(self) -> bool {
        self.waiting > 0 || self.dumper > 0
    }
    fn pickup(&mut self) -> bool {
        if self.needs_operator() || self.count() >= ROVER_BALL_CAPACITY {
            return false;
        }
        self.collected += 1;
        true
    }
    fn state_label(self) -> &'static str {
        if self.needs_operator() {
            "WAITING FOR OPERATOR"
        } else if self.count() >= ROVER_BALL_CAPACITY {
            "FULL: RETURN TO GOAL"
        } else if self.collected > 0 {
            "CARRYING BALLS"
        } else {
            "SEARCHING"
        }
    }
    fn queue_at_goal(&mut self) {
        if !self.needs_operator() {
            self.waiting = std::mem::take(&mut self.collected);
        }
    }
    fn load_dumper(&mut self) -> bool {
        if self.waiting == 0 || self.dumper >= DUMPER_CAPACITY {
            return false;
        }
        self.waiting -= 1;
        self.dumper += 1;
        true
    }
    fn dump(&mut self) -> usize {
        std::mem::take(&mut self.dumper)
    }
}

#[derive(Resource, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct NextTeamScore(pub usize);
/// Only the CrabTrapBot model has the simulated front intake.
#[derive(Component)]
pub struct CrabTrapIntake;
#[derive(Component)]
struct WaitingMarker(Entity);
#[derive(Component)]
struct NextHud;
#[derive(Component)]
struct LoadRover(Entity);
#[derive(Component)]
struct DumpRover(Entity);
#[derive(Component)]
struct TrapInstalled;
#[derive(Component)]
struct TrapFrontClosed;
#[derive(Component)]
struct TrapFlap {
    side: f32,
}
#[derive(Component)]
struct CarriedBall;
#[derive(Component)]
struct DumperBall;
/// Released by DUMP; scored once it settles inside a bucket.
#[derive(Component)]
struct DumpedBall;
#[derive(Component)]
pub struct PitchBoard;
/// Primary rover in the NEXT scene.
#[derive(Component)]
pub struct Zatara;

pub(crate) fn spawn_field(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    config: Option<Res<crate::world::WorldConfig>>,
    mut score: Option<ResMut<NextTeamScore>>,
) {
    if config.is_some_and(|config| config.scene != crate::world::SceneKind::Next) {
        return;
    }
    if let Some(score) = score.as_mut() {
        score.0 = 0;
    }
    spawn_field_contents(&mut commands, &mut meshes, &mut materials);
}

fn spawn_field_contents(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
) {
    let cube = meshes.add(Cuboid::new(1.0, 1.0, 1.0));
    let material = |materials: &mut Assets<StandardMaterial>, color, roughness| {
        materials.add(StandardMaterial {
            base_color: color,
            perceptual_roughness: roughness,
            ..default()
        })
    };
    let grass = material(materials, Color::srgb(0.18, 0.52, 0.22), 0.95);
    let line = material(materials, Color::srgb(0.95, 0.95, 0.92), 0.7);
    let board = material(materials, Color::srgb(0.90, 0.92, 0.94), 0.55);
    let orange_bucket = material(materials, Color::srgb(0.95, 0.38, 0.06), 0.45);
    let blue_bucket = material(materials, Color::srgb(0.05, 0.34, 0.88), 0.45);
    let goal_color = material(materials, Color::srgb(0.95, 0.95, 0.96), 0.4);
    let ball_color = material(materials, Color::srgb(0.79, 0.95, 0.10), 0.35);
    let ball_mesh = meshes.add(Sphere::new(TENNIS_BALL_RADIUS).mesh().ico(3).unwrap());
    assert!(
        ball_positions()
            .iter()
            .all(|ball| on_pitch(ball.x, ball.z, 0.4)),
        "NEXT tennis balls must sit on the pitch"
    );

    let length = PITCH_SOUTH - PITCH_NORTH;
    let width = PITCH_HALF_WIDTH * 2.0;
    let mid_z = (PITCH_SOUTH + PITCH_NORTH) * 0.5;
    commands.spawn((
        Name::new("NEXT pitch"),
        PitchSurface,
        NextField,
        Mesh3d(cube.clone()),
        MeshMaterial3d(grass),
        Transform::from_xyz(0.0, 0.003, mid_z).with_scale(Vec3::new(width, 0.004, length)),
    ));

    paint_lines(commands, &cube, &line);
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
    for (index, place) in buckets(bucket_seed()).into_iter().enumerate() {
        let bucket = place.volume();
        let side = BucketSide::at(place.z);
        commands.spawn((
            Name::new(format!("NEXT bucket {}", index + 1)),
            NextField,
            bucket,
            side,
            RigidBody::Static,
            bucket_collider(),
            Friction::new(0.7),
            Restitution::new(0.05),
            Mesh3d(shell.clone()),
            MeshMaterial3d(match side {
                BucketSide::Orange => orange_bucket.clone(),
                BucketSide::Blue => blue_bucket.clone(),
            }),
            Transform::from_xyz(place.x, 0.0, place.z),
        ));
    }
    for (index, position) in ball_positions().into_iter().enumerate() {
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
        ball_positions().len(),
        buckets(bucket_seed()).len()
    );
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
    let bottom = BUCKET_BASE_DIAMETER * 0.5 - BUCKET_COLLIDER_WALL * 0.5;
    let top = BUCKET_TOP_DIAMETER * 0.5 - BUCKET_COLLIDER_WALL * 0.5;
    let wall_r = (bottom + top) * 0.5;
    let segments = 16;
    let arc = std::f32::consts::TAU * wall_r / segments as f32;
    let mut shapes = vec![(
        Position::from_xyz(0.0, BUCKET_WALL * 0.5, 0.0),
        Rotation::default(),
        Collider::cylinder((bottom - BUCKET_COLLIDER_WALL).max(0.02), BUCKET_WALL),
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
            Collider::cuboid(BUCKET_COLLIDER_WALL, BUCKET_BODY_HEIGHT, arc * 1.15),
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
    rovers: Query<Entity, (With<CrabTrapIntake>, Without<TrapInstalled>)>,
) {
    if rovers.is_empty() {
        return;
    }
    let flap_mesh = meshes.add(Cuboid::new(0.205, 0.12, 0.012));
    let flap_material = materials.add(Color::srgb(0.18, 0.22, 0.25));
    for rover in &rovers {
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
    // Include the space behind both flap edges, where a ball can roll or bounce
    // after crossing the front opening. The rear wall starts beyond this region.
    local.x.abs() <= 0.245
        && local.y >= -0.35
        && local.y <= -0.10
        && local.z >= -0.37
        && local.z <= 0.18
}

fn collect_crab_trap_balls(
    mut commands: Commands,
    mut rovers: Query<(Entity, &Position, &Rotation, &mut RoverBallLoad), With<CrabTrapIntake>>,
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
        if carried.needs_operator() {
            for (_, parent, mut transform) in &mut flaps {
                if parent.parent() == rover {
                    transform.rotation = Quat::IDENTITY;
                }
            }
            continue;
        }
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
            if !in_front_intake(local) || claimed.contains(&ball) {
                continue;
            }
            let slot = carried.count();
            if !carried.pickup() {
                continue;
            }
            claimed.insert(ball);
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
                if carried.count() >= ROVER_BALL_CAPACITY {
                    transform.rotation = Quat::IDENTITY;
                    continue;
                }
                let target = if open { flap.side * 1.05 } else { 0.0 };
                let current = transform.rotation.to_euler(EulerRot::YXZ).0;
                let next = current + (target - current).clamp(-0.15, 0.15);
                transform.rotation = Quat::from_rotation_y(next);
            }
        }
    }
}

fn sync_full_crab_traps(
    mut commands: Commands,
    physics: Res<crate::physics::RoverPhysicsConfig>,
    mut rovers: Query<
        (
            Entity,
            &RoverBallLoad,
            &mut Collider,
            Option<&TrapFrontClosed>,
        ),
        With<CrabTrapIntake>,
    >,
) {
    for (rover, load, mut collider, closed) in &mut rovers {
        let should_close = load.count() >= ROVER_BALL_CAPACITY || load.needs_operator();
        if should_close == closed.is_some() {
            continue;
        }
        *collider =
            crate::physics::RoverBody::crab_trap_collider(physics.chassis_size, should_close);
        if should_close {
            commands.entity(rover).insert(TrapFrontClosed);
        } else {
            commands.entity(rover).remove::<TrapFrontClosed>();
        }
    }
}

fn at_home_goal(slot: usize, position: Vec3, chassis: Vec3) -> bool {
    let goal_z = if slot % 2 == 0 {
        PITCH_SOUTH
    } else {
        PITCH_NORTH
    };
    position.x.abs() <= GOAL_WIDTH * 0.5 - chassis.x * 0.5 && (position.z - goal_z).abs() <= 1.0
}

fn queue_balls_at_goal(
    physics: Res<crate::physics::RoverPhysicsConfig>,
    mut rovers: Query<
        (
            &crate::terra::RoverSlot,
            &Position,
            &mut RoverBallLoad,
            &mut crate::terra::DriveCommand,
            &mut LinearVelocity,
            &mut AngularVelocity,
        ),
        With<crate::terra::Rover>,
    >,
) {
    for (slot, position, mut load, mut drive, mut linear, mut angular) in &mut rovers {
        if load.collected == 0
            || load.needs_operator()
            || !at_home_goal(slot.0, position.0, physics.chassis_size)
        {
            continue;
        }
        load.queue_at_goal();
        *drive = crate::terra::DriveCommand::default();
        linear.x = 0.0;
        linear.z = 0.0;
        angular.0 = Vec3::ZERO;
    }
}

fn operator_active(
    config: Option<&crate::world::WorldConfig>,
    session: Option<&crate::session::SimSession>,
) -> bool {
    config.is_some_and(|config| config.scene == crate::world::SceneKind::Next)
        && session.is_none_or(|session| session.phase == crate::session::Phase::Playing)
}

fn load_rover_once(
    rover: Entity,
    load: &mut RoverBallLoad,
    carried_visuals: &Query<
        (Entity, &ChildOf, &Mesh3d, &MeshMaterial3d<StandardMaterial>),
        With<CarriedBall>,
    >,
    commands: &mut Commands,
) {
    if !load.load_dumper() {
        return;
    }
    if let Some((ball, _, mesh, material)) = carried_visuals
        .iter()
        .find(|(_, parent, _, _)| parent.parent() == rover)
    {
        commands.entity(ball).despawn();
        let slot = load.dumper - 1;
        commands.entity(rover).with_children(|parent| {
            parent.spawn((
                Name::new("Ball in dumper"),
                DumperBall,
                Mesh3d(mesh.0.clone()),
                MeshMaterial3d(material.0.clone()),
                Transform::from_xyz(0.0, DUMPER_BALL_LOCAL_Y + slot as f32 * 0.075, 0.0),
            ));
        });
    }
}

fn load_hud_clicks(
    mouse: Res<ButtonInput<MouseButton>>,
    config: Option<Res<crate::world::WorldConfig>>,
    session: Option<Res<crate::session::SimSession>>,
    physics: Res<crate::physics::RoverPhysicsConfig>,
    buttons: Query<(&Interaction, &LoadRover), Changed<Interaction>>,
    mut rovers: Query<(&crate::terra::RoverSlot, &Position, &mut RoverBallLoad)>,
    carried_visuals: Query<
        (Entity, &ChildOf, &Mesh3d, &MeshMaterial3d<StandardMaterial>),
        With<CarriedBall>,
    >,
    mut commands: Commands,
) {
    if !mouse.just_pressed(MouseButton::Left)
        || !operator_active(config.as_deref(), session.as_deref())
    {
        return;
    }
    for (interaction, button) in &buttons {
        if *interaction != Interaction::Pressed {
            continue;
        }
        if let Ok((slot, position, mut load)) = rovers.get_mut(button.0) {
            if at_home_goal(slot.0, position.0, physics.chassis_size) {
                load_rover_once(button.0, &mut load, &carried_visuals, &mut commands);
            }
        }
    }
}

fn dump_rover_once(
    rover: Entity,
    slot: usize,
    load: &mut RoverBallLoad,
    dumper_visuals: &Query<(Entity, &ChildOf), With<DumperBall>>,
    mesh: &Handle<Mesh>,
    material: &Handle<StandardMaterial>,
    commands: &mut Commands,
) {
    let count = load.dump();
    if count == 0 {
        return;
    }
    for (ball, parent) in dumper_visuals {
        if parent.parent() == rover {
            commands.entity(ball).despawn();
        }
    }
    let target = goal_bucket_place(slot % 2 == 0);
    for index in 0..count {
        commands.spawn((
            Name::new("Dumped tennis ball"),
            NextField,
            TennisBall,
            DumpedBall,
            RigidBody::Dynamic,
            Collider::sphere(TENNIS_BALL_RADIUS),
            Mass(TENNIS_BALL_MASS),
            Friction::new(0.65),
            Restitution::new(0.05),
            LinearDamping(0.8),
            AngularDamping(0.8),
            SweptCcd::default(),
            Mesh3d(mesh.clone()),
            MeshMaterial3d(material.clone()),
            Transform::from_xyz(
                target.x + (index as f32 - (count - 1) as f32 * 0.5) * 0.05,
                BUCKET_BODY_HEIGHT + 0.14 + index as f32 * 0.08,
                target.z,
            ),
        ));
    }
}

fn dump_hud_clicks(
    mouse: Res<ButtonInput<MouseButton>>,
    config: Option<Res<crate::world::WorldConfig>>,
    session: Option<Res<crate::session::SimSession>>,
    physics: Res<crate::physics::RoverPhysicsConfig>,
    buttons: Query<(&Interaction, &DumpRover), Changed<Interaction>>,
    mut rovers: Query<(&crate::terra::RoverSlot, &Position, &mut RoverBallLoad)>,
    dumper_visuals: Query<(Entity, &ChildOf), With<DumperBall>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut ball_assets: Local<Option<(Handle<Mesh>, Handle<StandardMaterial>)>>,
    mut commands: Commands,
) {
    if !mouse.just_pressed(MouseButton::Left)
        || !operator_active(config.as_deref(), session.as_deref())
    {
        return;
    }
    for (interaction, button) in &buttons {
        if *interaction != Interaction::Pressed {
            continue;
        }
        if let Ok((slot, position, mut load)) = rovers.get_mut(button.0) {
            if !at_home_goal(slot.0, position.0, physics.chassis_size) || load.dumper == 0 {
                continue;
            }
            let (mesh, material) = ball_assets.get_or_insert_with(|| {
                (
                    meshes.add(Sphere::new(TENNIS_BALL_RADIUS).mesh().ico(3).unwrap()),
                    materials.add(Color::srgb(0.79, 0.95, 0.10)),
                )
            });
            dump_rover_once(
                button.0,
                slot.0,
                &mut load,
                &dumper_visuals,
                mesh,
                material,
                &mut commands,
            );
        }
    }
}

fn ray_hits_rover(ray: Ray3d, center: Vec3, rotation: Quat, size: Vec3) -> Option<f32> {
    let inverse = rotation.inverse();
    let origin = inverse * (ray.origin - center);
    let direction = inverse * *ray.direction;
    let half = size * 0.5 + Vec3::splat(0.08);
    let mut near = 0.0_f32;
    let mut far = f32::INFINITY;
    for axis in 0..3 {
        let o = origin[axis];
        let d = direction[axis];
        let h = half[axis];
        if d.abs() < 1e-6 {
            if o.abs() > h {
                return None;
            }
        } else {
            let a = (-h - o) / d;
            let b = (h - o) / d;
            near = near.max(a.min(b));
            far = far.min(a.max(b));
            if far < near {
                return None;
            }
        }
    }
    Some(near)
}

fn load_world_clicks(
    mouse: Res<ButtonInput<MouseButton>>,
    config: Option<Res<crate::world::WorldConfig>>,
    session: Option<Res<crate::session::SimSession>>,
    physics: Res<crate::physics::RoverPhysicsConfig>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    cameras: Query<(&Camera, &GlobalTransform), With<crate::world::WorldCamera>>,
    ui: Query<&Interaction, With<Button>>,
    mut rovers: Query<
        (
            Entity,
            &crate::terra::RoverSlot,
            &Position,
            &Rotation,
            &mut RoverBallLoad,
        ),
        With<crate::terra::Rover>,
    >,
    carried_visuals: Query<
        (Entity, &ChildOf, &Mesh3d, &MeshMaterial3d<StandardMaterial>),
        With<CarriedBall>,
    >,
    mut commands: Commands,
) {
    if !mouse.just_pressed(MouseButton::Left)
        || !operator_active(config.as_deref(), session.as_deref())
        || ui
            .iter()
            .any(|interaction| *interaction == Interaction::Pressed)
    {
        return;
    }
    let (Ok(window), Ok((camera, camera_transform))) = (windows.single(), cameras.single()) else {
        return;
    };
    let Some(cursor) = window.cursor_position() else {
        return;
    };
    let Ok(ray) = camera.viewport_to_world(camera_transform, cursor) else {
        return;
    };
    let mut nearest = None;
    for (rover, _, position, rotation, _) in &mut rovers {
        if let Some(distance) = ray_hits_rover(ray, position.0, rotation.0, physics.chassis_size) {
            if nearest.is_none_or(|(_, best)| distance < best) {
                nearest = Some((rover, distance));
            }
        }
    }
    if let Some((rover, _)) = nearest {
        if let Ok((_, slot, position, _, mut load)) = rovers.get_mut(rover) {
            if at_home_goal(slot.0, position.0, physics.chassis_size) {
                load_rover_once(rover, &mut load, &carried_visuals, &mut commands);
            }
        }
    }
}

fn sync_waiting_markers(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut marker_assets: Local<Option<(Handle<Mesh>, Handle<StandardMaterial>)>>,
    rovers: Query<(Entity, &RoverBallLoad, Option<&WaitingMarker>), With<crate::terra::Rover>>,
) {
    for (rover, load, marker) in &rovers {
        match (load.needs_operator(), marker) {
            (true, None) => {
                let (mesh, material) = marker_assets.get_or_insert_with(|| {
                    (
                        meshes.add(Sphere::new(0.16).mesh().ico(2).unwrap()),
                        materials.add(StandardMaterial {
                            base_color: Color::srgb(1.0, 0.32, 0.02),
                            unlit: true,
                            ..default()
                        }),
                    )
                });
                let marker = commands
                    .spawn((
                        Name::new("Operator attention needed"),
                        Mesh3d(mesh.clone()),
                        MeshMaterial3d(material.clone()),
                        Transform::from_xyz(0.0, 0.55, 0.0),
                    ))
                    .id();
                commands
                    .entity(rover)
                    .add_child(marker)
                    .insert(WaitingMarker(marker));
            }
            (false, Some(marker)) => {
                commands.entity(marker.0).despawn();
                commands.entity(rover).remove::<WaitingMarker>();
            }
            _ => {}
        }
    }
}

#[derive(Default, PartialEq, Eq)]
struct HudSnapshot {
    score: usize,
    rows: Vec<(Entity, u64, usize, usize, usize)>,
}

fn update_score_hud(
    mut commands: Commands,
    config: Option<Res<crate::world::WorldConfig>>,
    session: Option<Res<crate::session::SimSession>>,
    score: Res<NextTeamScore>,
    rovers: Query<(Entity, &crate::terra::RoverId, &RoverBallLoad), With<crate::terra::Rover>>,
    roots: Query<Entity, With<NextHud>>,
    mut previous: Local<Option<HudSnapshot>>,
) {
    if !operator_active(config.as_deref(), session.as_deref()) {
        for root in &roots {
            commands.entity(root).despawn();
        }
        *previous = None;
        return;
    }
    let mut rows: Vec<_> = rovers
        .iter()
        .map(|(entity, id, load)| (entity, id.0, load.collected, load.waiting, load.dumper))
        .collect();
    rows.sort_by_key(|row| row.1);
    let snapshot = HudSnapshot {
        score: score.0,
        rows,
    };
    if previous.as_ref() == Some(&snapshot) && !roots.is_empty() {
        return;
    }
    for root in &roots {
        commands.entity(root).despawn();
    }
    let attention = snapshot.rows.iter().any(|row| row.3 > 0 || row.4 > 0);
    commands
        .spawn((
            Name::new("NEXT operator score"),
            NextHud,
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(16.0),
                right: Val::Px(16.0),
                width: Val::Px(250.0),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(4.0),
                padding: UiRect::all(Val::Px(10.0)),
                ..default()
            },
            BackgroundColor(if attention {
                Color::srgb(0.35, 0.15, 0.03)
            } else {
                Color::srgb(0.09, 0.12, 0.14)
            }),
        ))
        .with_children(|panel| {
            panel.spawn((
                Text::new(format!("NEXT team score: {}", snapshot.score)),
                TextFont {
                    font_size: bevy::text::FontSize::Px(18.0),
                    ..default()
                },
                TextColor(Color::WHITE),
            ));
            for (rover, id, collected, waiting, dumper) in &snapshot.rows {
                let load = RoverBallLoad {
                    collected: *collected,
                    waiting: *waiting,
                    dumper: *dumper,
                };
                let label = if load.needs_operator() {
                    format!(
                        "Rover {id}: {}/{} balls\nState: {}\nHeld: {waiting}  Dumper: {dumper}/{}",
                        load.count(),
                        ROVER_BALL_CAPACITY,
                        load.state_label(),
                        DUMPER_CAPACITY
                    )
                } else {
                    format!(
                        "Rover {id}: {}/{} balls\nState: {}",
                        load.count(),
                        ROVER_BALL_CAPACITY,
                        load.state_label()
                    )
                };
                panel
                    .spawn((
                        Node {
                            width: Val::Percent(100.0),
                            flex_direction: FlexDirection::Column,
                            row_gap: Val::Px(3.0),
                            padding: UiRect::all(Val::Px(6.0)),
                            ..default()
                        },
                        BackgroundColor(if load.needs_operator() {
                            Color::srgb(0.78, 0.31, 0.03)
                        } else {
                            Color::srgb(0.16, 0.20, 0.22)
                        }),
                    ))
                    .with_children(|row| {
                        row.spawn((
                            Text::new(label),
                            TextFont {
                                font_size: bevy::text::FontSize::Px(14.0),
                                ..default()
                            },
                            TextColor(Color::WHITE),
                        ));
                        if *waiting > 0 && *dumper < DUMPER_CAPACITY {
                            row.spawn((
                                Button,
                                LoadRover(*rover),
                                Node {
                                    padding: UiRect::all(Val::Px(5.0)),
                                    ..default()
                                },
                                BackgroundColor(Color::srgb(0.28, 0.14, 0.03)),
                            ))
                            .with_children(|button| {
                                button.spawn((
                                    Text::new("LOAD 1 INTO DUMPER"),
                                    TextColor(Color::WHITE),
                                    TextFont {
                                        font_size: bevy::text::FontSize::Px(13.0),
                                        ..default()
                                    },
                                ));
                            });
                        }
                        if *dumper > 0 {
                            row.spawn((
                                Button,
                                DumpRover(*rover),
                                Node {
                                    padding: UiRect::all(Val::Px(5.0)),
                                    ..default()
                                },
                                BackgroundColor(Color::srgb(0.03, 0.20, 0.45)),
                            ))
                            .with_children(|button| {
                                button.spawn((
                                    Text::new(format!("DUMP {dumper} INTO BUCKET")),
                                    TextColor(Color::WHITE),
                                    TextFont {
                                        font_size: bevy::text::FontSize::Px(13.0),
                                        ..default()
                                    },
                                ));
                            });
                        }
                    });
            }
        });
    *previous = Some(snapshot);
}

/// Draws a ball that has crossed a bucket mouth toward the back wall so it stays deposited.
fn settle_deposits(
    time: Res<Time<Fixed>>,
    mut commands: Commands,
    mut score: Option<ResMut<NextTeamScore>>,
    mut balls: Query<
        (Entity, &Position, &mut LinearVelocity, Option<&DumpedBall>),
        With<TennisBall>,
    >,
    bins: Query<&Bucket>,
) {
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }
    let damp = (-5.0 * dt).exp();
    for (ball, position, mut velocity, dumped) in &mut balls {
        for bucket in &bins {
            if !bucket.contains(position.0) {
                continue;
            }
            if dumped.is_some() {
                if let Some(score) = score.as_mut() {
                    score.0 += 1;
                    commands.entity(ball).remove::<DumpedBall>();
                }
            }
            let to_rest = Vec3::new(bucket.rest.x - position.x, 0.0, bucket.rest.z - position.z);
            let pull = to_rest.clamp_length_max(0.25) * 8.0;
            velocity.x += pull.x * dt;
            velocity.z += pull.z * dt;
            velocity.x *= damp;
            velocity.z *= damp;
            break;
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
        assert!(in_front_intake(Vec3::new(0.22, -0.287, 0.08)));
        assert!(!in_front_intake(Vec3::new(0.27, -0.287, 0.08)));
        assert!(!in_front_intake(Vec3::new(0.0, -0.287, 0.25)));
        assert!(!in_front_intake(Vec3::new(0.0, 0.0, -0.25)));
    }

    #[test]
    fn crab_trap_collects_ball_behind_side_flap_but_leaves_rear_ball() {
        let mut app = App::new();
        app.add_systems(FixedUpdate, collect_crab_trap_balls);
        let rover = app
            .world_mut()
            .spawn((
                Rover,
                CrabTrapIntake,
                Position(Vec3::new(0.0, 0.32, 0.0)),
                Rotation::default(),
                RoverBallLoad::default(),
            ))
            .id();
        let spawn_ball = |app: &mut App, x, z| {
            app.world_mut()
                .spawn((
                    TennisBall,
                    Position(Vec3::new(x, TENNIS_BALL_RADIUS, z)),
                    Mesh3d(Handle::<Mesh>::default()),
                    MeshMaterial3d(Handle::<StandardMaterial>::default()),
                ))
                .id()
        };
        let front = spawn_ball(&mut app, 0.0, -0.25);
        let inside_side = spawn_ball(&mut app, 0.22, 0.08);
        let outside_side = spawn_ball(&mut app, 0.27, 0.08);
        let rear = spawn_ball(&mut app, 0.0, 0.25);
        app.world_mut().run_schedule(FixedUpdate);
        assert!(app.world().get_entity(front).is_err());
        assert!(app.world().get_entity(inside_side).is_err());
        assert!(app.world().get_entity(outside_side).is_ok());
        assert!(app.world().get_entity(rear).is_ok());
        assert_eq!(app.world().get::<RoverBallLoad>(rover).unwrap().count(), 2);
        let mut carried = app.world_mut().query::<&CarriedBall>();
        assert_eq!(carried.iter(app.world()).count(), 2);
    }

    #[test]
    fn each_pickup_load_and_dump_obeys_dumper_capacity() {
        let mut first = RoverBallLoad::default();
        let mut second = RoverBallLoad::default();
        for _ in 0..3 {
            assert!(first.pickup());
        }
        assert!(second.pickup());
        assert_eq!(first.state_label(), "CARRYING BALLS");
        assert_eq!((first.count(), second.count()), (3, 1));
        assert!(!first.load_dumper());
        first.queue_at_goal();
        assert_eq!(first.state_label(), "WAITING FOR OPERATOR");
        assert_eq!((first.collected, first.waiting, first.dumper), (0, 3, 0));
        assert!(!first.pickup()); // A waiting rover cannot collect another ball.
        for _ in 0..DUMPER_CAPACITY {
            assert!(first.load_dumper());
        }
        assert!(!first.load_dumper());
        assert_eq!(
            (first.waiting, first.dumper, first.count()),
            (3 - DUMPER_CAPACITY, DUMPER_CAPACITY, 3)
        );
        assert_eq!(first.dump(), DUMPER_CAPACITY);
        assert_eq!(first.count(), 3 - DUMPER_CAPACITY);
        assert_eq!(first.dump(), 0);
        for _ in DUMPER_CAPACITY..3 {
            assert!(first.load_dumper());
            assert_eq!(first.dump(), 1);
        }
        assert_eq!((first.count(), second.count()), (0, 1));
        assert_eq!(first.state_label(), "SEARCHING");
        assert!(first.pickup()); // Normal collection resumes after the final click.
        assert_eq!(first.count(), 1);
    }

    #[test]
    fn full_rover_leaves_ball_on_field_until_it_scores() {
        let mut app = App::new();
        app.add_systems(FixedUpdate, collect_crab_trap_balls);
        let rover = app
            .world_mut()
            .spawn((
                Rover,
                CrabTrapIntake,
                Position(Vec3::new(0.0, 0.32, 0.0)),
                Rotation::default(),
                RoverBallLoad {
                    collected: ROVER_BALL_CAPACITY,
                    waiting: 0,
                    dumper: 0,
                },
            ))
            .id();
        let ball = app
            .world_mut()
            .spawn((
                TennisBall,
                Position(Vec3::new(0.0, TENNIS_BALL_RADIUS, -0.25)),
                Mesh3d(Handle::<Mesh>::default()),
                MeshMaterial3d(Handle::<StandardMaterial>::default()),
            ))
            .id();
        app.world_mut().run_schedule(FixedUpdate);
        assert!(app.world().get_entity(ball).is_ok());
        let load = app.world().get::<RoverBallLoad>(rover).unwrap();
        assert_eq!(load.count(), ROVER_BALL_CAPACITY);
        assert_eq!(load.state_label(), "FULL: RETURN TO GOAL");
        let mut carried = app.world_mut().query::<&CarriedBall>();
        assert_eq!(carried.iter(app.world()).count(), 0);
        let mut load = app.world_mut().get_mut::<RoverBallLoad>(rover).unwrap();
        load.queue_at_goal();
        for _ in 0..ROVER_BALL_CAPACITY / DUMPER_CAPACITY {
            let mut load = app.world_mut().get_mut::<RoverBallLoad>(rover).unwrap();
            for _ in 0..DUMPER_CAPACITY {
                assert!(load.load_dumper());
            }
            assert_eq!(load.dump(), DUMPER_CAPACITY);
        }
        app.world_mut().run_schedule(FixedUpdate);
        assert!(app.world().get_entity(ball).is_err());
        assert_eq!(app.world().get::<RoverBallLoad>(rover).unwrap().count(), 1);
    }

    #[test]
    fn eighth_pickup_closes_flaps_and_front_collider_until_scoring_finishes() {
        let physics = RoverPhysicsConfig::from_body(&zorvane_vehicle::CrabTrapBot).unwrap();
        let mut app = App::new();
        app.insert_resource(physics.clone()).add_systems(
            FixedUpdate,
            (collect_crab_trap_balls, sync_full_crab_traps).chain(),
        );
        let rover = app
            .world_mut()
            .spawn((
                CrabTrapIntake,
                Position(Vec3::new(0.0, 0.32, 0.0)),
                Rotation::default(),
                RoverBallLoad {
                    collected: ROVER_BALL_CAPACITY - 1,
                    waiting: 0,
                    dumper: 0,
                },
                RoverBody::crab_trap_collider(physics.chassis_size, false),
            ))
            .id();
        let flap = app
            .world_mut()
            .spawn((
                TrapFlap { side: 1.0 },
                Transform::from_rotation(Quat::from_rotation_y(1.0)),
            ))
            .id();
        app.world_mut().entity_mut(rover).add_child(flap);
        app.world_mut().spawn((
            TennisBall,
            Position(Vec3::new(0.0, TENNIS_BALL_RADIUS, -0.25)),
            Mesh3d(Handle::<Mesh>::default()),
            MeshMaterial3d(Handle::<StandardMaterial>::default()),
        ));
        app.world_mut().run_schedule(FixedUpdate);
        assert_eq!(app.world().get::<RoverBallLoad>(rover).unwrap().count(), 8);
        assert!(app.world().get::<TrapFrontClosed>(rover).is_some());
        assert_eq!(
            app.world()
                .get::<Collider>(rover)
                .unwrap()
                .shape()
                .as_compound()
                .unwrap()
                .shapes()
                .len(),
            5
        );
        let front_center = Vec3::new(0.0, 0.0, -(physics.chassis_size.z - 0.065) * 0.5);
        assert!(app.world().get::<Collider>(rover).unwrap().contains_point(
            Vec3::ZERO,
            Quat::IDENTITY,
            front_center
        ));
        assert_eq!(
            app.world().get::<Transform>(flap).unwrap().rotation,
            Quat::IDENTITY
        );
        let mut load = app.world_mut().get_mut::<RoverBallLoad>(rover).unwrap();
        load.queue_at_goal();
        assert!(load.load_dumper());
        assert_eq!(load.dump(), 1);
        app.world_mut().run_schedule(FixedUpdate);
        assert!(app.world().get::<TrapFrontClosed>(rover).is_some());
        for _ in 1..ROVER_BALL_CAPACITY {
            let mut load = app.world_mut().get_mut::<RoverBallLoad>(rover).unwrap();
            assert!(load.load_dumper());
            assert_eq!(load.dump(), 1);
        }
        app.world_mut().run_schedule(FixedUpdate);
        assert!(app.world().get::<TrapFrontClosed>(rover).is_none());
        assert_eq!(
            app.world()
                .get::<Collider>(rover)
                .unwrap()
                .shape()
                .as_compound()
                .unwrap()
                .shapes()
                .len(),
            4
        );
        assert!(!app.world().get::<Collider>(rover).unwrap().contains_point(
            Vec3::ZERO,
            Quat::IDENTITY,
            front_center
        ));
    }

    #[test]
    fn one_button_press_loads_one_ball_even_while_mouse_is_held() {
        let mut app = App::new();
        app.insert_resource(WorldConfig::for_choice(crate::session::WorldChoice::Next))
            .insert_resource(RoverPhysicsConfig::default())
            .insert_resource(NextTeamScore::default())
            .insert_resource(ButtonInput::<MouseButton>::default())
            .add_systems(Update, load_hud_clicks);
        let rover = app
            .world_mut()
            .spawn((
                crate::terra::RoverSlot(0),
                Position(Vec3::new(0.0, 0.32, PITCH_SOUTH - 0.5)),
                RoverBallLoad {
                    collected: 0,
                    waiting: 2,
                    dumper: 0,
                },
            ))
            .id();
        app.world_mut().entity_mut(rover).with_children(|parent| {
            parent.spawn((
                CarriedBall,
                Mesh3d(Handle::<Mesh>::default()),
                MeshMaterial3d(Handle::<StandardMaterial>::default()),
                Transform::default(),
            ));
        });
        app.world_mut()
            .spawn((LoadRover(rover), Interaction::Pressed));
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        app.update();
        assert_eq!(app.world().get::<RoverBallLoad>(rover).unwrap().waiting, 1);
        assert_eq!(app.world().get::<RoverBallLoad>(rover).unwrap().dumper, 1);
        assert_eq!(app.world().resource::<NextTeamScore>().0, 0);
        let mut staged = app.world_mut().query::<(&DumperBall, &Transform)>();
        let staged_balls: Vec<_> = staged.iter(app.world()).collect();
        assert_eq!(staged_balls.len(), 1);
        assert!((staged_balls[0].1.translation.y - DUMPER_BALL_LOCAL_Y).abs() < 1e-5);
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .clear();
        app.update();
        assert_eq!(app.world().get::<RoverBallLoad>(rover).unwrap().waiting, 1);
        assert_eq!(app.world().get::<RoverBallLoad>(rover).unwrap().dumper, 1);
        assert_eq!(app.world().resource::<NextTeamScore>().0, 0);
    }

    #[test]
    fn dump_button_releases_full_dumper_without_scoring_early() {
        let mut app = physics_app();
        app.insert_resource(WorldConfig::for_choice(crate::session::WorldChoice::Next))
            .insert_resource(NextTeamScore::default())
            .insert_resource(Assets::<Mesh>::default())
            .insert_resource(Assets::<StandardMaterial>::default())
            .insert_resource(ButtonInput::<MouseButton>::default())
            .add_systems(Update, dump_hud_clicks);
        let goal = goal_bucket_place(true);
        app.world_mut().spawn((
            goal.volume(),
            RigidBody::Static,
            bucket_collider(),
            Transform::from_xyz(goal.x, 0.0, goal.z),
        ));
        let rover = app
            .world_mut()
            .spawn((
                crate::terra::RoverSlot(0),
                Position(Vec3::new(0.0, 0.32, PITCH_SOUTH - 0.5)),
                RoverBallLoad {
                    collected: 0,
                    waiting: 0,
                    dumper: DUMPER_CAPACITY,
                },
            ))
            .id();
        app.world_mut().entity_mut(rover).with_children(|parent| {
            for _ in 0..DUMPER_CAPACITY {
                parent.spawn((DumperBall, Transform::default()));
            }
        });
        app.world_mut()
            .spawn((DumpRover(rover), Interaction::Pressed));
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        app.update();
        assert_eq!(app.world().get::<RoverBallLoad>(rover).unwrap().dumper, 0);
        assert_eq!(app.world().resource::<NextTeamScore>().0, 0);
        assert_eq!(
            app.world_mut()
                .query::<&DumpedBall>()
                .iter(app.world())
                .count(),
            DUMPER_CAPACITY
        );
        assert_eq!(
            app.world_mut()
                .query::<&DumperBall>()
                .iter(app.world())
                .count(),
            0
        );
        app.world_mut()
            .resource_mut::<ButtonInput<MouseButton>>()
            .clear();
        tick(&mut app, 180);
        assert_eq!(app.world().resource::<NextTeamScore>().0, DUMPER_CAPACITY);
    }

    #[test]
    fn arrival_at_own_goal_queues_balls_and_stops_the_rover() {
        let mut app = App::new();
        app.insert_resource(RoverPhysicsConfig::default())
            .add_systems(FixedUpdate, queue_balls_at_goal);
        let rover = app
            .world_mut()
            .spawn((
                Rover,
                crate::terra::RoverSlot(0),
                Position(Vec3::new(0.0, 0.32, PITCH_SOUTH - 0.5)),
                RoverBallLoad {
                    collected: 2,
                    waiting: 0,
                    dumper: 0,
                },
                DriveCommand {
                    linear: 1.0,
                    angular: 0.0,
                },
                LinearVelocity(Vec3::new(0.0, 0.0, -1.0)),
                AngularVelocity::ZERO,
            ))
            .id();
        app.world_mut().run_schedule(FixedUpdate);
        assert_eq!(app.world().get::<RoverBallLoad>(rover).unwrap().waiting, 2);
        assert_eq!(app.world().get::<DriveCommand>(rover).unwrap().linear, 0.0);
        assert_eq!(app.world().get::<LinearVelocity>(rover).unwrap().z, 0.0);
        assert!(at_home_goal(
            1,
            Vec3::new(0.0, 0.0, PITCH_NORTH + 0.5),
            Vec3::splat(0.6)
        ));
        assert!(!at_home_goal(
            1,
            Vec3::new(0.0, 0.0, PITCH_SOUTH - 0.5),
            Vec3::splat(0.6)
        ));
    }

    #[test]
    fn layout_fits_the_pitch_and_leaves_the_spawn_clear() {
        let spawn = Vec2::ZERO;
        let mut balls = ball_positions();
        assert_eq!(balls.len(), BALLS_PER_GOAL * 2);
        let mut half_counts = [0; 2];
        let mut penalty_counts = [0; 2];
        let mut x_ranges = [(f32::INFINITY, f32::NEG_INFINITY); 2];
        for ball in &balls {
            assert!(on_pitch(ball.x, ball.z, 0.5), "{ball:?}");
            assert!((ball.y - TENNIS_BALL_RADIUS).abs() < 1e-4);
            assert!(Vec2::new(ball.x, ball.z).distance(spawn) > 1.5);
            assert!(ball.z.abs() >= 40.0, "ball is near field center: {ball:?}");
            let half = usize::from(ball.z > 0.0);
            half_counts[half] += 1;
            if ball.x.abs() <= PENALTY_AREA_WIDTH * 0.5
                && ball.z.abs() >= PITCH_SOUTH - PENALTY_AREA_DEPTH
            {
                penalty_counts[half] += 1;
            }
            x_ranges[half].0 = x_ranges[half].0.min(ball.x);
            x_ranges[half].1 = x_ranges[half].1.max(ball.x);
        }
        assert_eq!(half_counts, [BALLS_PER_GOAL, BALLS_PER_GOAL]);
        assert_eq!(penalty_counts, [30, 30]);
        assert!(x_ranges.iter().all(|(left, right)| right - left >= 60.0));
        balls.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.z.total_cmp(&b.z)));
        for pair in balls.windows(2) {
            assert!(pair[0].distance(pair[1]) > 0.5, "{pair:?}");
        }
        assert!(WORLD_SIZE * 0.5 > PITCH_HALF_WIDTH + 1.0);
        assert!(WORLD_SIZE * 0.5 > PITCH_SOUTH.abs().max(PITCH_NORTH.abs()) + 1.0);
        assert_eq!(PITCH_LENGTH, 105.0);
        assert_eq!(PITCH_WIDTH, 68.0);
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
        let mut goal_buckets = [0usize, 0];
        for place in &placed {
            let bucket = place.volume();
            assert!(bucket.contains(bucket.rest), "{place:?}");
            assert!(on_pitch(place.x, place.z, BUCKET_TOP_DIAMETER * 0.5));
            let goal = goal_bucket_place(place.z > 0.0);
            if *place == goal {
                goal_buckets[usize::from(place.z > 0.0)] += 1;
            } else {
                assert!(
                    place.z < PITCH_SOUTH - SPAWN_CLEARANCE
                        && place.z > PITCH_NORTH + SPAWN_CLEARANCE,
                    "bucket overlaps a goal-line spawn: {place:?}"
                );
            }
            assert!(place.z.abs() > 0.5, "bucket sits on the halfway line");
            halves[usize::from(place.z > 0.0)] += 1;
            for ball in &ball_positions() {
                assert!(Vec2::new(place.x, place.z).distance(Vec2::new(ball.x, ball.z)) > 0.7);
            }
        }
        assert_eq!(halves, [BUCKETS_PER_HALF, BUCKETS_PER_HALF]);
        assert_eq!(goal_buckets, [1, 1]);
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
            .insert_resource(NextTeamScore(3))
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
        assert_eq!(world.resource::<NextTeamScore>().0, 0);
        assert_eq!(
            world.query::<&TennisBall>().iter(world).count(),
            ball_positions().len()
        );
        assert_eq!(
            world.query::<&Bucket>().iter(world).count(),
            buckets(bucket_seed()).len()
        );
        let colored_buckets: Vec<_> = world
            .query::<(&Bucket, &BucketSide, &MeshMaterial3d<StandardMaterial>)>()
            .iter(world)
            .map(|(bucket, side, material)| (bucket.center_z, *side, material.0.clone()))
            .collect();
        let mut side_counts = [0usize; 2];
        for (z, side, handle) in colored_buckets {
            assert_eq!(side, BucketSide::at(z));
            side_counts[usize::from(side == BucketSide::Orange)] += 1;
            let color = world
                .resource::<Assets<StandardMaterial>>()
                .get(&handle)
                .unwrap()
                .base_color;
            assert_eq!(
                color,
                match side {
                    BucketSide::Orange => Color::srgb(0.95, 0.38, 0.06),
                    BucketSide::Blue => Color::srgb(0.05, 0.34, 0.88),
                }
            );
        }
        assert_eq!(side_counts, [BUCKETS_PER_HALF, BUCKETS_PER_HALF]);
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

    #[test]
    fn crab_trap_cannot_drive_through_bucket_from_any_side() {
        let physics = RoverPhysicsConfig::from_body(&zorvane_vehicle::CrabTrapBot).unwrap();
        for direction in [Vec3::NEG_Z, Vec3::Z, Vec3::NEG_X, Vec3::X] {
            let mut app = physics_app();
            let bucket_position = direction * 0.8;
            app.world_mut().spawn((
                BucketPlace::open(bucket_position.x, bucket_position.z).volume(),
                RigidBody::Static,
                bucket_collider(),
                Transform::from_translation(bucket_position),
            ));
            let rover = app
                .world_mut()
                .spawn((
                    RoverBody::new(&physics).with_front_intake(physics.chassis_size),
                    Transform::from_xyz(0.0, physics.chassis_size.y * 0.5 + 0.02, 0.0),
                ))
                .id();
            app.world_mut().get_mut::<LinearVelocity>(rover).unwrap().0 = direction * 1.5;
            tick(&mut app, 120);
            let travel = app.world().get::<Position>(rover).unwrap().0.dot(direction);
            assert!(
                travel < 0.65,
                "rover crossed bucket from {direction:?}: {travel}"
            );
        }
    }

    #[test]
    fn only_dumped_balls_score_after_entering_a_bucket() {
        let mut app = physics_app();
        app.insert_resource(NextTeamScore::default());
        app.world_mut().spawn((
            BucketPlace::open(0.0, 0.0).volume(),
            RigidBody::Static,
            bucket_collider(),
            Transform::default(),
        ));
        let dumped = app
            .world_mut()
            .spawn((
                TennisBall,
                DumpedBall,
                RigidBody::Dynamic,
                Collider::sphere(TENNIS_BALL_RADIUS),
                Mass(TENNIS_BALL_MASS),
                Restitution::new(0.05),
                SweptCcd::default(),
                Transform::from_xyz(0.0, BUCKET_BODY_HEIGHT + 0.14, 0.0),
            ))
            .id();
        assert_eq!(app.world().resource::<NextTeamScore>().0, 0);
        tick(&mut app, 120);
        assert_eq!(app.world().resource::<NextTeamScore>().0, 1);
        assert!(app.world().get::<DumpedBall>(dumped).is_none());
        app.world_mut().spawn((
            TennisBall,
            RigidBody::Dynamic,
            Collider::sphere(TENNIS_BALL_RADIUS),
            Mass(TENNIS_BALL_MASS),
            Transform::from_xyz(0.0, BUCKET_BODY_HEIGHT + 0.14, 0.0),
        ));
        tick(&mut app, 120);
        assert_eq!(app.world().resource::<NextTeamScore>().0, 1);
    }
}
