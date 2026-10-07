//! Active vehicle body for this process. World systems read it; they do not
//! hard-code a chassis.
use bevy::prelude::*;

#[derive(Resource)]
pub struct Vehicles(pub zorvane_vehicle::VehicleRegistry);
