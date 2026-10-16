//! Blocking native Zenoh transport; simulation logic lives in bridge.rs.
use super::*;

pub(super) fn run_transport(config: &BridgeConfig, shared: &Arc<Shared>) -> zenoh::Result<()> {
    let result = if let Some(path) = config.config_path() {
        run_configured_file(path, config, shared)
    } else {
        let (local_spec, fleet_spec) = config.link_specs().expect("validated Zenoh configuration");
        run_split_sessions(config, shared, local_spec, fleet_spec)
    };
    shared.connected.store(false, Ordering::Release);
    result
}

fn run_configured_file(
    path: &str,
    config: &BridgeConfig,
    shared: &Arc<Shared>,
) -> zenoh::Result<()> {
    let session = zenoh::open(zenoh::Config::from_file(path)?).wait()?;
    let subscriber = subscribe(&session, &config.prefix, shared)?;
    log_ready(config);
    let result = publish_loop(config, shared, Some(session.clone()), None);
    drop(subscriber);
    session.close().wait()?;
    result
}

fn run_split_sessions(
    config: &BridgeConfig,
    shared: &Arc<Shared>,
    local_spec: Option<ZenohLinkSpec>,
    fleet_spec: Option<ZenohLinkSpec>,
) -> zenoh::Result<()> {
    let local = match local_spec {
        Some(spec) => {
            let session = zenoh::open(spec.to_zenoh_config()?).wait()?;
            let subscriber = subscribe(&session, &config.prefix, shared)?;
            info!("Terra Zenoh listening on {}", spec.listen.join(", "));
            Some((session, subscriber))
        }
        None => None,
    };
    let fleet_state = Arc::new(FleetState::default());
    let fleet_thread = fleet_spec.map(|spec| {
        let state = fleet_state.clone();
        let shared = shared.clone();
        let prefix = config.prefix.clone();
        std::thread::Builder::new()
            .name("terra-zenoh-fleet".into())
            .spawn(move || connect_fleet(spec, prefix, shared, state))
            .expect("could not start Zenoh fleet thread")
    });
    let local_session = local.as_ref().map(|(session, _)| session.clone());
    let published = if local_session.is_none() {
        match wait_for_fleet(shared, &fleet_state) {
            Ok(()) => {
                log_ready(config);
                publish_loop(config, shared, None, Some(&fleet_state))
            }
            Err(error) => Err(error),
        }
    } else {
        log_ready(config);
        publish_loop(config, shared, local_session, Some(&fleet_state))
    };
    // Unblock the fleet thread before joining it. Bridge drop also sets this.
    shared.stop.store(true, Ordering::Release);
    let local_session = local.map(|(session, subscriber)| {
        drop(subscriber);
        session
    });
    if let Some(handle) = fleet_thread {
        let _ = handle.join();
    }
    let mut close_error = None;
    if let Some(session) = local_session
        && let Err(error) = session.close().wait()
    {
        close_error = Some(error);
    }
    if let Some(session) = fleet_state.session.lock().unwrap().take()
        && let Err(error) = session.close().wait()
    {
        close_error = Some(error);
    }
    match (published, close_error) {
        (Err(error), _) => Err(error),
        (Ok(()), Some(error)) => Err(error),
        (Ok(()), None) => Ok(()),
    }
}

fn log_ready(config: &BridgeConfig) {
    info!(
        "Terra Zenoh ready: {}/<id>/{{cmd_vel,goal,pose}}, {}/<id>/goal/status, {}/<id>/camera/{{rgb,depth}} (depth includes exposure pose), {}/fleet/{{size,state}}",
        config.prefix, config.prefix, config.prefix, config.prefix
    );
}

