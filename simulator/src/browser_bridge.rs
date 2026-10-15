//! Main-thread browser transport. Never blocks a Bevy schedule or GPU callback.
use super::*;
use std::{cell::RefCell, collections::VecDeque, rc::Rc};
use wasm_bindgen::{JsCast, closure::Closure};
use web_sys::{Event, MessageEvent, WebSocket};
use zorvane_browser_protocol::{
    Control, HEARTBEAT_MS, LEASE_MS, MAX_PACKET_BYTES, Outbox, Publication, VERSION, matches_topic,
};

enum Incoming {
    Text(String),
    Binary(Vec<u8>),
    Closed,
}
#[derive(Default)]
struct Inbox {
    messages: VecDeque<Incoming>,
    bytes: usize,
    failed: bool,
}

struct BrowserTransport {
    endpoint: String,
    token: String,
    prefix: String,
    shared: Arc<Shared>,
    socket: Option<WebSocket>,
    handlers: Vec<Closure<dyn FnMut(Event)>>,
    inbox: Rc<RefCell<Inbox>>,
    last_received: Instant,
    last_heartbeat: Instant,
    last_tick: Instant,
    next_connect: Instant,
    subscriptions: Vec<String>,
    outgoing: Outbox,
    baselines: BTreeMap<u64, OccupancyBaseline>,
    manifest_sent: bool,
}

fn status(message: &str) {
    if let Some(element) = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.get_element_by_id("bridge-status"))
    {
        element.set_text_content(Some(message));
        let _ = element.set_attribute("data-active", "true");
    }
}

pub(super) fn install(app: &mut App, shared: Arc<Shared>, config: &BridgeConfig) -> bool {
    let parameters = web_sys::window()
        .and_then(|w| w.location().hash().ok())
        .and_then(|hash| web_sys::UrlSearchParams::new_with_str(hash.trim_start_matches('#')).ok());
    let Some(parameters) = parameters else {
        return false;
    };
    let (Some(endpoint), Some(token)) = (parameters.get("endpoint"), parameters.get("token"))
    else {
        return false;
    };
    let valid = web_sys::Url::new(&endpoint).ok().is_some_and(|url| {
        url.protocol() == "ws:"
            && matches!(url.hostname().as_str(), "127.0.0.1" | "localhost" | "[::1]")
            && !url.port().is_empty()
            && url.pathname() == "/bridge"
            && url.username().is_empty()
            && url.password().is_empty()
    }) && token.len() == 64
        && token.bytes().all(|b| b.is_ascii_hexdigit());
    if !valid {
        status("Connection unavailable. Open a fresh pairing URL from the Zorvane client.");
        return false;
    }
    let now = Instant::now();
    app.insert_non_send(BrowserTransport {
        endpoint,
        token,
        prefix: config.prefix.clone(),
        shared,
        socket: None,
        handlers: Vec::new(),
        inbox: Rc::new(RefCell::new(Inbox::default())),
        last_received: now,
        last_heartbeat: now,
        last_tick: now,
        next_connect: now,
        subscriptions: Vec::new(),
        outgoing: Outbox::default(),
        baselines: BTreeMap::new(),
        manifest_sent: false,
    })
    .add_systems(PreUpdate, poll.before(crate::terra::reconcile_fleet))
    .add_systems(FixedPreUpdate, enforce_lease);
    status("Connecting to the Zorvane client.");
    true
}

impl BrowserTransport {
    fn connect(&mut self) -> Result<(), String> {
        let socket = WebSocket::new(&self.endpoint).map_err(|_| "The browser could not open the simulator connection. Reopen the pairing URL from the client.".to_string())?;
        socket.set_binary_type(web_sys::BinaryType::Arraybuffer);
        let incoming = self.inbox.clone();
        let message = Closure::<dyn FnMut(Event)>::new(move |event: Event| {
            let event: MessageEvent = event.unchecked_into();
            let data = event.data();
            let mut inbox = incoming.borrow_mut();
            let (message, size) = if let Some(text) = data.as_string() {
                let size = text.len();
                (Incoming::Text(text), size)
            } else if data.is_instance_of::<js_sys::ArrayBuffer>() {
                let bytes = js_sys::Uint8Array::new(&data);
                if bytes.length() as usize > MAX_PACKET_BYTES {
                    inbox.failed = true;
                    return;
                }
                let bytes = bytes.to_vec();
                let size = bytes.len();
                (Incoming::Binary(bytes), size)
            } else {
                inbox.failed = true;
                return;
            };
            if inbox.messages.len() >= 512 || inbox.bytes + size > 32 * 1024 * 1024 {
                inbox.failed = true;
                return;
            }
            inbox.bytes += size;
            inbox.messages.push_back(message);
        });
        socket.set_onmessage(Some(message.as_ref().unchecked_ref()));
        let incoming = self.inbox.clone();
        let close = Closure::<dyn FnMut(Event)>::new(move |_| {
            incoming.borrow_mut().messages.push_back(Incoming::Closed);
        });
        socket.set_onclose(Some(close.as_ref().unchecked_ref()));
        let incoming = self.inbox.clone();
        let error = Closure::<dyn FnMut(Event)>::new(move |_| {
            incoming.borrow_mut().failed = true;
        });
        socket.set_onerror(Some(error.as_ref().unchecked_ref()));
        let opened = socket.clone();
        let token = self.token.clone();
        let incoming = self.inbox.clone();
        let open = Closure::<dyn FnMut(Event)>::new(move |_| {
            let hello = serde_json::to_string(&Control::Hello {
                version: VERSION,
                token: token.clone(),
            })
            .unwrap();
            if opened.send_with_str(&hello).is_err() {
                incoming.borrow_mut().failed = true;
            }
        });
        socket.set_onopen(Some(open.as_ref().unchecked_ref()));
        self.handlers = vec![message, close, error, open];
        self.socket = Some(socket);
        self.last_received = Instant::now();
        self.last_heartbeat = Instant::now();
        Ok(())
    }

