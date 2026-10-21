//! Simulated sensors and protocol adapter for Terra's shared outdoor controller.
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn geographic_axes_follow_field_heading() {
        let a = GeoReference {
            latitude: 38.,
            longitude: -77.,
            heading: 90.,
        };
        let (lat, lon, heading) = a.sample(terra_navigation::Pose {
            x: 10.,
            y: 0.,
            yaw: 0.,
        });
        assert!((lat - 38.).abs() < 1e-8);
        assert!(lon > -77.);
        assert!((heading - 90.).abs() < 1e-8);
    }
}

use serde_json::Value;
use std::collections::VecDeque;
use terra_autonomy::{
    AutonomyArbiter,
    outdoor::{Evidence, GpsFix, HeadingFix, OutdoorController},
};
use terra_navigation::Pose;
use terra_waypoint::GoalCommand;

#[derive(Clone, Copy)]
pub(crate) struct GeoReference {
    pub latitude: f64,
    pub longitude: f64,
    /// Clockwise bearing of local +X (robotics north).
    pub heading: f64,
}
impl GeoReference {
    pub fn sample(self, pose: Pose) -> (f64, f64, f64) {
        let (s, c) = self.heading.to_radians().sin_cos();
        let north = pose.x * c + pose.y * s;
        let west = -pose.x * s + pose.y * c;
        let metres_per_degree = 6_371_000.0 * std::f64::consts::PI / 180.;
        (
            self.latitude + north / metres_per_degree,
            self.longitude - west / (metres_per_degree * self.latitude.to_radians().cos()),
            (self.heading - pose.yaw.to_degrees()).rem_euclid(360.),
        )
    }
}

