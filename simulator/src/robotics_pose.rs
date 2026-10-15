//! Rover pose in the robotics frame shared with Terra's waypoint follower.
//!
//! `x` is −Bevy Z, `y` is −Bevy X, and yaw 0 faces Bevy −Z (robotics +x).

use bevy::prelude::*;

pub fn robotics_pose(translation: Vec3, rotation: Quat) -> (f64, f64, f64) {
    let forward = rotation * Vec3::NEG_Z;
    (
        f64::from(-translation.z),
        f64::from(-translation.x),
        f64::from((-forward.x).atan2(-forward.z)),
    )
}

/// JSON body of `terra/rover/<id>/pose`. `heading` and `yaw` are the same radians.
#[cfg_attr(not(feature = "native"), allow(dead_code))]
pub fn pose_payload(x: f64, y: f64, yaw: f64) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "x": x,
        "y": y,
        "yaw": yaw,
        "heading": yaw,
    }))
    .expect("pose json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pose_json_names_heading_as_yaw() {
        let bytes = pose_payload(1.25, -0.5, 0.3);
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["x"], 1.25);
        assert_eq!(value["y"], -0.5);
        assert_eq!(value["yaw"], 0.3);
        assert_eq!(value["heading"], value["yaw"]);
    }
}
