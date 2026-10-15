//! Native file recorder or a bounded, host-acknowledged browser recording sink.
use super::*;

pub(super) enum Recorder {
    #[cfg(feature = "native")]
    Native(terra_experiment::RunRecorder),
    #[cfg(all(target_arch = "wasm32", feature = "browser-bridge"))]
    Browser(Arc<Shared>),
}

impl Recorder {
    #[cfg(feature = "native")]
    pub fn native(manifest: serde_json::Value) -> Option<Self> {
        let root = std::env::var("TERRA_RUN_DIR").unwrap_or_else(|_| "runs".into());
        std::fs::create_dir_all(&root).ok()?;
        let run_id = manifest["run_id"].as_str()?;
        terra_experiment::RunRecorder::create(
            &std::path::Path::new(&root).join(format!("{run_id}.jsonl")),
            manifest,
            8192,
        )
        .ok()
        .map(Self::Native)
    }

    #[cfg(all(target_arch = "wasm32", feature = "browser-bridge"))]
    pub fn browser(shared: Arc<Shared>, run_id: String, manifest: serde_json::Value) -> Self {
        *shared.recording.manifest.lock().unwrap() = Some((run_id, manifest));
        Self::Browser(shared)
    }

    pub fn failed(&self) -> bool {
        match self {
            #[cfg(feature = "native")]
            Self::Native(recorder) => recorder.failed(),
            #[cfg(all(target_arch = "wasm32", feature = "browser-bridge"))]
            Self::Browser(shared) => {
                !shared.recording.healthy.load(Ordering::Acquire)
                    || shared.recording.failed.load(Ordering::Acquire)
            }
        }
    }

    pub fn record(&self, event: serde_json::Value) -> std::io::Result<()> {
        match self {
            #[cfg(feature = "native")]
            Self::Native(recorder) => recorder.record(event),
            #[cfg(all(target_arch = "wasm32", feature = "browser-bridge"))]
            Self::Browser(shared) => {
                if !shared.connected.load(Ordering::Acquire)
                    || shared.recording.failed.load(Ordering::Acquire)
                {
                    return Err(std::io::Error::other("browser recorder unavailable"));
                }
                let mut queue = shared.recording.events.lock().unwrap();
                if queue.len() >= 8192 {
                    shared.recording.failed.store(true, Ordering::Release);
                    return Err(std::io::Error::other("browser recorder queue full"));
                }
                queue.push_back(event);
                Ok(())
            }
        }
    }
}

#[cfg(all(target_arch = "wasm32", feature = "browser-bridge"))]
#[derive(Default)]
pub(super) struct BrowserRecording {
    pub manifest: Mutex<Option<(String, serde_json::Value)>>,
    pub events: Mutex<std::collections::VecDeque<serde_json::Value>>,
    pub healthy: AtomicBool,
    pub failed: AtomicBool,
}
