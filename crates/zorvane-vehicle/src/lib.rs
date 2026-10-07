//! Vehicle bodies that plug into Zorvane.
//!
//! Zorvane owns the world. A chassis, drone, or later import implements
//! [`VehicleBody`] and is registered on [`VehicleRegistry`]. The Terra ground
//! rover ships as [`TerraGround`] (`terra-ground`) so the current simulator
//! still drives that chassis. Terra #22 (easy robot import and flexible
//! actuators) should construct a [`VehicleBody`]; it should not add world
//! geometry inside a vehicle repo.
//!
//! [`Locomotion::Unsupported`] is the extension point for bodies this build
//! cannot actuate yet. Selecting one succeeds. Turning it into wheel speeds
//! does not.

/// Axis-aligned chassis box in the simulator frame: X right, Y up, Z depth.
/// Differential drive treats local −Z as forward.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChassisSpec {
    pub size_xyz: [f32; 3],
    pub mass_kg: f32,
    /// Uniform scale applied to the visual asset when it is parented to the body.
    pub visual_scale: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DifferentialDriveSpec {
    pub wheel_radius_m: f32,
    pub track_width_m: f32,
    pub max_wheel_speed_rad_s: f32,
    pub keyboard_linear_speed: f32,
    pub keyboard_angular_speed: f32,
}

/// How Zorvane turns a body-velocity request into motion.
///
/// A new variant is a new actuator family. Call sites that only implement
/// differential drive must handle the other variants (today: return an error).
#[derive(Clone, Debug, PartialEq)]
pub enum Locomotion {
    DifferentialDrive(DifferentialDriveSpec),
    /// Registered so a later body can be selected without pretending it is a rover.
    Unsupported {
        reason: &'static str,
    },
}

impl Locomotion {
    pub fn differential(&self) -> Result<&DifferentialDriveSpec, &'static str> {
        match self {
            Locomotion::DifferentialDrive(spec) => Ok(spec),
            Locomotion::Unsupported { reason } => Err(reason),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VehicleVisual {
    /// Path relative to the simulator asset root (`simulator/assets`, or `ZORVANE_ASSETS`).
    pub asset_path: &'static str,
}

pub trait VehicleBody: Send + Sync {
    fn id(&self) -> &'static str;
    fn display_name(&self) -> &'static str;
    fn visual(&self) -> VehicleVisual;
    fn chassis(&self) -> ChassisSpec;
    fn locomotion(&self) -> Locomotion;
}

/// Terra's ground chassis: the GLB, collider, and differential-drive geometry
/// that used to be constants inside the Terra simulator.
#[derive(Clone, Copy, Debug, Default)]
pub struct TerraGround;

impl VehicleBody for TerraGround {
    fn id(&self) -> &'static str {
        "terra-ground"
    }

    fn display_name(&self) -> &'static str {
        "Terra"
    }

    fn visual(&self) -> VehicleVisual {
        VehicleVisual {
            asset_path: "models/rover.glb",
        }
    }

    fn chassis(&self) -> ChassisSpec {
        ChassisSpec {
            size_xyz: [0.9, 0.16, 0.8],
            mass_kg: 20.0,
            visual_scale: 0.025,
        }
    }

    fn locomotion(&self) -> Locomotion {
        Locomotion::DifferentialDrive(DifferentialDriveSpec {
            wheel_radius_m: 0.15,
            track_width_m: 0.6,
            max_wheel_speed_rad_s: 20.0,
            keyboard_linear_speed: 1.5,
            keyboard_angular_speed: 1.5,
        })
    }
}

/// Bodies Zorvane can spawn. The binary registers built-ins, then selects one.
pub struct VehicleRegistry {
    bodies: Vec<Box<dyn VehicleBody>>,
    active: usize,
}

impl VehicleRegistry {
    pub fn new() -> Self {
        Self {
            bodies: Vec::new(),
            active: 0,
        }
    }

    pub fn register(&mut self, body: Box<dyn VehicleBody>) {
        self.bodies.push(body);
    }

    pub fn select(&mut self, id: &str) -> Result<(), String> {
        let Some(index) = self.bodies.iter().position(|body| body.id() == id) else {
            let known: Vec<_> = self.bodies.iter().map(|body| body.id()).collect();
            return Err(format!("unknown vehicle '{id}', registered: {known:?}"));
        };
        self.active = index;
        Ok(())
    }

    pub fn active(&self) -> &dyn VehicleBody {
        self.bodies
            .get(self.active)
            .expect("vehicle registry has no bodies")
            .as_ref()
    }

    /// Built-in bodies shipped with Zorvane. `id` `None` selects Terra's ground rover.
    pub fn from_id(id: Option<&str>) -> Result<Self, String> {
        let mut registry = Self::new();
        registry.register(Box::new(TerraGround));
        registry.select(id.unwrap_or(TerraGround.id()))?;
        Ok(registry)
    }

    /// `ZORVANE_VEHICLE` selects a registered id. Unset means `terra-ground`.
    pub fn from_env() -> Result<Self, String> {
        let id = std::env::var("ZORVANE_VEHICLE").ok();
        Self::from_id(id.as_deref())
    }
}

impl Default for VehicleRegistry {
    fn default() -> Self {
        Self::from_id(None).expect("terra-ground is registered")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct PlaceholderDrone;

    impl VehicleBody for PlaceholderDrone {
        fn id(&self) -> &'static str {
            "placeholder-drone"
        }
        fn display_name(&self) -> &'static str {
            "Placeholder drone"
        }
        fn visual(&self) -> VehicleVisual {
            VehicleVisual {
                asset_path: "models/drone.glb",
            }
        }
        fn chassis(&self) -> ChassisSpec {
            ChassisSpec {
                size_xyz: [0.4, 0.15, 0.4],
                mass_kg: 1.5,
                visual_scale: 1.0,
            }
        }
        fn locomotion(&self) -> Locomotion {
            Locomotion::Unsupported {
                reason: "drone actuators are not implemented",
            }
        }
    }

    #[test]
    fn terra_ground_keeps_the_extracted_chassis_numbers() {
        let body = TerraGround;
        assert_eq!(body.id(), "terra-ground");
        assert_eq!(body.display_name(), "Terra");
        assert_eq!(body.visual().asset_path, "models/rover.glb");
        assert_eq!(
            body.chassis(),
            ChassisSpec {
                size_xyz: [0.9, 0.16, 0.8],
                mass_kg: 20.0,
                visual_scale: 0.025,
            }
        );
        let locomotion = body.locomotion();
        let drive = locomotion.differential().unwrap();
        assert_eq!(drive.wheel_radius_m, 0.15);
        assert_eq!(drive.track_width_m, 0.6);
        assert_eq!(drive.max_wheel_speed_rad_s, 20.0);
        assert_eq!(drive.keyboard_linear_speed, 1.5);
        assert_eq!(drive.keyboard_angular_speed, 1.5);
    }

    #[test]
    fn registry_defaults_to_terra_and_accepts_another_body() {
        let registry = VehicleRegistry::from_id(None).unwrap();
        assert_eq!(registry.active().id(), "terra-ground");

        let mut registry = VehicleRegistry::from_id(Some("terra-ground")).unwrap();
        registry.register(Box::new(PlaceholderDrone));
        registry.select("placeholder-drone").unwrap();
        assert_eq!(registry.active().id(), "placeholder-drone");
        assert!(registry.active().locomotion().differential().is_err());
        assert!(registry.select("missing").is_err());
    }
}
