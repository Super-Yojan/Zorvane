//! Bucket and ball detection with Terra's YOLO model, shared by the whole fleet.
//!
//! One worker thread owns the only copy of the model. Each pass takes the newest
//! front RGB frame of every rover, runs them through that model one after another
//! (tract runs a batch slower per image than single frames), and returns the boxes.
//! Boxes are placed with the colocated depth camera and go into each rover's Terra
//! object map, which the bridge publishes on `objects` like TerraPhone does.
//! `ZORVANE_OBJECTS=0` turns detection off.
use crate::{
    depth_camera::{DepthCamera, DepthFrame},
    rgb_camera::RgbFrame,
    terra::RoverId,
    velocity_controller::VelocityControlState,
};
use bevy::prelude::*;
use std::{
    collections::BTreeMap,
    sync::{
        Mutex,
        mpsc::{Receiver, Sender, TryRecvError, channel},
    },
    time::Instant,
};
use terra_autonomy::objects::ObjectSighting;
use terra_mapping::{CameraIntrinsics, CameraPose};
use terra_types::Vector3;
use tract_onnx::prelude::*;

/// Terra `models/yolo26n-rover`, exported to ONNX by `tools/export_rover_detector.py`.
const MODEL: &[u8] = include_bytes!("../assets/models/rover_detector.onnx");
/// Longest input side, as trained.
const INPUT_SIZE: usize = 640;
/// Ultralytics letterbox grey.
const PAD: f32 = 114.0 / 255.0;
/// Boxes below this never reach the object map (Terra's own gate is the same).
const MIN_CONFIDENCE: f32 = terra_autonomy::objects::MIN_CONFIDENCE as f32;
/// At most one pass every this many seconds, like TerraPhone's 5 Hz detector.
const PASS_PERIOD: f64 = 0.2;
/// RGB and depth readbacks finish separately; pair them only when this close.
const MAX_PAIR_GAP: f64 = 0.15;
/// Worker threads for tract's matrix kernels; the renderer keeps the rest.
const INFERENCE_THREADS: usize = 2;

pub struct TerraObjectDetectionPlugin;
impl Plugin for TerraObjectDetectionPlugin {
    fn build(&self, app: &mut App) {
        if std::env::var("ZORVANE_OBJECTS").as_deref() == Ok("0") {
            return;
        }
        app.insert_resource(SharedDetector::spawn())
            .add_systems(Update, (apply_detections, submit_frames).chain());
    }
}

