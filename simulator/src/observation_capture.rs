//! Exposure-aligned depth evidence and honestly labelled receipt-only RGB captures.
#[cfg(feature = "native")]
use std::sync::{OnceLock, RwLock};
#[cfg(feature = "native")]
use terra_observability::{AttachmentInput, EventInput, EvidenceBundle, EvidenceWriter};
#[cfg(feature = "native")]
static WRITER: OnceLock<Option<EvidenceWriter>> = OnceLock::new();
#[cfg(feature = "native")]
static SESSION: OnceLock<RwLock<String>> = OnceLock::new();
pub fn initialize() {
    #[cfg(feature = "native")]
    {
        WRITER.get_or_init(|| {
            let journal = terra_observability::runtime_journal()?;
            let directory = terra_observability::runtime_evidence_directory()?;
            EvidenceWriter::open(
                directory,
                journal,
                terra_observability::runtime_settings().attachment_max_bytes,
            )
            .ok()
        });
    }
}
pub fn set_session(run: &str) {
    #[cfg(feature = "native")]
    {
        if let Ok(mut s) = SESSION
            .get_or_init(|| RwLock::new(String::new()))
            .try_write()
        {
            *s = run.into();
        }
    }
    #[cfg(not(feature = "native"))]
    let _ = run;
}
#[cfg(feature = "native")]
fn session() -> Option<String> {
    SESSION
        .get()
        .and_then(|s| s.try_read().ok().map(|s| s.clone()))
}
#[cfg(feature = "native")]
pub fn capture_depth(
    rover: u64,
    frame: &crate::depth_camera::DepthFrame,
    k: &terra_mapping::CameraIntrinsics,
    map: &terra_runtime::mapping::MappingState,
    config: &terra_mapping::MapConfig,
) {
    use std::sync::Mutex;
    static LAST: OnceLock<Mutex<std::collections::BTreeMap<u64, f64>>> = OnceLock::new();
    let Some(exposure) = frame.exposure else {
        return;
    };
    capture_map(rover, exposure.timestamp, frame.sequence, map);
    let Ok(mut last) = LAST.get_or_init(Default::default).try_lock() else {
        return;
    };
    let at = last.entry(rover).or_insert(f64::NEG_INFINITY);
    if exposure.timestamp - *at < terra_observability::runtime_settings().evidence_sample_seconds {
        return;
    }
    *at = exposure.timestamp;
    let Some(Some(writer)) = WRITER.get() else {
        return;
    };
    let pose = crate::occupancy_map::camera_pose(&exposure.camera_transform);
    let body = crate::occupancy_map::body_pose(&exposure.body_transform);
    let grid = map.snapshot();
    let manifest = serde_json::json!({"exposure_ns":(exposure.timestamp*1e9) as u64,"frame_id":"zorvane-world","session_id":session(),"rgb_missing_reason":"simulator_rgb_exposure_not_available","depth_intrinsics":{"width":k.width,"height":k.height,"fx":k.fx,"fy":k.fy,"cx":k.cx,"cy":k.cy},"camera_to_world":{"position":[pose.position.x,pose.position.y,pose.position.z],"quaternion_xyzw":[pose.orientation.x,pose.orientation.y,pose.orientation.z,pose.orientation.w]},"body_pose":{"position":[body.0,body.1,exposure.body_transform.translation().y as f64],"yaw":body.2},"ground_plane":{"normal":[0,0,1],"offset":0},"ground_reference":"world_default","thresholds":{"min_obstacle_height":config.min_obstacle_height,"max_obstacle_height":config.max_obstacle_height,"max_range":config.max_range,"pixel_stride":config.pixel_stride},"map_revision":frame.sequence,"integration_result":"accepted","occupancy":{"width":grid.width,"height":grid.height,"origin_x":grid.origin_x,"origin_y":grid.origin_y,"resolution":grid.resolution}});
    let attachments = vec![
        AttachmentInput {
            name: "depth".into(),
            data: frame
                .depth_metres
                .iter()
                .flat_map(|v| v.to_le_bytes())
                .collect(),
            encoding: "f32le".into(),
            dimensions: vec![frame.width, frame.height],
            units: "metres".into(),
        },
        AttachmentInput {
            name: "occupancy".into(),
            data: grid.occupancy.iter().map(|v| *v as u8).collect(),
            encoding: "i8".into(),
            dimensions: vec![grid.width, grid.height],
            units: "occupancy_percent".into(),
        },
    ];
    let mut event = EventInput::new(
        &rover.to_string(),
        "mapping",
        "mapping.evidence",
        serde_json::json!({}),
    );
    event.session_id = session();
    event.monotonic_ns = (exposure.timestamp * 1e9) as u64;
    writer.try_capture(EvidenceBundle {
        event,
        manifest,
        attachments,
    });
}
#[cfg(not(feature = "native"))]
pub fn capture_depth(
    _: u64,
    _: &crate::depth_camera::DepthFrame,
    _: &terra_mapping::CameraIntrinsics,
    _: &terra_runtime::mapping::MappingState,
    _: &terra_mapping::MapConfig,
) {
}
#[cfg(feature = "native")]
pub fn capture_rgb(rover: u64, frame: &crate::rgb_camera::RgbFrame) {
    let Some(Some(writer)) = WRITER.get() else {
        return;
    };
    let mut event = EventInput::new(
        &rover.to_string(),
        "camera",
        "camera.receipt_only",
        serde_json::json!({}),
    );
    event.session_id = session();
    event.monotonic_ns = (frame.received_at * 1e9) as u64;
    let manifest = serde_json::json!({"exposure_ns":null,"rgb_exposure_ns":null,"receipt_ns":event.monotonic_ns,"time_basis":"receipt_only","frame_id":"zorvane-rgb-unpaired","sequence":frame.sequence,"image_orientation":"rendered top-left RGBA","alignment":"exposure unknown; not registered to depth","session_id":event.session_id});
    writer.try_capture(EvidenceBundle {
        event,
        manifest,
        attachments: vec![AttachmentInput {
            name: "rgb".into(),
            data: frame.rgba.clone(),
            encoding: "rgba8".into(),
            dimensions: vec![frame.width, frame.height],
            units: "rgba".into(),
        }],
    });
}
#[cfg(not(feature = "native"))]
pub fn capture_rgb(_: u64, _: &crate::rgb_camera::RgbFrame) {}