pub(crate) struct OutdoorSim {
    pub controller: OutdoorController,
    pub armed: bool,
    tokens: VecDeque<String>,
}
impl Default for OutdoorSim {
    fn default() -> Self {
        Self {
            controller: OutdoorController::default(),
            armed: true,
            tokens: VecDeque::new(),
        }
    }
}
impl OutdoorSim {
    pub fn sensors(&mut self, reference: Option<GeoReference>, pose: Pose, now: f64) {
        self.controller.supported = reference.is_some();
        if let Some(reference) = reference {
            let (latitude, longitude, true_heading) = reference.sample(pose);
            let _ = self.controller.gps(
                GpsFix {
                    timestamp: now,
                    latitude,
                    longitude,
                    accuracy: 1.,
                },
                now,
            );
            let _ = self.controller.heading(
                HeadingFix {
                    timestamp: now,
                    true_heading,
                    accuracy: 1.,
                },
                now,
            );
        } else {
            self.controller.invalidate_sensors();
        }
    }
    fn remember(&mut self, token: &str) -> Result<(), &'static str> {
        if !terra_autonomy::valid_token(token) || self.tokens.iter().any(|t| t == token) {
            return Err("invalid_or_replayed_token");
        }
        if self.tokens.len() == 128 {
            self.tokens.pop_front();
        }
        self.tokens.push_back(token.into());
        Ok(())
    }
    pub fn request(
        &mut self,
        kind: &str,
        bytes: &[u8],
        age: f64,
        now: f64,
        e: Evidence,
        arbiter: &mut AutonomyArbiter,
    ) {
        let result = self.apply(kind, bytes, age, now, e, arbiter);
        self.controller.result = Some(match result {
            Ok(()) => "accepted".into(),
            Err(reason) => format!("rejected: {reason}"),
        });
    }
    fn apply(
        &mut self,
        kind: &str,
        bytes: &[u8],
        age: f64,
        now: f64,
        e: Evidence,
        arbiter: &mut AutonomyArbiter,
    ) -> Result<(), &'static str> {
        let mut v: Value = serde_json::from_slice(bytes).map_err(|_| "invalid_json")?;
        if kind == "hardware" && v["action"] == "disarm" {
            self.armed = false;
            self.controller.pause("disarmed");
            arbiter.clear_guidance();
            return Ok(());
        }
        if kind == "goal" && v["cancel"] == true {
            self.controller.cancel();
            arbiter.accept_goal(&GoalCommand::Cancel, 20_000.);
            return Ok(());
        }
        if !age.is_finite() || !(0.0..0.5).contains(&age) {
            return Err("request_expired");
        }
        if kind == "goal" && !self.controller.selected {
            let g = terra_waypoint::decode_goal(bytes).ok_or("invalid_goal")?;
            return if arbiter.accept_goal(&g, 20_000.) {
                Ok(())
            } else {
                Err("goal_rejected")
            };
        }
        let token = v["token"].as_str().ok_or("missing_token")?.to_owned();
        self.remember(&token)?;
        self.controller.token = Some(token.clone());
        if v["run_id"] != arbiter.run_id {
            return Err("run_changed");
        }
        if kind == "hardware" {
            if v["action"] != "arm"
                || !arbiter.navigation_configuration_allowed(
                    &arbiter.run_id,
                    v["authority_revision"].as_u64().ok_or("revision_missing")?,
                    e.pose,
                )
            {
                return Err("arm_rejected");
            }
            self.armed = true;
            return Ok(());
        }
        if v["navigation_revision"] != self.controller.revision || !self.controller.supported {
            return Err("navigation_changed_or_unsupported");
        }
        match kind {
            "navigation/source" => {
                if v["schema_version"] != 1
                    || self.armed
                    || self.controller.active()
                    || arbiter.has_active_goal(e.pose)
                {
                    return Err("disarm_and_cancel_before_switching");
                }
                let selected = match v["source"].as_str() {
                    Some("local") => false,
                    Some("outdoor_gps") => true,
                    _ => return Err("invalid_source"),
                };
                let radius = v["arrival_radius_m"]
                    .as_f64()
                    .unwrap_or(self.controller.radius);
                if !radius.is_finite() || !(2.0..=20.0).contains(&radius) {
                    return Err("invalid_radius");
                }
                self.controller.revision = self
                    .controller
                    .revision
                    .checked_add(1)
                    .ok_or("revision_exhausted")?;
                self.controller.cancel();
                self.controller.selected = selected;
                self.controller.radius = radius;
                arbiter.clear_guidance();
                Ok(())
            }
            "navigation/resume" => {
                if v["schema_version"] != 1 {
                    return Err("invalid_version");
                }
                self.controller.resume(now, e)
            }
            "goal" => {
                let radius = v["arrival_radius_m"].as_f64().ok_or("missing_radius")?;
                let o = v.as_object_mut().ok_or("invalid_goal")?;
                for key in ["run_id", "navigation_revision", "arrival_radius_m"] {
                    o.remove(key);
                }
                let g = terra_waypoint::decode_goal(&serde_json::to_vec(&v).unwrap())
                    .ok_or("invalid_goal")?;
                let GoalCommand::Wgs84 {
                    latitude,
                    longitude,
                    yaw: None,
                    ..
                } = g
                else {
                    return Err("geographic_goal_required");
                };
                self.controller
                    .start(latitude, longitude, token.clone(), radius, now, e)?;
                let (x, y) = self.controller.tick(now, e).unwrap_or((e.pose.x, e.pose.y));
                if !arbiter.accept_goal(
                    &GoalCommand::Local {
                        x,
                        y,
                        yaw: None,
                        token: Some(token),
                    },
                    20_000.,
                ) {
                    self.controller.cancel();
                    return Err("goal_rejected");
                }
                Ok(())
            }
            _ => Err("unknown_request"),
        }
    }
    pub fn guidance(&mut self, now: f64, e: Evidence, arbiter: &mut AutonomyArbiter) {
        if !self.controller.selected {
            return;
        }
        if let Some((x, y)) = self.controller.tick(now, e) {
            arbiter.update_guidance(x, y);
        }
        if self.controller.phase() != "planning" {
            arbiter.clear_guidance();
        }
    }
}