/// One rover's frame, ready for the model.
struct Job {
    rover: u64,
    frame_id: u64,
    timestamp: f64,
    width: u32,
    height: u32,
    rgba: Vec<u8>,
    depth: Vec<f32>,
    depth_width: u32,
    depth_height: u32,
    vertical_fov: f32,
    near: f32,
    far: f32,
    camera: CameraPose,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Detection {
    pub class: &'static str,
    pub color: Option<&'static str>,
    pub confidence: f32,
    /// Box corners in source image pixels, top-left origin.
    pub x1: f32,
    pub y1: f32,
    pub x2: f32,
    pub y2: f32,
}

struct Done {
    job: Job,
    detections: Vec<Detection>,
}

struct Pass {
    results: Vec<Done>,
    milliseconds: f64,
}

#[derive(Resource)]
struct SharedDetector {
    jobs: Sender<Vec<Job>>,
    results: Mutex<Receiver<Result<Pass, String>>>,
    busy: bool,
    last_pass: f64,
    submitted: BTreeMap<u64, u64>,
    failed: bool,
    /// Frames and inference time since the last log line.
    stats: (u32, f64, Instant),
}

impl SharedDetector {
    fn spawn() -> Self {
        let (jobs, inbox) = channel::<Vec<Job>>();
        let (outbox, results) = channel();
        std::thread::Builder::new()
            .name("zorvane-yolo".into())
            .spawn(move || worker(inbox, outbox))
            .expect("start object detector thread");
        Self {
            jobs,
            results: Mutex::new(results),
            busy: false,
            last_pass: f64::NEG_INFINITY,
            submitted: BTreeMap::new(),
            failed: false,
            stats: (0, 0.0, Instant::now()),
        }
    }
}

type Plan = std::sync::Arc<TypedRunnableModel>;

fn worker(inbox: Receiver<Vec<Job>>, outbox: Sender<Result<Pass, String>>) {
    tract_linalg::multithread::set_default_executor(
        tract_linalg::multithread::Executor::multithread_with_name(
            INFERENCE_THREADS,
            "zorvane-yolo-mm",
        ),
    );
    // One optimized plan per input size; the fleet's cameras normally share one.
    let mut plans: BTreeMap<(usize, usize), Plan> = BTreeMap::new();
    while let Ok(jobs) = inbox.recv() {
        let started = Instant::now();
        let mut results = Vec::with_capacity(jobs.len());
        let mut error = None;
        for job in jobs {
            let (input_w, input_h) = input_size(job.width, job.height);
            let plan = match plans.get(&(input_w, input_h)) {
                Some(plan) => plan,
                None => match load(input_w, input_h) {
                    Ok(plan) => plans.entry((input_w, input_h)).or_insert(plan),
                    Err(e) => {
                        error = Some(format!("load rover detector: {e}"));
                        break;
                    }
                },
            };
            let letterbox = Letterbox::new(job.width, job.height, input_w, input_h);
            let input = letterbox.tensor(&job.rgba);
            match plan.run(tvec!(input.into())) {
                Ok(out) => {
                    let detections = out[0]
                        .to_plain_array_view::<f32>()
                        .map(|rows| {
                            decode(rows.as_slice().unwrap_or(&[]), &letterbox, MIN_CONFIDENCE)
                        })
                        .unwrap_or_default();
                    results.push(Done { job, detections });
                }
                Err(e) => {
                    error = Some(format!("run rover detector: {e}"));
                    break;
                }
            }
        }
        let reply = match error {
            Some(e) => Err(e),
            None => Ok(Pass {
                results,
                milliseconds: started.elapsed().as_secs_f64() * 1e3,
            }),
        };
        if outbox.send(reply).is_err() {
            return;
        }
    }
}

fn load(width: usize, height: usize) -> TractResult<Plan> {
    tract_onnx::onnx()
        .model_for_read(&mut std::io::Cursor::new(MODEL))?
        .with_input_fact(0, f32::fact([1, 3, height, width]).into())?
        .into_optimized()?
        .into_runnable()
}

/// Model input for a source frame: 640 on the long side, the short side rounded up to
/// a multiple of 32 (Ultralytics' rectangular inference), so 4:3 runs at 640×480.
pub(crate) fn input_size(width: u32, height: u32) -> (usize, usize) {
    let (w, h) = (width.max(1) as f64, height.max(1) as f64);
    let scale = INPUT_SIZE as f64 / w.max(h);
    let round = |v: f64| (((v * scale).round() as usize).div_ceil(32) * 32).clamp(32, INPUT_SIZE);
    (round(w), round(h))
}

/// Centred, aspect-preserving resize into the model input with grey padding.
pub(crate) struct Letterbox {
    source: (u32, u32),
    input: (usize, usize),
    scale: f32,
    pad: (f32, f32),
}

impl Letterbox {
    pub(crate) fn new(width: u32, height: u32, input_w: usize, input_h: usize) -> Self {
        let scale = (input_w as f32 / width as f32).min(input_h as f32 / height as f32);
        let pad = (
            ((input_w as f32 - width as f32 * scale) / 2.0).floor(),
            ((input_h as f32 - height as f32 * scale) / 2.0).floor(),
        );
        Self {
            source: (width, height),
            input: (input_w, input_h),
            scale,
            pad,
        }
    }
    /// NCHW RGB in 0..1, bilinear from the sRGB RGBA frame.
    fn tensor(&self, rgba: &[u8]) -> Tensor {
        let (iw, ih) = self.input;
        let (sw, sh) = (self.source.0 as usize, self.source.1 as usize);
        let mut data = vec![PAD; 3 * iw * ih];
        let plane = iw * ih;
        for y in 0..ih {
            let sy = (y as f32 + 0.5 - self.pad.1) / self.scale - 0.5;
            if sy < -0.5 || sy > sh as f32 - 0.5 {
                continue;
            }
            let y0 = (sy.floor().max(0.0) as usize).min(sh - 1);
            let y1 = (y0 + 1).min(sh - 1);
            let fy = (sy - y0 as f32).clamp(0.0, 1.0);
            for x in 0..iw {
                let sx = (x as f32 + 0.5 - self.pad.0) / self.scale - 0.5;
                if sx < -0.5 || sx > sw as f32 - 0.5 {
                    continue;
                }
                let x0 = (sx.floor().max(0.0) as usize).min(sw - 1);
                let x1 = (x0 + 1).min(sw - 1);
                let fx = (sx - x0 as f32).clamp(0.0, 1.0);
                for c in 0..3 {
                    let at = |px: usize, py: usize| rgba[(py * sw + px) * 4 + c] as f32;
                    let top = at(x0, y0) * (1.0 - fx) + at(x1, y0) * fx;
                    let bottom = at(x0, y1) * (1.0 - fx) + at(x1, y1) * fx;
                    data[c * plane + y * iw + x] = (top * (1.0 - fy) + bottom * fy) / 255.0;
                }
            }
        }
        tract_ndarray::Array4::from_shape_vec((1, 3, ih, iw), data)
            .expect("letterbox shape")
            .into_tensor()
    }
}

/// YOLO26 end-to-end rows `[x1, y1, x2, y2, score, class]` in input pixels, back to
/// source pixels. Classes as trained: 0 ball, 1 blue bucket, 2 orange bucket.
pub(crate) fn decode(rows: &[f32], letterbox: &Letterbox, threshold: f32) -> Vec<Detection> {
    let (w, h) = (letterbox.source.0 as f32, letterbox.source.1 as f32);
    rows.chunks_exact(6)
        .filter(|r| r[4] >= threshold && r[4] <= 1.0)
        .filter_map(|r| {
            let (class, color) = match r[5].round() as i32 {
                0 => ("ball", None),
                1 => ("bucket", Some("blue")),
                2 => ("bucket", Some("orange")),
                _ => return None,
            };
            let unmap =
                |v: f32, pad: f32, limit: f32| ((v - pad) / letterbox.scale).clamp(0.0, limit);
            let d = Detection {
                class,
                color,
                confidence: r[4],
                x1: unmap(r[0], letterbox.pad.0, w),
                y1: unmap(r[1], letterbox.pad.1, h),
                x2: unmap(r[2], letterbox.pad.0, w),
                y2: unmap(r[3], letterbox.pad.1, h),
            };
            (d.x2 - d.x1 >= 1.0 && d.y2 - d.y1 >= 1.0).then_some(d)
        })
        .collect()
}

/// Median axial depth over the middle half of the box, in metres.
pub(crate) fn box_depth(
    detection: &Detection,
    depth: &[f32],
    (width, height): (u32, u32),
    (source_w, source_h): (u32, u32),
    (near, far): (f32, f32),
) -> Option<f32> {
    if width == 0 || height == 0 || depth.len() != (width * height) as usize {
        return None;
    }
    let sx = width as f32 / source_w as f32;
    let sy = height as f32 / source_h as f32;
    let (cx, cy) = (
        (detection.x1 + detection.x2) / 2.0,
        (detection.y1 + detection.y2) / 2.0,
    );
    let (hw, hh) = (
        (detection.x2 - detection.x1) / 4.0,
        (detection.y2 - detection.y1) / 4.0,
    );
    let col = |x: f32| ((x * sx).floor().max(0.0) as u32).min(width - 1);
    let row = |y: f32| ((y * sy).floor().max(0.0) as u32).min(height - 1);
    let mut values: Vec<f32> = (row(cy - hh)..=row(cy + hh))
        .flat_map(|r| (col(cx - hw)..=col(cx + hw)).map(move |c| (r, c)))
        .map(|(r, c)| depth[(r * width + c) as usize])
        .filter(|d| d.is_finite() && *d > near && *d < far)
        .collect();
    if values.is_empty() {
        return None;
    }
    values.sort_by(f32::total_cmp);
    Some(values[values.len() / 2])
}

/// World point of the box centre at `depth` (axial), through the depth camera model.
pub(crate) fn locate(
    detection: &Detection,
    depth: f32,
    intrinsics: &CameraIntrinsics,
    (source_w, source_h): (u32, u32),
    camera: &CameraPose,
) -> Vector3 {
    let u = (detection.x1 + detection.x2) as f64 / 2.0 * intrinsics.width as f64 / source_w as f64;
    let v = (detection.y1 + detection.y2) as f64 / 2.0 * intrinsics.height as f64 / source_h as f64;
    let z = depth as f64;
    let optical = Vector3 {
        x: (u - 0.5 - intrinsics.cx) * z / intrinsics.fx,
        y: (v - 0.5 - intrinsics.cy) * z / intrinsics.fy,
        z,
    };
    let ray = camera.orientation.rotate(optical);
    Vector3 {
        x: camera.position.x + ray.x,
        y: camera.position.y + ray.y,
        z: camera.position.z + ray.z,
    }
}

/// Same pinhole model as the occupancy map's depth integration.
fn intrinsics(width: u32, height: u32, vertical_fov: f32) -> CameraIntrinsics {
    let f = height as f64 / (2.0 * (vertical_fov as f64 / 2.0).tan());
    CameraIntrinsics {
        width,
        height,
        fx: f,
        fy: f,
        cx: width as f64 / 2.0 - 0.5,
        cy: height as f64 / 2.0 - 0.5,
    }
}

/// Start a pass over every rover's newest RGB frame once the last pass is back.
fn submit_frames(
    time: Res<Time>,
    mut detector: ResMut<SharedDetector>,
    rgb: Query<(&RoverId, &RgbFrame)>,
    depth: Query<(&RoverId, &DepthCamera, &DepthFrame)>,
) {
    let now = time.elapsed_secs_f64();
    if detector.busy || detector.failed || now - detector.last_pass < PASS_PERIOD {
        return;
    }
    let depth: BTreeMap<u64, (&DepthCamera, &DepthFrame)> = depth
        .iter()
        .map(|(id, cam, frame)| (id.0, (cam, frame)))
        .collect();
    let mut jobs = Vec::new();
    for (id, frame) in &rgb {
        if frame.rgba.len() != (frame.width * frame.height * 4) as usize
            || detector
                .submitted
                .get(&id.0)
                .is_some_and(|s| *s >= frame.sequence)
        {
            continue;
        }
        let Some((camera, depth)) = depth.get(&id.0) else {
            continue;
        };
        let Some(exposure) = depth.exposure else {
            continue;
        };
        if (frame.received_at - depth.received_at).abs() > MAX_PAIR_GAP {
            continue;
        }
        jobs.push(Job {
            rover: id.0,
            frame_id: frame.sequence,
            timestamp: exposure.timestamp,
            width: frame.width,
            height: frame.height,
            rgba: frame.rgba.clone(),
            depth: depth.depth_metres.clone(),
            depth_width: depth.width,
            depth_height: depth.height,
            vertical_fov: camera.config.vertical_fov,
            near: camera.config.near,
            far: camera.config.far,
            camera: crate::occupancy_map::camera_pose(&exposure.camera_transform),
        });
    }
    if jobs.is_empty() {
        return;
    }
    for job in &jobs {
        detector.submitted.insert(job.rover, job.frame_id);
    }
    detector.last_pass = now;
    detector.busy = detector.jobs.send(jobs).is_ok();
}

/// Place the returned boxes and add them to each rover's object map.
fn apply_detections(
    time: Res<Time>,
    mut detector: ResMut<SharedDetector>,
    mut rovers: Query<(&RoverId, &mut VelocityControlState)>,
) {
    let received = detector.results.lock().unwrap().try_recv();
    let reply = match received {
        Ok(reply) => reply,
        Err(TryRecvError::Empty) => return,
        Err(TryRecvError::Disconnected) => {
            detector.busy = false;
            detector.failed = true;
            return;
        }
    };
    detector.busy = false;
    let pass = match reply {
        Ok(pass) => pass,
        Err(e) => {
            error!("{e}; object detection is off for this session");
            detector.failed = true;
            return;
        }
    };
    let now = time.elapsed_secs_f64();
    let mut controls: BTreeMap<u64, Mut<VelocityControlState>> =
        rovers.iter_mut().map(|(id, c)| (id.0, c)).collect();
    let mut sightings = Vec::new();
    detector.stats.0 += pass.results.len() as u32;
    for Done { job, detections } in pass.results {
        let k = intrinsics(job.depth_width, job.depth_height, job.vertical_fov);
        for d in &detections {
            let distance = box_depth(
                d,
                &job.depth,
                (job.depth_width, job.depth_height),
                (job.width, job.height),
                (job.near, job.far),
            );
            let point = distance.map(|z| locate(d, z, &k, (job.width, job.height), &job.camera));
            sightings.push(serde_json::json!({
                "rover_id": job.rover, "frame": job.frame_id, "class": d.class, "color": d.color,
                "confidence": d.confidence, "box": [d.x1, d.y1, d.x2, d.y2], "depth_m": distance,
                "x": point.map(|p| p.x), "y": point.map(|p| p.y),
            }));
            let (Some(point), Some(control)) = (point, controls.get_mut(&job.rover)) else {
                continue;
            };
            let _ = control.runtime.observe_object(
                ObjectSighting {
                    class: d.class.into(),
                    color: d.color.map(Into::into),
                    confidence: d.confidence as f64,
                    frame_id: job.frame_id,
                    timestamp: job.timestamp,
                    x: point.x,
                    y: point.y,
                    geo: None,
                },
                now,
            );
        }
    }
    detector.stats.1 += pass.milliseconds;
    if detector.stats.2.elapsed().as_secs() >= 30 {
        if detector.stats.0 > 0 {
            info!(
                "object detection: {} rover frames in 30 s, {:.0} ms per frame",
                detector.stats.0,
                detector.stats.1 / detector.stats.0 as f64
            );
        }
        detector.stats = (0, 0.0, Instant::now());
    }
    crate::observability::record_tick(serde_json::json!({
        "kind": "object_detection", "time": now, "inference_ms": pass.milliseconds,
        "sightings": sightings,
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use terra_types::Quaternion;

    #[test]
    fn four_by_three_frames_run_at_640_by_480() {
        assert_eq!(input_size(256, 192), (640, 480));
        assert_eq!(input_size(1024, 768), (640, 480));
        assert_eq!(input_size(640, 640), (640, 640));
        assert_eq!(input_size(192, 256), (480, 640));
    }

    #[test]
    fn boxes_map_back_through_the_letterbox() {
        // 256×192 into 640×640: scale 2.5, 80 px of grey above and below.
        let lb = Letterbox::new(256, 192, 640, 640);
        let rows = [
            25.0, 105.0, 125.0, 330.0, 0.9, 2.0, // orange bucket
            0.0, 80.0, 50.0, 130.0, 0.3, 1.0, // below threshold
            10.0, 90.0, 20.0, 100.0, 0.8, 7.0, // unknown class
        ];
        let found = decode(&rows, &lb, 0.5);
        assert_eq!(
            found,
            [Detection {
                class: "bucket",
                color: Some("orange"),
                confidence: 0.9,
                x1: 10.0,
                y1: 10.0,
                x2: 50.0,
                y2: 100.0
            }]
        );
    }

    #[test]
    fn letterbox_keeps_the_image_centred_with_grey_bars() {
        let lb = Letterbox::new(4, 2, 8, 8);
        let rgba: Vec<u8> = (0..8).flat_map(|_| [255, 0, 0, 255]).collect();
        let t = lb.tensor(&rgba);
        let v = t.to_plain_array_view::<f32>().unwrap();
        assert_eq!(v[[0, 0, 0, 0]], PAD);
        assert_eq!(v[[0, 0, 4, 4]], 1.0);
        assert_eq!(v[[0, 1, 4, 4]], 0.0);
        assert_eq!(v[[0, 2, 7, 7]], PAD);
    }

    #[test]
    fn box_centre_lands_at_its_depth_straight_ahead() {
        let d = Detection {
            class: "bucket",
            color: Some("blue"),
            confidence: 0.9,
            x1: 118.0,
            y1: 86.0,
            x2: 138.0,
            y2: 106.0,
        };
        let mut depth = vec![f32::NAN; 256 * 192];
        for r in 90..102 {
            for c in 122..134 {
                depth[r * 256 + c] = 4.0;
            }
        }
        let z = box_depth(&d, &depth, (256, 192), (256, 192), (0.05, 30.0)).unwrap();
        assert_eq!(z, 4.0);
        // Optical Z forward → world X, optical X right → world −Y, Y down → world −Z.
        let camera = CameraPose {
            position: Vector3 {
                x: 1.0,
                y: 2.0,
                z: 0.2,
            },
            orientation: Quaternion {
                x: 0.5,
                y: -0.5,
                z: 0.5,
                w: -0.5,
            },
        };
        let p = locate(
            &d,
            z,
            &intrinsics(256, 192, 60f32.to_radians()),
            (256, 192),
            &camera,
        );
        assert!(
            (p.x - 5.0).abs() < 1e-6 && (p.y - 2.0).abs() < 1e-6,
            "{p:?}"
        );
        assert!((p.z - 0.2).abs() < 1e-6, "{p:?}");
    }

    #[test]
    fn shared_model_loads_and_finds_nothing_in_a_grey_frame() {
        let plan = load(640, 480).unwrap();
        let lb = Letterbox::new(256, 192, 640, 480);
        let out = plan
            .run(tvec!(lb.tensor(&vec![128; 256 * 192 * 4]).into()))
            .unwrap();
        let rows = out[0].to_plain_array_view::<f32>().unwrap();
        assert_eq!(rows.shape(), &[1, 300, 6]);
        assert!(decode(rows.as_slice().unwrap(), &lb, MIN_CONFIDENCE).is_empty());
    }
}