#[cfg(feature = "native")]
fn capture_map(rover: u64, time: f64, revision: u64, map: &terra_runtime::mapping::MappingState) {
    use std::sync::Mutex;
    static LAST_MAP: OnceLock<Mutex<std::collections::BTreeMap<u64, f64>>> = OnceLock::new();
    let Ok(mut last) = LAST_MAP.get_or_init(Default::default).try_lock() else {
        return;
    };
    let at = last.entry(rover).or_insert(f64::NEG_INFINITY);
    if time - *at < terra_observability::runtime_settings().map_sample_seconds {
        return;
    }
    *at = time;
    let Some(Some(writer)) = WRITER.get() else {
        return;
    };
    let grid = map.snapshot();
    let manifest = serde_json::json!({"exposure_ns":(time*1e9) as u64,"frame_id":"zorvane-world","evidence_type":"occupancy","map_revision":revision,"occupancy":{"width":grid.width,"height":grid.height,"origin_x":grid.origin_x,"origin_y":grid.origin_y,"resolution":grid.resolution}});
    let mut event = EventInput::new(
        &rover.to_string(),
        "mapping",
        "mapping.snapshot",
        serde_json::json!({}),
    );
    event.session_id = session();
    event.monotonic_ns = (time * 1e9) as u64;
    writer.try_capture(EvidenceBundle {
        event,
        manifest,
        attachments: vec![AttachmentInput {
            name: "occupancy".into(),
            data: grid.occupancy.iter().map(|x| *x as u8).collect(),
            encoding: "i8".into(),
            dimensions: vec![grid.width, grid.height],
            units: "occupancy_percent".into(),
        }],
    });
}
pub fn evidence_period() -> f64 {
    #[cfg(feature = "native")]
    {
        terra_observability::runtime_settings().evidence_sample_seconds
    }
    #[cfg(not(feature = "native"))]
    {
        1.0
    }
}
