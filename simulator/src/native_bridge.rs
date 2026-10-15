//! Blocking native Zenoh transport; simulation logic lives in bridge.rs.
use super::*;

pub(super) fn run_transport(config: &BridgeConfig, shared: &Arc<Shared>) -> zenoh::Result<()> {
    let session = zenoh::open(config.session_config()?).wait()?;
    let incoming = shared.clone();
    let prefix = config.prefix.clone();
    let _subscriber = session
        .declare_subscriber(format!("{prefix}/**"))
        .callback(move |sample| {
            incoming.receive(
                &prefix,
                sample.key_expr().as_str(),
                sample.payload().to_bytes().as_ref(),
            );
        })
        .wait()?;
    shared.connected.store(true, Ordering::Release);
    info!(
        "Terra Zenoh ready: {}/<id>/{{cmd_vel,goal,pose}}, {}/<id>/goal/status, {}/<id>/camera/{{rgb,depth}} (depth includes exposure pose), {}/fleet/{{size,state}}",
        config.prefix, config.prefix, config.prefix, config.prefix
    );
    let mut occupancy_baselines = BTreeMap::<u64, OccupancyBaseline>::new();
    while !shared.stop.load(Ordering::Acquire) {
        let outgoing = std::mem::take(&mut *shared.outgoing.lock().unwrap());
        for ((id, kind), payload) in outgoing.frames {
            if !shared.commands.lock().unwrap().contains_key(&id) {
                continue;
            }
            session
                .put(format!("{}/{id}/camera/{kind}", config.prefix), payload)
                .encoding(zenoh::bytes::Encoding::APPLICATION_OCTET_STREAM)
                .congestion_control(zenoh::qos::CongestionControl::Drop)
                .wait()?;
        }
        if let Some(state) = outgoing.fleet {
            session
                .put(format!("{}/fleet/state", config.prefix), state)
                .encoding(zenoh::bytes::Encoding::APPLICATION_JSON)
                .congestion_control(zenoh::qos::CongestionControl::Drop)
                .wait()?;
        }
        let live: std::collections::BTreeSet<u64> =
            shared.commands.lock().unwrap().keys().copied().collect();
        occupancy_baselines.retain(|id, _| live.contains(id));
        for ((id, kind), payload) in outgoing.states {
            if live.contains(&id) {
                session
                    .put(format!("{}/{id}/{kind}", config.prefix), payload)
                    .encoding(zenoh::bytes::Encoding::APPLICATION_JSON)
                    .wait()?;
            }
        }
        for (id, publication) in outgoing.occupancy {
            if live.contains(&id) {
                session
                    .put(
                        format!("{}/{id}/map/occupancy", config.prefix),
                        publication.payload,
                    )
                    .encoding(zenoh::bytes::Encoding::APPLICATION_JSON)
                    .wait()?;
                // Commit only after the paired full payload was published.
                occupancy_baselines.insert(id, publication.baseline);
            }
        }
        for id in outgoing.occupancy_heartbeats {
            if live.contains(&id)
                && let Some(baseline) = occupancy_baselines.get(&id)
            {
                session
                    .put(
                        format!("{}/{id}/map/occupancy/heartbeat", config.prefix),
                        serde_json::to_vec(baseline).unwrap(),
                    )
                    .encoding(zenoh::bytes::Encoding::APPLICATION_JSON)
                    .wait()?;
            }
        }
        for (id, reports) in outgoing.reports {
            if live.contains(&id) {
                for report in reports {
                    session
                        .put(format!("{}/{id}/search/report", config.prefix), report)
                        .encoding(zenoh::bytes::Encoding::APPLICATION_JSON)
                        .wait()?;
                }
            }
        }
        for (id, payload) in outgoing.goals {
            if !live.contains(&id) {
                continue;
            }
            session
                .put(format!("{}/{id}/goal/status", config.prefix), payload)
                .encoding(zenoh::bytes::Encoding::APPLICATION_JSON)
                .congestion_control(zenoh::qos::CongestionControl::Drop)
                .wait()?;
        }
        for (id, payload) in outgoing.poses {
            if !live.contains(&id) {
                continue;
            }
            session
                .put(format!("{}/{id}/pose", config.prefix), payload)
                .encoding(zenoh::bytes::Encoding::APPLICATION_JSON)
                .congestion_control(zenoh::qos::CongestionControl::Drop)
                .wait()?;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    shared.connected.store(false, Ordering::Release);
    session.close().wait()
}