    fn disconnect(&mut self, reason: &str) {
        self.shared.disconnect();
        if let Some(socket) = self.socket.take() {
            socket.set_onopen(None);
            socket.set_onmessage(None);
            socket.set_onclose(None);
            socket.set_onerror(None);
            let _ = socket.close();
        }
        self.handlers.clear();
        *self.inbox.borrow_mut() = Inbox::default();
        self.outgoing = Outbox::default();
        self.baselines.clear();
        self.subscriptions.clear();
        self.manifest_sent = false;
        self.next_connect = Instant::now() + Duration::from_millis(1000);
        status(reason);
    }

    fn send(&self, control: &Control) -> Result<(), String> {
        let text = serde_json::to_string(control).map_err(|e| e.to_string())?;
        self.socket
            .as_ref()
            .ok_or("socket unavailable")?
            .send_with_str(&text)
            .map_err(|_| "WebSocket send failed".into())
    }

    fn check_lease(&mut self) {
        let now = Instant::now();
        let suspended = now.duration_since(self.last_tick) >= Duration::from_millis(LEASE_MS);
        self.last_tick = now;
        let expired = self.socket.is_some()
            && now.duration_since(self.last_received) >= Duration::from_millis(LEASE_MS);
        let hidden = web_sys::window()
            .and_then(|w| w.document())
            .is_some_and(|d| d.hidden());
        if self.socket.is_some() && (suspended || expired || hidden) {
            self.disconnect(
                "Disconnected. Remote motion stopped. Reconnecting when this tab is active.",
            );
        }
    }

    fn tick(&mut self) -> Result<(), String> {
        if self.socket.is_none() {
            if Instant::now() >= self.next_connect
                && web_sys::window()
                    .and_then(|w| w.document())
                    .is_some_and(|d| !d.hidden())
            {
                self.connect()?;
            }
            return Ok(());
        }
        let inbox = std::mem::take(&mut *self.inbox.borrow_mut());
        if inbox.failed {
            return Err(
                "Connection failed. Reopen the pairing URL printed by the Zorvane client.".into(),
            );
        }
        for message in inbox.messages {
            match message {
                Incoming::Closed => {
                    return Err("Client disconnected. Remote motion stopped; reconnecting.".into());
                }
                Incoming::Text(text) if text.len() <= 1024 * 1024 => {
                    match serde_json::from_str::<Control>(&text)
                        .map_err(|_| "invalid host control")?
                    {
                        Control::Ready { version: VERSION } => {
                            self.last_received = Instant::now();
                            self.shared.epoch.fetch_add(1, Ordering::Relaxed);
                            self.shared.connected.store(true, Ordering::Release);
                            status(
                                "Connected to the Zorvane client. Select a world and press Start.",
                            );
                        }
                        Control::Heartbeat if self.shared.connected.load(Ordering::Acquire) => {
                            self.last_received = Instant::now();
                        }
                        Control::Subscribe { topics }
                            if topics.len() <= 256
                                && topics
                                    .iter()
                                    .all(|t| zorvane_browser_protocol::valid_topic(t, true)) =>
                        {
                            self.subscriptions = topics;
                            self.shared.epoch.fetch_add(1, Ordering::Relaxed);
                            self.baselines.clear();
                            self.outgoing = Outbox::default();
                        }
                        Control::Recorder { healthy } => self
                            .shared
                            .recording
                            .healthy
                            .store(healthy, Ordering::Release),
                        Control::Error { message } => {
                            return Err(format!("Pairing rejected: {message}"));
                        }
                        _ => return Err("unsupported host protocol".into()),
                    }
                }
                Incoming::Binary(bytes) if self.shared.connected.load(Ordering::Acquire) => {
                    let publication = Publication::decode(&bytes)?;
                    self.shared
                        .receive(&self.prefix, &publication.topic, &publication.payload);
                }
                _ => return Err("unexpected host message".into()),
            }
        }
        if !self.shared.connected.load(Ordering::Acquire) {
            return Ok(());
        }
        if self.socket.as_ref().unwrap().buffered_amount() > 1024 * 1024 {
            // Let the socket drain; heartbeat expiration stops motion if it cannot.
            self.collect_publications()?;
            return Ok(());
        }
        if self.last_heartbeat.elapsed() >= Duration::from_millis(HEARTBEAT_MS) {
            self.send(&Control::Heartbeat)?;
            self.last_heartbeat = Instant::now();
        }
        if !self.manifest_sent {
            let manifest = self.shared.recording.manifest.lock().unwrap().clone();
            if let Some((run_id, manifest)) = manifest {
                self.send(&Control::Manifest { run_id, manifest })?;
                self.manifest_sent = true;
            }
        }
        for _ in 0..64 {
            if self.socket.as_ref().unwrap().buffered_amount() > 1024 * 1024 {
                break;
            }
            let event = self.shared.recording.events.lock().unwrap().pop_front();
            let Some(event) = event else {
                break;
            };
            self.send(&Control::Record { event })?;
        }
        self.collect_publications()?;
        for _ in 0..32 {
            if self.socket.as_ref().unwrap().buffered_amount() > 1024 * 1024 {
                break;
            }
            let Some(publication) = self.outgoing.pop() else {
                break;
            };
            self.socket
                .as_ref()
                .unwrap()
                .send_with_u8_array(&publication.encode()?)
                .map_err(|_| "WebSocket binary send failed")?;
        }
        Ok(())
    }

