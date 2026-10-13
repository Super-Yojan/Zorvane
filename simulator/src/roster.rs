//! Editable identities and per-slot settings for the simulated robot fleet.
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RobotDefinition {
    pub enabled: bool,
    pub name: String,
    /// A descriptive label for humans and fleet/state consumers.
    pub description: String,
    /// ID of a body registered by `zorvane-vehicle`.
    pub vehicle: String,
    /// One Zenoh key segment below the configured global prefix.
    pub topic: String,
    #[serde(default)]
    pub color: Option<[f32; 3]>,
}

#[derive(Resource, Clone, Debug)]
pub struct RobotRoster(pub Vec<RobotDefinition>);

impl RobotRoster {
    pub fn from_env(default_vehicle: &str) -> Result<Self, String> {
        let roster = if let Ok(path) = std::env::var("ZORVANE_ROBOT_CONFIG") {
            let bytes = std::fs::read(&path)
                .map_err(|error| format!("cannot read robot config {path}: {error}"))?;
            let definitions: Vec<RobotDefinition> = serde_json::from_slice(&bytes)
                .map_err(|error| format!("invalid robot config {path}: {error}"))?;
            Self(definitions)
        } else {
            let count = std::env::var("TERRA_ROVER_COUNT").map_or(Ok(1usize), |value| {
                value
                    .parse::<usize>()
                    .map_err(|_| "TERRA_ROVER_COUNT must be an integer")
            })?;
            if count > crate::terra::MAX_ROVERS {
                return Err("TERRA_ROVER_COUNT must be between 0 and 32".into());
            }
            Self(
                (0..count)
                    .map(|slot| RobotDefinition {
                        enabled: true,
                        name: format!("robot{}", slot + 1),
                        description: format!("Terra ground rover {}", slot + 1),
                        vehicle: default_vehicle.to_owned(),
                        topic: format!("robot{}", slot + 1),
                        color: None,
                    })
                    .collect(),
            )
        };
        roster.validate()?;
        Ok(roster)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.0.len() > crate::terra::MAX_ROVERS {
            return Err("robot config can contain at most 32 slots".into());
        }
        let mut names = BTreeSet::new();
        let mut topics = BTreeSet::new();
        for (slot, robot) in self.0.iter().enumerate() {
            if robot.name.trim().is_empty()
                || robot.description.trim().is_empty()
                || robot.vehicle.trim().is_empty()
            {
                return Err(format!(
                    "robot slot {} needs a name, description, and vehicle",
                    slot + 1
                ));
            }
            if !valid_key_segment(&robot.topic) || robot.topic.parse::<u64>().is_ok() {
                return Err(format!(
                    "robot slot {} topic must be one non-empty Zenoh key segment",
                    slot + 1
                ));
            }
            if robot.topic == "fleet" {
                return Err("robot topic 'fleet' is reserved for fleet management".into());
            }
            if let Some(other_slot) = robot
                .topic
                .strip_prefix("robot")
                .and_then(|number| number.parse::<usize>().ok())
                .filter(|number| (1..=self.0.len()).contains(number))
                .map(|number| number - 1)
                && other_slot != slot
            {
                return Err(format!(
                    "robot topic '{}' collides with the default topic for slot {}",
                    robot.topic,
                    other_slot + 1
                ));
            }
            if robot.color.is_some_and(|rgb| {
                rgb.iter()
                    .any(|channel| !channel.is_finite() || !(0.0..=1.0).contains(channel))
            }) {
                return Err(format!(
                    "robot slot {} color channels must be finite values in [0,1]",
                    slot + 1
                ));
            }
            if !names.insert(robot.name.clone()) {
                return Err(format!("duplicate robot name '{}'", robot.name));
            }
            if !topics.insert(robot.topic.clone()) {
                return Err(format!("duplicate robot topic '{}'", robot.topic));
            }
        }
        Ok(())
    }

    pub fn slot(&self, slot: usize) -> Option<&RobotDefinition> {
        self.0.get(slot)
    }
}

fn valid_key_segment(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && !value.contains(['/', '*', '#', '$'])
        && zenoh::key_expr::KeyExpr::new(value).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn robot_topic_segments_must_be_unique_and_concrete() {
        let definition = |topic: &str| RobotDefinition {
            enabled: true,
            name: topic.into(),
            description: "test robot".into(),
            vehicle: "terra-ground".into(),
            topic: topic.into(),
            color: None,
        };
        assert!(
            RobotRoster(vec![definition("robot1"), definition("robot2")])
                .validate()
                .is_ok()
        );
        assert!(
            RobotRoster(vec![definition("robot1"), definition("robot1")])
                .validate()
                .is_err()
        );
        assert!(RobotRoster(vec![definition("robot/1")]).validate().is_err());
        assert!(RobotRoster(vec![definition("fleet")]).validate().is_err());
    }

    #[test]
    fn a_roster_keeps_disabled_slots_and_editable_vehicle_metadata() {
        let json = r#"[
          {"enabled":true,"name":"robot1","description":"first","vehicle":"terra-ground","topic":"robot1"},
          {"enabled":false,"name":"robot2","description":"second","vehicle":"terra-ground","topic":"robot2"}
        ]"#;
        let roster = RobotRoster(serde_json::from_str(json).unwrap());
        assert!(roster.validate().is_ok());
        assert_eq!(roster.slot(1).unwrap().name, "robot2");
        assert!(!roster.slot(1).unwrap().enabled);
    }
}
