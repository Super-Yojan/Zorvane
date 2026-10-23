//! Read only the Maps key; never import fleet credentials or print secrets.
use std::path::{Path, PathBuf};

fn parsed(text: &str) -> Option<String> {
    text.lines().filter_map(|line| {
        let line = line.trim().strip_prefix("export ").unwrap_or(line.trim());
        let (name, value) = line.split_once('=')?;
        if name.trim() != "GOOGLE_MAPS_API_KEY" { return None; }
        let value = value.trim();
        let value = if let Some(quote) = value.chars().next().filter(|c| *c == '\'' || *c == '"') {
            let rest = &value[1..];
            &rest[..rest.find(quote)?]
        } else { value.split(" #").next().unwrap_or("").trim() };
        Some(value.to_owned())
    }).last()
}

fn resolve(environment: Option<String>, paths: &[PathBuf]) -> Option<String> {
    let value = environment.or_else(|| paths.iter().find_map(|path| {
        std::fs::read_to_string(path).ok().and_then(|text| parsed(&text))
    }))?;
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

pub fn load() -> Option<String> {
    // Local-Terra builds use a temporary symlinked workspace removed after build.
    // Resolve the real checkout from the working directory or executable as well.
    let cwd = std::env::current_dir().ok();
    let compiled = Path::new(env!("CARGO_MANIFEST_DIR")).canonicalize().ok();
    let executable = std::env::current_exe().ok();
    let root = cwd.as_deref().into_iter()
        .chain(compiled.as_deref())
        .chain(executable.as_deref())
        .flat_map(Path::ancestors)
        .find(|path| path.join("simulator/Cargo.toml").is_file());
    let mut paths = Vec::new();
    if let Some(cwd) = &cwd { paths.push(cwd.join(".env")); }
    if let Some(root) = root {
        paths.push(root.join(".env"));
        if let Some(parent) = root.parent() { paths.push(parent.join("ARGOS/.env")); }
    }
    resolve(std::env::var("GOOGLE_MAPS_API_KEY").ok(), &paths)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reads_only_maps_key_and_handles_quotes() {
        assert_eq!(parsed("PASSWORD=ignore\nexport GOOGLE_MAPS_API_KEY='test-key' # comment"), Some("test-key".into()));
        assert_eq!(parsed("GOOGLE_MAPS_API_KEY=first\nGOOGLE_MAPS_API_KEY=second # comment"), Some("second".into()));
        assert_eq!(parsed("OTHER_KEY=ignore"), None);
    }
    #[test]
    fn priority_and_explicit_empty_override() {
        let root = std::env::temp_dir().join(format!("zorvane-key-test-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let own = root.join("own"); let shared = root.join("shared");
        std::fs::write(&own, "GOOGLE_MAPS_API_KEY=own-test-key").unwrap();
        std::fs::write(&shared, "GOOGLE_MAPS_API_KEY=shared-test-key").unwrap();
        let paths = [own.clone(), shared.clone()];
        assert_eq!(resolve(Some("environment-test-key".into()), &paths), Some("environment-test-key".into()));
        assert_eq!(resolve(Some(String::new()), &paths), None);
        assert_eq!(resolve(None, &paths), Some("own-test-key".into()));
        std::fs::remove_file(own).unwrap();
        assert_eq!(resolve(None, &paths), Some("shared-test-key".into()));
        std::fs::remove_dir_all(root).unwrap();
    }
}
