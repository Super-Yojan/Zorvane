//! Field 3, West Campus, George Mason University: where the NEXT pitch sits
//! on Earth, and the authored world around it.
//!
//! Nothing here uses Google data. The NEXT 2026 rules place the competition on
//! Field 3, West Campus, George Mason University (Fairfax): OpenStreetMap way
//! 233194680. Its height is an independent USGS/GEOID18 value (see
//! `simulator/NEXT.md`).
//! The surroundings are `tools/blender/build_next_assets.py` output, built from
//! the mapped OpenStreetMap footprints around that pitch. Robot cameras, depth,
//! physics and the simulated GPS all see the same authored world.

use crate::next_competition::NextField;
use crate::world::WorldConfig;
use avian3d::prelude::*;
use bevy::prelude::*;

pub const SURROUNDINGS_MODEL: &str = "models/mason_surroundings.glb";
pub const BUCKET_MODEL: &str = "models/homer_bucket.glb";
pub const BLUE_BUCKET_MODEL: &str = "models/blue_bucket.glb";
pub const TURF_TEXTURE: &str = "textures/turf.png";
pub const TENNIS_BALL_TEXTURE: &str = "textures/tennis_ball.png";
/// Metres covered by one repeat of the turf texture.
pub const TURF_REPEAT: f32 = 4.0;
/// Blender object names with this prefix get static trimesh colliders.
const SOLID_PREFIX: &str = "solid:";

/// Independent nominal placement of the NEXT pitch centre.
/// Center/heading: the mapped outline of Field #3, OpenStreetMap way 233194680
/// (ODbL, © OSM contributors): centroid and long-axis bearing.
/// Height: USGS 116.988838 m + NOAA GEOID18 -31.856 m, approximately 85.13 m.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub latitude: f64,
    pub longitude: f64,
    /// Ellipsoid height of the authored surface, metres.
    pub elevation: f64,
    /// Clockwise from true north of the pitch's forward axis (Bevy -Z, the
    /// north goal, robotics +X).
    pub heading: f64,
}

impl Default for Placement {
    fn default() -> Self {
        Self {
            latitude: 38.833116675,
            longitude: -77.323160625,
            elevation: 85.13,
            heading: 327.04,
        }
    }
}

impl Placement {
    /// `TERRA_GMU_LAT`, `TERRA_GMU_LON`, `TERRA_GMU_ELEVATION` and
    /// `TERRA_GMU_HEADING` override the defaults.
    pub fn from_env() -> Result<Self, String> {
        let mut placement = Self::default();
        for (name, target) in [
            ("TERRA_GMU_LAT", &mut placement.latitude),
            ("TERRA_GMU_LON", &mut placement.longitude),
            ("TERRA_GMU_ELEVATION", &mut placement.elevation),
            ("TERRA_GMU_HEADING", &mut placement.heading),
        ] {
            if let Ok(value) = std::env::var(name) {
                *target = value
                    .parse()
                    .map_err(|_| format!("{name} must be a finite number"))?;
            }
        }
        placement.validate()?;
        Ok(placement)
    }

    pub fn validate(&self) -> Result<(), String> {
        if !self.latitude.is_finite()
            || !(-85.0..=85.0).contains(&self.latitude)
            || !self.longitude.is_finite()
            || !(-180.0..=180.0).contains(&self.longitude)
            || !self.elevation.is_finite()
            || !(-500.0..=9000.0).contains(&self.elevation)
            || !self.heading.is_finite()
            || !(0.0..360.0).contains(&self.heading)
        {
            return Err("GMU placement requires latitude [-85,85], longitude [-180,180], ellipsoid elevation [-500,9000] m and heading [0,360) degrees".into());
        }
        Ok(())
    }

    /// Frame for simulated GPS, heading and the ARGOS localization report.
    pub(crate) fn reference(self) -> crate::outdoor::GeoReference {
        crate::outdoor::GeoReference {
            latitude: self.latitude,
            longitude: self.longitude,
            heading: self.heading,
        }
    }
}

/// The NEXT world's geographic reference. An invalid environment override
/// yields `None` rather than a silently wrong position.
pub(crate) fn geographic_reference() -> Option<crate::outdoor::GeoReference> {
    Placement::from_env().ok().map(Placement::reference)
}

/// Lighting for camera data. The authored scene stays the same; light changes
/// what object detection has to cope with.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Lighting {
    /// Clear midday sun with hard shadows.
    #[default]
    Noon,
    /// Even sky light, no hard shadows.
    Overcast,
    /// Low warm sun, long shadows.
    Evening,
}

impl Lighting {
    /// `TERRA_NEXT_LIGHTING` = `noon` (default), `overcast` or `evening`.
    pub fn from_env() -> Result<Self, String> {
        match std::env::var("TERRA_NEXT_LIGHTING").as_deref() {
            Err(_) | Ok("") | Ok("noon") => Ok(Self::Noon),
            Ok("overcast") => Ok(Self::Overcast),
            Ok("evening") => Ok(Self::Evening),
            Ok(other) => Err(format!(
                "TERRA_NEXT_LIGHTING must be noon, overcast or evening, not {other:?}"
            )),
        }
    }

