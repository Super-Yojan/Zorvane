#[path = "../simulator/src/observability.rs"]
mod observability;
#[test]
fn transition_is_kept_between_periodic_samples() {
    let mut p = observability::Projector::default();
    let first =
        serde_json::json!({"rover_id":4,"run_id":"run","time":1.0,"status":{"reason":"active"}});
    assert!(!p.project(first.clone()).is_empty());
    let mut e = first.clone();
    e["time"] = serde_json::json!(1.01);
    assert!(p.project(e.clone()).is_empty());
    e["status"]["reason"] = serde_json::json!("map_stale");
    let rows = p.project(e);
    assert!(rows.iter().any(|x| x.kind == "autonomy.transition"));
    assert_eq!(rows[0].session_id.as_deref(), Some("run"));
}
