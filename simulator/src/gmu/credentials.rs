//! Read an existing key from the user's authenticated CLI. Never log secrets or
//! raw CLI output, change the project, create keys, or modify API restrictions.
use std::{
    path::PathBuf,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

fn cloud(program: &PathBuf, project: &str, args: &[&str]) -> Result<String, String> {
    let mut child = Command::new(program)
        .args(["services", "api-keys"])
        .args(args)
        .arg(format!("--project={project}"))
        .arg("--quiet")
        .env("CLOUDSDK_CORE_DISABLE_PROMPTS", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "Google Cloud CLI is unavailable. See GMU setup instructions.".to_owned())?;
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(25)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("Google Cloud connection timed out. Sign in and retry.".into());
            }
        }
    }
    let output = child
        .wait_with_output()
        .map_err(|_| "Could not read the Maps connection.".to_owned())?;
    if !output.status.success() {
        return Err(
            "Maps connection failed. Check Google Cloud sign-in and key access, then retry.".into(),
        );
    }
    String::from_utf8(output.stdout).map_err(|_| "Could not read the Maps connection.".to_owned())
}

fn choose_key(metadata: &str) -> Result<String, String> {
    let keys: Vec<serde_json::Value> = serde_json::from_str(metadata)
        .map_err(|_| "Could not read Maps key settings.".to_owned())?;
    let candidates: Vec<_> = keys
        .iter()
        .filter(|key| {
            key.pointer("/restrictions/apiTargets")
                .and_then(|v| v.as_array())
                .is_some_and(|targets| {
                    targets
                        .iter()
                        .any(|t| t["service"] == "tile.googleapis.com")
                })
        })
        .filter_map(|key| key["name"].as_str())
        .collect();
    if candidates.len() != 1 {
        return Err("Choose a Maps key in GMU setup, then retry the connection.".into());
    }
    candidates[0]
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| "Could not read Maps key settings.".into())
}

pub fn existing_key() -> Result<String, String> {
    let program = std::env::var_os("PATH")
        .and_then(|paths| {
            std::env::split_paths(&paths)
                .map(|dir| dir.join("gcloud"))
                .find(|file| file.is_file())
        })
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .map(|home| home.join("Startups/google-cloud-sdk/bin/gcloud"))
                .filter(|p| p.is_file())
        })
        .ok_or_else(|| "Sign in to Google Cloud or configure a Maps key, then retry.".to_owned())?;
    let project = std::env::var("GOOGLE_CLOUD_PROJECT").unwrap_or_else(|_| "glid-app".into());
    let key_id = match std::env::var("GOOGLE_MAPS_KEY_ID")
        .ok()
        .filter(|s| !s.trim().is_empty())
    {
        Some(id) => id,
        None => choose_key(&cloud(&program, &project, &["list", "--format=json"])?)?,
    };
    let key = cloud(
        &program,
        &project,
        &[
            "get-key-string",
            &key_id,
            "--location=global",
            "--format=value(keyString)",
        ],
    )?;
    let key = key.trim();
    if key.is_empty() {
        return Err("Google Cloud returned no Maps key. Check key access and retry.".into());
    }
    Ok(key.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_one_explicit_map_tiles_key_is_selected() {
        let key = r#"{"name":"projects/demo/locations/global/keys/tiles-key","restrictions":{"apiTargets":[{"service":"tile.googleapis.com"}]}}"#;
        assert_eq!(choose_key(&format!("[{key}]")).unwrap(), "tiles-key");
        assert!(choose_key(&format!("[{key},{key}]")).is_err());
        assert!(choose_key(r#"[{"name":"unrestricted-key"}]"#).is_err());
        assert!(
            choose_key("malformed metadata with a pretend secret")
                .unwrap_err()
                .find("pretend")
                .is_none()
        );
    }
}
