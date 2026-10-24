//! Run recorder: one JSONL file per run under `TERRA_RUN_DIR` (default `runs`).
use super::*;

pub(super) enum Recorder {
    Native(terra_experiment::RunRecorder),
}

impl Recorder {
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


    pub fn failed(&self) -> bool {
        match self {
            Self::Native(recorder) => recorder.failed(),
        }
    }

    pub fn record(&self, event: serde_json::Value) -> std::io::Result<()> {
        match self {
            Self::Native(recorder) => recorder.record(event),
        }
    }
}