    /// Sun illuminance (lux), colour, elevation and azimuth (degrees), ambient
    /// brightness, sky colour, and whether the sun casts shadows.
    fn values(self) -> (f32, Color, f32, f32, f32, Color, bool) {
        match self {
            Self::Noon => (
                10_000.0,
                Color::srgb(1.0, 0.98, 0.94),
                62.0,
                200.0,
                220.0,
                Color::srgb(0.62, 0.76, 0.9),
                true,
            ),
            Self::Overcast => (
                3_500.0,
                Color::srgb(0.92, 0.94, 0.98),
                55.0,
                160.0,
                650.0,
                Color::srgb(0.74, 0.77, 0.8),
                false,
            ),
            Self::Evening => (
                5_000.0,
                Color::srgb(1.0, 0.72, 0.48),
                12.0,
                255.0,
                160.0,
                Color::srgb(0.86, 0.66, 0.5),
                true,
            ),
        }
    }
}

#[derive(Component)]
pub struct MasonSurroundings;

pub fn install(app: &mut App) {
    let lighting = Lighting::from_env().expect("invalid NEXT lighting");
    app.insert_resource(lighting)
        .add_systems(Update, (solidify_surroundings, apply_lighting));
}

/// The world around the pitch, unless spectator-only Google scenery replaces it.
pub(crate) fn spawn_surroundings(
    mut commands: Commands,
    assets: Option<Res<AssetServer>>,
    config: Option<Res<WorldConfig>>,
    existing: Query<(), With<MasonSurroundings>>,
) {
    if config.is_none() || !existing.is_empty() {
        return;
    }
    let Some(assets) = assets else {
        return;
    };
    commands.spawn((
        Name::new("Field 3 surroundings"),
        NextField,
        MasonSurroundings,
        WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset(SURROUNDINGS_MODEL))),
        Transform::default(),
    ));
}

/// Gives every `solid:*` Blender object a static trimesh collider once its
/// mesh exists. glTF names the node; the mesh is on its child primitive.
fn solidify_surroundings(
    mut commands: Commands,
    meshes: Query<(Entity, &ChildOf), (Added<Mesh3d>, Without<Collider>)>,
    names: Query<&Name>,
) {
    for (entity, parent) in &meshes {
        let solid = names
            .get(parent.parent())
            .is_ok_and(|name| name.as_str().starts_with(SOLID_PREFIX));
        if solid {
            commands.entity(entity).insert((
                RigidBody::Static,
                ColliderConstructor::TrimeshFromMesh,
                Friction::new(0.8),
                Restitution::new(0.05),
            ));
        }
    }
}

/// Applies the chosen lighting to the world's sun, ambient light and sky.
/// Runs when the sun is (re)spawned, so world rebuilds keep the preset.
fn apply_lighting(
    lighting: Res<Lighting>,
    config: Option<Res<WorldConfig>>,
    mut ambient: ResMut<GlobalAmbientLight>,
    mut sky: ResMut<ClearColor>,
    mut suns: Query<(&mut DirectionalLight, &mut Transform), Added<DirectionalLight>>,
) {
    if config.is_none() {
        return;
    }
    let (lux, color, elevation, azimuth, ambient_brightness, sky_color, shadows) =
        lighting.values();
    for (mut sun, mut transform) in &mut suns {
        sun.illuminance = lux;
        sun.color = color;
        sun.shadow_maps_enabled = shadows;
        let (elevation, azimuth) = (elevation.to_radians(), azimuth.to_radians());
        // Azimuth clockwise from the pitch's north goal (Bevy -Z).
        let toward_sun = Vec3::new(
            elevation.cos() * azimuth.sin(),
            elevation.sin(),
            -elevation.cos() * azimuth.cos(),
        );
        *transform = Transform::from_translation(toward_sun).looking_at(Vec3::ZERO, Vec3::Y);
        ambient.brightness = ambient_brightness;
        sky.0 = sky_color;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_placement_is_field_3_and_feeds_gps() {
        let placement = Placement::default();
        assert!(placement.validate().is_ok());
        let reference = placement.reference();
        assert_eq!(
            (reference.latitude, reference.longitude, reference.heading),
            (38.833116675, -77.323160625, 327.04)
        );
        // The north goal line, 50 m ahead, lies north-north-west of centre.
        let (lat, lon, heading) = reference.sample(terra_navigation::Pose {
            x: 50.0,
            y: 0.0,
            yaw: 0.0,
        });
        let metres_per_degree = 6_371_000.0 * std::f64::consts::PI / 180.0;
        let north = (lat - 38.833116675) * metres_per_degree;
        let east = (lon + 77.323160625) * metres_per_degree * 38.833116675_f64.to_radians().cos();
        assert!((north - 41.952).abs() < 0.05, "north {north}");
        assert!((east + 27.199).abs() < 0.05, "east {east}");
        assert!((heading - 327.04).abs() < 1e-9);
    }

    #[test]
    fn lighting_presets_parse_and_reject_unknown_names() {
        assert_eq!(Lighting::default(), Lighting::Noon);
        for preset in [Lighting::Noon, Lighting::Overcast, Lighting::Evening] {
            let (lux, ..) = preset.values();
            assert!(lux > 0.0);
        }
        assert!(!Lighting::Overcast.values().6);
    }
}