fn wait_for_fleet(shared: &Shared, fleet: &FleetState) -> zenoh::Result<()> {
    loop {
        if fleet.session.lock().unwrap().is_some() {
            return Ok(());
        }
        if let Some(error) = fleet.error.lock().unwrap().take() {
            return Err(error);
        }
        if shared.stop.load(Ordering::Acquire) {
            return Err(zenoh_error("Zenoh fleet connect stopped"));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn connect_fleet(spec: ZenohLinkSpec, prefix: String, shared: Arc<Shared>, state: Arc<FleetState>) {
    let endpoints = spec.connect.join(", ");
    let password = spec.password.clone();
    let as_user = spec
        .user
        .as_ref()
        .map(|user| format!(" as {user}"))
        .unwrap_or_else(|| " without a password".into());
    info!("Terra Zenoh fleet connecting to {endpoints}{as_user}");
    let opened = spec.to_zenoh_config().and_then(|config| {
        zenoh::open(config).wait().and_then(|session| {
            subscribe(&session, &prefix, &shared).map(|subscriber| (session, subscriber))
        })
    });
    match opened {
        Ok((session, subscriber)) => {
            info!("Terra Zenoh fleet connected to {endpoints}");
            *state.session.lock().unwrap() = Some(session);
            while !shared.stop.load(Ordering::Acquire) {
                std::thread::sleep(Duration::from_millis(50));
            }
            drop(subscriber);
        }
        Err(error) => {
            let message = redact(&error.to_string(), password.as_deref());
            warn!("Terra Zenoh fleet connect failed: {message}");
            *state.error.lock().unwrap() = Some(zenoh_error(message));
        }
    }
}

fn subscribe(
    session: &zenoh::Session,
    prefix: &str,
    shared: &Arc<Shared>,
) -> zenoh::Result<zenoh::pubsub::Subscriber<()>> {
    let incoming = shared.clone();
    let prefix = prefix.to_owned();
    session
        .declare_subscriber(format!("{prefix}/**"))
        .callback(move |sample| {
            incoming.receive(
                &prefix,
                sample.key_expr().as_str(),
                sample.payload().to_bytes().as_ref(),
            );
        })
        .wait()
}

fn publish_loop(
    config: &BridgeConfig,
    shared: &Arc<Shared>,
    local: Option<zenoh::Session>,
    fleet: Option<&FleetState>,
) -> zenoh::Result<()> {
    shared.connected.store(true, Ordering::Release);
    let mut occupancy_baselines = BTreeMap::<u64, OccupancyBaseline>::new();
    let mut noted_partial = false;
    while !shared.stop.load(Ordering::Acquire) {
        let sessions = active_sessions(local.as_ref(), fleet);
        if sessions.is_empty() {
            std::thread::sleep(Duration::from_millis(5));
            continue;
        }
        let outgoing = std::mem::take(&mut *shared.outgoing.lock().unwrap());
        for ((id, kind), payload) in outgoing.frames {
            if !shared.commands.lock().unwrap().contains_key(&id) {
                continue;
            }
            publish_each(
                &sessions,
                format!("{}/{id}/camera/{kind}", config.prefix),
                payload,
                zenoh::bytes::Encoding::APPLICATION_OCTET_STREAM,
                Some(zenoh::qos::CongestionControl::Drop),
                &mut noted_partial,
            )?;
        }
        if let Some(state) = outgoing.fleet {
            publish_each(
                &sessions,
                format!("{}/fleet/state", config.prefix),
                state,
                zenoh::bytes::Encoding::APPLICATION_JSON,
                Some(zenoh::qos::CongestionControl::Drop),
                &mut noted_partial,
            )?;
        }
        let live: std::collections::BTreeSet<u64> =
            shared.commands.lock().unwrap().keys().copied().collect();
        occupancy_baselines.retain(|id, _| live.contains(id));
        for ((id, kind), payload) in outgoing.states {
            if live.contains(&id) {
                publish_each(
                    &sessions,
                    format!("{}/{id}/{kind}", config.prefix),
                    payload,
                    zenoh::bytes::Encoding::APPLICATION_JSON,
                    None,
                    &mut noted_partial,
                )?;
            }
        }
        for (id, publication) in outgoing.occupancy {
            if live.contains(&id) {
                publish_each(
                    &sessions,
                    format!("{}/{id}/map/occupancy", config.prefix),
                    publication.payload,
                    zenoh::bytes::Encoding::APPLICATION_JSON,
                    None,
                    &mut noted_partial,
                )?;
                occupancy_baselines.insert(id, publication.baseline);
            }
        }
        for id in outgoing.occupancy_heartbeats {
            if live.contains(&id)
                && let Some(baseline) = occupancy_baselines.get(&id)
            {
                publish_each(
                    &sessions,
                    format!("{}/{id}/map/occupancy/heartbeat", config.prefix),
                    serde_json::to_vec(baseline).unwrap(),
                    zenoh::bytes::Encoding::APPLICATION_JSON,
                    None,
                    &mut noted_partial,
                )?;
            }
        }
        for (id, reports) in outgoing.reports {
            if live.contains(&id) {
                for report in reports {
                    publish_each(
                        &sessions,
                        format!("{}/{id}/search/report", config.prefix),
                        report,
                        zenoh::bytes::Encoding::APPLICATION_JSON,
                        None,
                        &mut noted_partial,
                    )?;
                }
            }
        }
        for (id, payload) in outgoing.goals {
            if !live.contains(&id) {
                continue;
            }
            publish_each(
                &sessions,
                format!("{}/{id}/goal/status", config.prefix),
                payload,
                zenoh::bytes::Encoding::APPLICATION_JSON,
                Some(zenoh::qos::CongestionControl::Drop),
                &mut noted_partial,
            )?;
        }
        for (id, payload) in outgoing.poses {
            if !live.contains(&id) {
                continue;
            }
            publish_each(
                &sessions,
                format!("{}/{id}/pose", config.prefix),
                payload,
                zenoh::bytes::Encoding::APPLICATION_JSON,
                Some(zenoh::qos::CongestionControl::Drop),
                &mut noted_partial,
            )?;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    Ok(())
}

fn active_sessions(
    local: Option<&zenoh::Session>,
    fleet: Option<&FleetState>,
) -> Vec<zenoh::Session> {
    let mut sessions = Vec::new();
    if let Some(session) = local {
        sessions.push(session.clone());
    }
    if let Some(fleet) = fleet
        && let Some(session) = fleet.session.lock().unwrap().clone()
    {
        sessions.push(session);
    }
    sessions
}

fn publish_each(
    sessions: &[zenoh::Session],
    key: String,
    payload: Vec<u8>,
    encoding: zenoh::bytes::Encoding,
    congestion: Option<zenoh::qos::CongestionControl>,
    noted_partial: &mut bool,
) -> zenoh::Result<()> {
    let mut delivered = false;
    let mut failure = None;
    for session in sessions {
        let result = if let Some(congestion) = congestion {
            session
                .put(key.as_str(), payload.clone())
                .encoding(encoding.clone())
                .congestion_control(congestion)
                .wait()
        } else {
            session
                .put(key.as_str(), payload.clone())
                .encoding(encoding.clone())
                .wait()
        };
        match result {
            Ok(()) => delivered = true,
            Err(error) => failure = Some(error),
        }
    }
    if delivered {
        if let Some(error) = failure
            && !*noted_partial
        {
            warn!("Terra Zenoh publish failed on one session: {error}");
            *noted_partial = true;
        }
        Ok(())
    } else {
        Err(failure.unwrap_or_else(|| zenoh_error("Zenoh publish had no session")))
    }
}

#[derive(Default)]
struct FleetState {
    session: Mutex<Option<zenoh::Session>>,
    error: Mutex<Option<zenoh::Error>>,
}

fn zenoh_error(message: impl Into<String>) -> zenoh::Error {
    std::io::Error::other(message.into()).into()
}

fn redact(message: &str, password: Option<&str>) -> String {
    match password {
        Some(password) if !password.is_empty() => message.replace(password, "***"),
        _ => message.to_owned(),
    }
}
