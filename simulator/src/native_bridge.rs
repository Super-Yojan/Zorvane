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
    let subscriber = subscribe(&session, &config.prefix, shared, false)?;
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
            let loopback = spec.listen.iter().all(|endpoint| is_loopback(endpoint));
            let subscriber = subscribe(&session, &config.prefix, shared, loopback)?;
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
            subscribe(&session, &prefix, &shared, false).map(|subscriber| (session, subscriber))
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

/// The command subscriber and the multiplayer queryables of one session.
struct Inbox {
    _commands: zenoh::pubsub::Subscriber<()>,
    _join: zenoh::query::Queryable<()>,
    _ping: zenoh::query::Queryable<()>,
}

/// Only a listener on this machine may stand in for the host player's seat.
fn is_loopback(endpoint: &str) -> bool {
    let address = endpoint.split_once('/').map_or(endpoint, |(_, rest)| rest);
    address.starts_with("127.") || address.starts_with("[::1]") || address.starts_with("localhost:")
}

fn subscribe(
    session: &zenoh::Session,
    prefix: &str,
    shared: &Arc<Shared>,
    local: bool,
) -> zenoh::Result<Inbox> {
    let incoming = shared.clone();
    let owned_prefix = prefix.to_owned();
    let commands = session
        .declare_subscriber(format!("{prefix}/**"))
        .callback(move |sample| {
            let attachment = sample.attachment().map(|bytes| bytes.to_bytes());
            incoming.receive_signed(
                &owned_prefix,
                sample.key_expr().as_str(),
                sample.payload().to_bytes().as_ref(),
                attachment.as_deref(),
                local,
            );
        })
        .wait()?;
    // Joins need the fleet, so the simulation answers them on its next frame.
    let joins = shared.clone();
    let join = session
        .declare_queryable(format!("{prefix}/match/join"))
        .callback(move |query| joins.joins.lock().unwrap().push(query))
        .wait()?;
    // Answered here, so a client measures the network, not the frame rate.
    let ping = session
        .declare_queryable(format!("{prefix}/match/ping"))
        .callback(|query| {
            let key = query.key_expr().clone();
            if let Err(error) = query.reply(key, "{}").wait() {
                debug!("match/ping reply failed: {error}");
            }
        })
        .wait()?;
    Ok(Inbox {
        _commands: commands,
        _join: join,
        _ping: ping,
    })
}

fn publish_loop(
    config: &BridgeConfig,
    shared: &Arc<Shared>,
    local: Option<zenoh::Session>,
    fleet: Option<&FleetState>,
) -> zenoh::Result<()> {
    shared.connection_generation.fetch_add(1, Ordering::Release);
    shared.connected.store(true, Ordering::Release);
    let mut occupancy_baselines = BTreeMap::<u64, OccupancyBaseline>::new();
    let mut noted_partial = false;
    let mut room = RoomLink::default();
    let local = local.map(|session| (session, config.prefix.clone()));
    while !shared.stop.load(Ordering::Acquire) {
        room.sync(shared);
        let sessions = active_sessions(local.as_ref(), fleet, &config.prefix, &room);
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
                format!("{id}/camera/{kind}"),
                payload,
                zenoh::bytes::Encoding::APPLICATION_OCTET_STREAM,
                Some(zenoh::qos::CongestionControl::Drop),
                &mut noted_partial,
            )?;
        }
        if let Some(state) = outgoing.fleet {
            publish_each(
                &sessions,
                "fleet/state".to_owned(),
                state,
                zenoh::bytes::Encoding::APPLICATION_JSON,
                Some(zenoh::qos::CongestionControl::Drop),
                &mut noted_partial,
            )?;
        }
        if let Some(state) = outgoing.match_state {
            publish_each(
                &sessions,
                "match/state".to_owned(),
                state,
                zenoh::bytes::Encoding::APPLICATION_JSON,
                Some(zenoh::qos::CongestionControl::Drop),
                &mut noted_partial,
            )?;
        }
        if let Some(score) = outgoing.next_score {
            publish_each(
                &sessions,
                "next/score".to_owned(),
                score,
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
                    format!("{id}/{kind}"),
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
                    format!("{id}/map/occupancy"),
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
                    format!("{id}/map/occupancy/heartbeat"),
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
                        format!("{id}/search/report"),
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
                format!("{id}/goal/status"),
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
                format!("{id}/pose"),
                payload,
                zenoh::bytes::Encoding::APPLICATION_JSON,
                Some(zenoh::qos::CongestionControl::Drop),
                &mut noted_partial,
            )?;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    room.close();
    Ok(())
}