    fn collect_publications(&mut self) -> Result<(), String> {
        let outgoing = std::mem::take(&mut *self.shared.outgoing.lock().unwrap());
        let live: std::collections::BTreeSet<_> = self
            .shared
            .commands
            .lock()
            .unwrap()
            .keys()
            .copied()
            .collect();
        self.baselines.retain(|id, _| live.contains(id));
        for ((id, kind), payload) in outgoing.states {
            if live.contains(&id) {
                self.queue(format!("{}/{id}/{kind}", self.prefix), payload, false)?;
            }
        }
        for (id, payload) in outgoing.goals {
            if live.contains(&id) {
                self.queue(format!("{}/{id}/goal/status", self.prefix), payload, false)?;
            }
        }
        for (id, reports) in outgoing.reports {
            if live.contains(&id) {
                for payload in reports {
                    self.queue(
                        format!("{}/{id}/search/report", self.prefix),
                        payload,
                        false,
                    )?;
                }
            }
        }
        if let Some(payload) = outgoing.fleet {
            self.queue(format!("{}/fleet/state", self.prefix), payload, false)?;
        }
        for (id, payload) in outgoing.poses {
            if live.contains(&id) {
                self.queue(format!("{}/{id}/pose", self.prefix), payload, false)?;
            }
        }
        for (id, publication) in outgoing.occupancy {
            if live.contains(&id) {
                self.queue(
                    format!("{}/{id}/map/occupancy", self.prefix),
                    publication.payload,
                    false,
                )?;
                self.baselines.insert(id, publication.baseline);
            }
        }
        for id in outgoing.occupancy_heartbeats {
            if let Some(baseline) = self.baselines.get(&id) {
                self.queue(
                    format!("{}/{id}/map/occupancy/heartbeat", self.prefix),
                    serde_json::to_vec(baseline).unwrap(),
                    false,
                )?;
            }
        }
        for ((id, kind), payload) in outgoing.frames {
            if live.contains(&id) {
                self.queue(format!("{}/{id}/camera/{kind}", self.prefix), payload, true)?;
            }
        }
        Ok(())
    }

    fn queue(&mut self, topic: String, payload: Vec<u8>, binary: bool) -> Result<(), String> {
        if self
            .subscriptions
            .iter()
            .any(|expression| matches_topic(expression, &topic))
        {
            self.outgoing.push(Publication::new(
                topic,
                if binary {
                    "application/octet-stream"
                } else {
                    "application/json"
                },
                payload,
            ))?;
        }
        Ok(())
    }
}

impl Drop for BrowserTransport {
    fn drop(&mut self) {
        self.disconnect("Client connection closed.");
    }
}

fn poll(mut transport: NonSendMut<BrowserTransport>) {
    transport.check_lease();
    if let Err(reason) = transport.tick() {
        transport.disconnect(&reason);
    }
}
fn enforce_lease(mut transport: NonSendMut<BrowserTransport>) {
    transport.check_lease();
}
