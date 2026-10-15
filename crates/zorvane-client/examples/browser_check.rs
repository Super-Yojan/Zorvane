//! Interactive end-to-end check against the real browser simulator.
//! Run with an optional play URL (default: hosted WebGPU build), open the
//! printed URL, and press Start. Fails if sensors or full bridge topics do not
//! arrive. Sends a short drive, a waypoint request, cancellation, and safety stop.
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::mpsc,
    time::{Duration, Instant},
};
use zorvane_client::{BrowserHostConfig, Client, Pose, decode_camera_frame, parse_pose};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut config = BrowserHostConfig::default();
    if let Some(url) = std::env::args().nth(1) {
        config.play_url = url;
    }
    let client = Client::host_browser(config)?;
    let url = client.pairing_url().unwrap();
    println!("Open this pairing URL, select a world, and press Start:\n{url}");
    if let Ok(path) = std::env::var("ZORVANE_PAIRING_FILE") {
        std::fs::write(path, &url)?;
    }
    let (sender, receiver) = mpsc::sync_channel(512);
    client.subscribe("terra/rover/**", move |sample| {
        let _ = sender.try_send(sample);
    })?;
    client.wait_connected(Duration::from_secs(600))?;
    let deadline = Instant::now() + Duration::from_secs(600);
    let mut topics = BTreeSet::new();
    let mut poses = BTreeMap::<u64, Pose>::new();
    let mut start_pose = None;
    let mut drive_started = None;
    let mut last_command = Instant::now();
    let mut cameras = BTreeSet::new();
    let mut moved = false;
    let mut goal_sent = false;
    let mut goal_seen = false;
    let mut cancel_seen = false;
    let mut safety_seen = false;
    let mut fleet_seen = false;
    let mut recording_seen = false;
    while Instant::now() < deadline {
        if last_command.elapsed() >= Duration::from_millis(50) {
            if let Some(started) = drive_started {
                let elapsed = Instant::now().duration_since(started).as_secs_f64();
                client.set_velocity(0, if elapsed < 3.0 { 0.5 } else { 0.0 }, 0.0)?;
                if elapsed >= 3.0 && !goal_sent {
                    let pose = poses[&0];
                    client.publish_json(
                        "terra/rover/0/autonomy",
                        &serde_json::json!({"level":"waypoint","token":"browser-check-waypoint"}),
                    )?;
                    client.send_goal(0, &serde_json::json!({"frame":"local","x":pose.x + 2.0,"y":pose.y,"token":"browser-check-goal"}))?;
                    goal_sent = true;
                }
            }
            last_command = Instant::now();
        }
        let sample = match receiver.recv_timeout(Duration::from_millis(50)) {
            Ok(sample) => sample,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(e) => return Err(e.into()),
        };
        let first = topics.insert(sample.topic.clone());
        if first {
            println!("received {}", sample.topic);
        }
        if sample.topic.contains("/camera/") {
            if let Some(frame) = decode_camera_frame(&sample.payload) {
                if frame.header.encoding == "32FC1_LE" {
                    assert!(frame.header.metadata["exposure_time"].as_f64().is_some());
                    assert!(frame.header.metadata["body"].is_object());
                    assert!(frame.depth_metres().unwrap().iter().any(|v| v.is_finite()));
                }
                cameras.insert(frame.header.encoding.clone());
            }
        } else if sample.topic.ends_with("/pose") {
            if let Some(pose) = parse_pose(&sample.payload) {
                let id: u64 = sample.topic.split('/').nth(2).unwrap().parse()?;
                poses.insert(id, pose);
                if id == 0 {
                    if start_pose.is_none() {
                        start_pose = Some(pose);
                        drive_started = Some(Instant::now());
                        client.set_fleet_size(2)?;
                    }
                    let start = start_pose.unwrap();
                    moved |= (pose.x - start.x).hypot(pose.y - start.y) > 0.2;
                }
            }
        } else if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&sample.payload) {
            if sample.topic.ends_with("/fleet/state") {
                fleet_seen |= value["count"] == 2;
            }
            if sample.topic.ends_with("/experiment/status") {
                recording_seen |= value["recording"] == true;
            }
            if sample.topic == "terra/rover/0/goal/status" {
                if value["token"] == "browser-check-goal" && value["state"] != "idle" {
                    goal_seen = true;
                    client.cancel_goal(0)?;
                } else if goal_seen && value["state"] == "idle" && !cancel_seen {
                    cancel_seen = true;
                    client.publish_json(
                        "terra/rover/0/safety",
                        &serde_json::json!({"action":"stop","token":"browser-check-stop"}),
                    )?;
                }
            }
            if sample.topic == "terra/rover/0/autonomy/status" && cancel_seen {
                safety_seen |= value["safety"] == "emergency_stop";
            }
        }
        let states = [
            "autonomy/status",
            "mission/status",
            "search/status",
            "experiment/status",
            "hardware/status",
            "goal/proposal",
            "map/occupancy",
        ];
        if moved
            && fleet_seen
            && poses.contains_key(&1)
            && goal_seen
            && cancel_seen
            && safety_seen
            && recording_seen
            && cameras.contains("RGBA8_SRGB")
            && cameras.contains("32FC1_LE")
            && states
                .iter()
                .all(|suffix| topics.contains(&format!("terra/rover/0/{suffix}")))
        {
            client.set_velocity(0, 0.0, 0.0)?;
            println!(
                "PASS: pose, drive, fleet, goals, safety, autonomy, mission, search, recording, occupancy, RGB, and exposure-aligned depth"
            );
            return Ok(());
        }
    }
    Err(format!("browser check timed out: moved={moved} fleet={fleet_seen} goal={goal_seen} cancel={cancel_seen} safety={safety_seen} recording={recording_seen} cameras={cameras:?} topics={topics:?}").into())
}