/// Every open session with the prefix it publishes under.
fn active_sessions(
    local: Option<&(zenoh::Session, String)>,
    fleet: Option<&FleetState>,
    prefix: &str,
    room: &RoomLink,
) -> Vec<(zenoh::Session, String)> {
    let mut sessions = Vec::new();
    if let Some(session) = local {
        sessions.push(session.clone());
    }
    if let Some(fleet) = fleet
        && let Some(session) = fleet.session.lock().unwrap().clone()
    {
        sessions.push((session, prefix.to_owned()));
    }
    if let Some(open) = &room.open {
        sessions.push((open.session.clone(), open.prefix.clone()));
    }
    sessions
}

/// The hosted room's session on the lobby server. Connecting runs on its own
/// thread so a slow TLS handshake never stalls the local listener.
#[derive(Default)]
struct RoomLink {
    spec: Option<RoomSpec>,
    pending: Option<Arc<Mutex<Option<Result<OpenRoom, String>>>>>,
    open: Option<OpenRoom>,
}

struct OpenRoom {
    session: zenoh::Session,
    prefix: String,
    _inbox: Inbox,
    _info: zenoh::query::Queryable<()>,
    _token: zenoh::liveliness::LivelinessToken,
}

impl RoomLink {
    fn sync(&mut self, shared: &Arc<Shared>) {
        let wanted = shared.room.lock().unwrap().wanted.clone();
        if wanted != self.spec {
            self.close();
            self.spec = wanted.clone();
            if let Some(spec) = wanted {
                let slot = Arc::new(Mutex::new(None));
                self.pending = Some(slot.clone());
                let shared = shared.clone();
                std::thread::Builder::new()
                    .name("zorvane-room".into())
                    .spawn(move || {
                        let result = open_room(&spec, &shared).map_err(|error| {
                            redact(&error.to_string(), spec.server_password.as_deref())
                        });
                        *slot.lock().unwrap() = Some(result);
                    })
                    .expect("could not start room thread");
            }
        }
        let finished = self
            .pending
            .as_ref()
            .and_then(|slot| slot.lock().unwrap().take());
        if let Some(result) = finished {
            self.pending = None;
            let mut room = shared.room.lock().unwrap();
            // The menu may have closed or replaced the room while it connected.
            if room.wanted != self.spec {
                return;
            }
            match result {
                Ok(open) => {
                    info!("Zorvane room open under {}", open.prefix);
                    room.status = RoomStatus::Open;
                    self.open = Some(open);
                }
                Err(error) => {
                    warn!("Zorvane room failed: {error}");
                    room.status = RoomStatus::Failed(error);
                }
            }
        }
    }

    fn close(&mut self) {
        self.pending = None;
        if let Some(open) = self.open.take() {
            let OpenRoom { session, .. } = open;
            if let Err(error) = session.close().wait() {
                debug!("closing the room session: {error}");
            }
        }
        self.spec = None;
    }
}

fn open_room(spec: &RoomSpec, shared: &Arc<Shared>) -> zenoh::Result<OpenRoom> {
    let link = ZenohLinkSpec {
        mode: "client",
        listen: Vec::new(),
        connect: vec![spec.server.clone()],
        user: spec.user.clone(),
        password: spec.server_password.clone(),
        tls_root_ca: None,
        exit_on_connect_failure: true,
    };
    let session = zenoh::open(link.to_zenoh_config()?).wait()?;
    let prefix = crate::lobby::room_prefix(&spec.meta.code);
    let inbox = subscribe(&session, &prefix, shared, false)?;
    let info = shared.clone();
    let _info = session
        .declare_queryable(format!("{}/{}", crate::lobby::ROOMS, spec.meta.code))
        .callback(move |query| {
            let body = info.room_info.lock().unwrap().clone();
            if body.is_empty() {
                return;
            }
            let key = query.key_expr().clone();
            if let Err(error) = query.reply(key, body).wait() {
                debug!("room info reply failed: {error}");
            }
        })
        .wait()?;
    let _token = session
        .liveliness()
        .declare_token(format!("{}/{}", crate::lobby::ROOMS, spec.meta.code))
        .wait()?;
    Ok(OpenRoom {
        session,
        prefix,
        _inbox: inbox,
        _info,
        _token,
    })
}

/// Publishes `key` (relative to each session's prefix) on every session.
fn publish_each(
    sessions: &[(zenoh::Session, String)],
    key: String,
    payload: Vec<u8>,
    encoding: zenoh::bytes::Encoding,
    congestion: Option<zenoh::qos::CongestionControl>,
    noted_partial: &mut bool,
) -> zenoh::Result<()> {
    let mut delivered = false;
    let mut failure = None;
    for (session, prefix) in sessions {
        let key = format!("{prefix}/{key}");
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
