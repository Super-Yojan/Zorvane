use crate::{ClientError, Result};
use std::{
    collections::BTreeSet,
    io,
    net::{SocketAddr, TcpListener, TcpStream},
    path::PathBuf,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use subtle::ConstantTimeEq;
use tungstenite::{
    Message, WebSocket,
    handshake::server::{Request, Response},
    protocol::WebSocketConfig,
};
use zorvane_browser_protocol::{
    Control, HEARTBEAT_MS, LEASE_MS, MAX_PACKET_BYTES, Outbox, Publication, VERSION, matches_topic,
};

/// The gateway runs inside this process and only accepts loopback connections.
#[derive(Clone, Debug)]
pub struct BrowserHostConfig {
    pub bind_addr: SocketAddr,
    /// HTTPS URLs are proxied and opened from this process. Loopback HTTP is
    /// opened directly so a local Trunk server can supply the page.
    pub play_url: String,
    pub recording_dir: PathBuf,
    /// Proxy `play_url` even when it is loopback HTTP. Used by tests.
    #[doc(hidden)]
    pub force_proxy: bool,
}

impl Default for BrowserHostConfig {
    fn default() -> Self {
        Self {
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            play_url: "https://super-yojan.dev/Zorvane/play/webgpu/".into(),
            recording_dir: "runs".into(),
            force_proxy: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConnectionState {
    Waiting,
    Connected,
    Disconnected(String),
}

pub(crate) type Callback = Arc<dyn Fn(Publication) + Send + Sync>;
struct State {
    connection: ConnectionState,
    subscriptions: Vec<(String, Callback)>,
    subscription_revision: u64,
    outgoing: Outbox,
    incoming: Outbox,
    fatal: Option<String>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            connection: ConnectionState::Waiting,
            subscriptions: Vec::new(),
            subscription_revision: 0,
            outgoing: Outbox::default(),
            incoming: Outbox::default(),
            fatal: None,
        }
    }
}
struct Shared {
    state: Mutex<State>,
    changed: Condvar,
    stop: AtomicBool,
}

pub(crate) struct BrowserHost {
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
    pairing: String,
}

fn error(e: impl std::fmt::Display) -> ClientError {
    ClientError(e.to_string())
}
fn random_id() -> Result<String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(error)?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

fn tokens_match(candidate: &str, token: &str) -> bool {
    bool::from(candidate.as_bytes().ct_eq(token.as_bytes()))
}

fn loopback_http(url: &url::Url) -> bool {
    if url.scheme() != "http" {
        return false;
    }
    match url.host() {
        Some(url::Host::Domain(host)) => host.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(address)) => address.is_loopback(),
        Some(url::Host::Ipv6(address)) => address.is_loopback(),
        None => false,
    }
}

impl BrowserHost {
    pub fn start(config: BrowserHostConfig) -> Result<Self> {
        if !config.bind_addr.ip().is_loopback() {
            return Err(error("browser host must bind to a loopback address"));
        }
        let configured = url::Url::parse(&config.play_url).map_err(error)?;
        if configured.scheme() != "https" && !loopback_http(&configured) {
            return Err(error("play_url must use HTTPS or loopback HTTP"));
        }
        if !configured.username().is_empty() || configured.password().is_some() {
            return Err(error("play_url cannot contain credentials"));
        }
        // A public HTTPS page cannot open ws://127.0.0.1. Serve that page from
        // this loopback listener instead. Loopback HTTP is a developer's own server.
        let proxy = configured.scheme() == "https" || config.force_proxy;
        let listener = TcpListener::bind(config.bind_addr).map_err(error)?;
        listener.set_nonblocking(true).map_err(error)?;
        let address = listener.local_addr().map_err(error)?;
        let token = random_id()?;
        let fragment = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("endpoint", &format!("ws://{address}/bridge"))
            .append_pair("token", &token)
            .finish();
        let mut page = if proxy {
            url::Url::parse(&format!(
                "http://{address}{}",
                crate::page::directory_prefix(&configured)
            ))
            .map_err(error)?
        } else {
            configured.clone()
        };
        page.set_fragment(Some(&fragment));
        let origin = page.origin().ascii_serialization();
        let upstream = proxy.then_some(configured);
        let http_agent = upstream.as_ref().map(|_| Arc::new(crate::page::agent()));
        let shared = Arc::new(Shared {
            state: Mutex::new(State::default()),
            changed: Condvar::new(),
            stop: AtomicBool::new(false),
        });
        let thread_shared = shared.clone();
        let recording_dir = config.recording_dir.clone();
        let session_gate = Arc::new(Mutex::new(()));
        let worker = thread::Builder::new()
            .name("zorvane-browser-host".into())
            .spawn(move || {
                while !thread_shared.stop.load(Ordering::Acquire) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            let origin = origin.clone();
                            let token = token.clone();
                            let recording_dir = recording_dir.clone();
                            let upstream = upstream.clone();
                            let http_agent = http_agent.clone();
                            let shared = thread_shared.clone();
                            let session_gate = session_gate.clone();
                            thread::spawn(move || {
                                dispatch(
                                    stream,
                                    &origin,
                                    &token,
                                    &recording_dir,
                                    upstream,
                                    http_agent,
                                    &shared,
                                    &session_gate,
                                );
                            });
                        }
                        Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(5))
                        }
                        Err(_) => break,
                    }
                }
            })
            .map_err(error)?;
        // Callbacks never run on the socket worker. Slow callbacks cannot keep a
        // command lease alive; their bounded inbox eventually fails closed.
        let dispatch = shared.clone();
        thread::Builder::new()
            .name("zorvane-browser-callbacks".into())
            .spawn(move || {
                while !dispatch.stop.load(Ordering::Acquire) {
                    let (publication, callbacks) = {
                        let mut state = dispatch.state.lock().unwrap();
                        match state.incoming.pop() {
                            Some(p) => {
                                let callbacks = state
                                    .subscriptions
                                    .iter()
                                    .filter(|(expression, _)| matches_topic(expression, &p.topic))
                                    .map(|(_, cb)| cb.clone())
                                    .collect::<Vec<_>>();
                                (Some(p), callbacks)
                            }
                            None => {
                                drop(
                                    dispatch
                                        .changed
                                        .wait_timeout(state, Duration::from_millis(20))
                                        .unwrap(),
                                );
                                (None, Vec::new())
                            }
                        }
                    };
                    if let Some(p) = publication {
                        for callback in callbacks {
                            if dispatch.stop.load(Ordering::Acquire) {
                                return;
                            }
                            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                callback(p.clone())
                            }))
                            .is_err()
                            {
                                dispatch.state.lock().unwrap().fatal =
                                    Some("subscriber callback panicked".into());
                                break;
                            }
                        }
                    }
                }
            })
            .map_err(error)?;
        Ok(Self {
            shared,
            worker: Some(worker),
            pairing: page.into(),
        })
    }

    pub fn pairing_url(&self) -> &str {
        &self.pairing
    }
    pub fn connection_state(&self) -> ConnectionState {
        self.shared.state.lock().unwrap().connection.clone()
    }
    pub fn wait_connected(&self, timeout: Duration) -> Result<()> {
        let (state, _) = self
            .shared
            .changed
            .wait_timeout_while(self.shared.state.lock().unwrap(), timeout, |state| {
                state.connection != ConnectionState::Connected
            })
            .unwrap();
        if state.connection == ConnectionState::Connected {
            Ok(())
        } else {
            Err(error(format!(
                "browser did not pair within {timeout:?}: {:?}",
                state.connection
            )))
        }
    }
    pub fn subscribe(&self, expression: String, callback: Callback) -> Result<()> {
        let mut state = self.shared.state.lock().unwrap();
        if state.subscriptions.len() >= 256 {
            return Err(error("too many subscriptions"));
        }
        state.subscriptions.push((expression, callback));
        state.subscription_revision += 1;
        Ok(())
    }
    pub fn publish(&self, publication: Publication) -> Result<()> {
        let mut state = self.shared.state.lock().unwrap();
        if state.connection != ConnectionState::Connected || state.fatal.is_some() {
            return Err(error("browser is not connected"));
        }
        if let Err(e) = state.outgoing.push(publication) {
            state.fatal = Some(e.clone());
            return Err(error(e));
        }
        Ok(())
    }
}

impl Drop for BrowserHost {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Release);
        self.shared.changed.notify_all();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// Socket timeouts bound individual reads, not the entire HTTP/Hello exchange.
/// Shut down the underlying socket at an absolute deadline (or on host drop),
/// including when a peer drips a partial handshake or fragmented Hello.
struct PairingDeadline {
    done: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl PairingDeadline {
    fn new(stream: &TcpStream, shared: Arc<Shared>) -> std::result::Result<Self, String> {
        let socket = stream.try_clone().map_err(|e| e.to_string())?;
        let done = Arc::new(AtomicBool::new(false));
        let completed = done.clone();
        let worker = thread::Builder::new()
            .name("zorvane-pairing-deadline".into())
            .spawn(move || {
                let deadline = Instant::now() + Duration::from_millis(500);
                while !completed.load(Ordering::Acquire) {
                    if shared.stop.load(Ordering::Acquire) || Instant::now() >= deadline {
                        let _ = socket.shutdown(std::net::Shutdown::Both);
                        break;
                    }
                    thread::park_timeout(Duration::from_millis(10));
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            done,
            worker: Some(worker),
        })
    }
}

impl Drop for PairingDeadline {
    fn drop(&mut self) {
        self.done.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            worker.thread().unpark();
            let _ = worker.join();
        }
    }
}

fn send(socket: &mut WebSocket<TcpStream>, control: &Control) -> std::result::Result<(), String> {
    socket
        .send(Message::text(
            serde_json::to_string(control).map_err(|e| e.to_string())?,
        ))
        .map_err(|e| e.to_string())
}

fn dispatch(
    stream: TcpStream,
    origin: &str,
    token: &str,
    directory: &std::path::Path,
    upstream: Option<url::Url>,
    http_agent: Option<Arc<ureq::Agent>>,
    shared: &Arc<Shared>,
    session_gate: &Arc<Mutex<()>>,
) {
    let peeked = match peek_headers(&stream, &shared.stop) {
        Ok(bytes) => bytes,
        Err(()) => return,
    };
    if !websocket_upgrade(&peeked) {
        if let (Some(upstream), Some(agent)) = (upstream, http_agent) {
            crate::page::serve(stream, &upstream, &agent);
        }
        return;
    }
    let connected = shared.state.lock().unwrap().connection == ConnectionState::Connected;
    let _session = if connected {
        match session_gate.try_lock() {
            Ok(guard) => guard,
            Err(_) => return,
        }
    } else {
        session_gate.lock().unwrap()
    };
    let result = serve(stream, origin, token, directory, shared);
    let mut state = shared.state.lock().unwrap();
    state.connection = ConnectionState::Disconnected(
        result
            .err()
            .unwrap_or_else(|| "browser disconnected".into()),
    );
    state.outgoing = Outbox::default();
    state.incoming = Outbox::default();
    state.fatal = None;
    shared.changed.notify_all();
}

fn peek_headers(stream: &TcpStream, stop: &AtomicBool) -> std::result::Result<Vec<u8>, ()> {
    stream.set_nonblocking(true).map_err(|_| ())?;
    let mut buffer = vec![0u8; 8192];
    let deadline = Instant::now() + Duration::from_millis(500);
    loop {
        if stop.load(Ordering::Acquire) {
            let _ = stream.shutdown(std::net::Shutdown::Both);
            return Err(());
        }
        match stream.peek(&mut buffer) {
            Ok(0) => return Err(()),
            Ok(count)
                if buffer[..count]
                    .windows(4)
                    .any(|window| window == b"\r\n\r\n") =>
            {
                return Ok(buffer[..count].to_vec());
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
            Err(_) => return Err(()),
        }
        if Instant::now() >= deadline {
            let _ = stream.shutdown(std::net::Shutdown::Both);
            return Err(());
        }
        thread::sleep(Duration::from_millis(5));
    }
}

fn websocket_upgrade(bytes: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return false;
    };
    let mut lines = text.split("\r\n");
    let Some(request) = lines.next() else {
        return false;
    };
    let mut parts = request.split_whitespace();
    let Some(method) = parts.next() else {
        return false;
    };
    let Some(target) = parts.next() else {
        return false;
    };
    let path = target.split('?').next().unwrap_or(target);
    method == "GET"
        && path == "/bridge"
        && lines.any(|line| {
            line.split_once(':').is_some_and(|(name, value)| {
                name.eq_ignore_ascii_case("upgrade")
                    && value.trim().eq_ignore_ascii_case("websocket")
            })
        })
}

fn serve(
    stream: TcpStream,
    origin: &str,
    token: &str,
    directory: &std::path::Path,
    shared: &Arc<Shared>,
) -> std::result::Result<(), String> {
    let deadline = PairingDeadline::new(&stream, shared.clone())?;
    // Accepted sockets inherit O_NONBLOCK on macOS. Pairing has a bounded
    // blocking deadline; switch back to polling only after authentication.
    stream.set_nonblocking(false).map_err(|e| e.to_string())?;
    stream
        .set_read_timeout(Some(Duration::from_millis(500)))
        .map_err(|e| e.to_string())?;
    stream
        .set_write_timeout(Some(Duration::from_millis(100)))
        .map_err(|e| e.to_string())?;
    stream.set_nodelay(true).map_err(|e| e.to_string())?;
    let host = stream.local_addr().map_err(|e| e.to_string())?.to_string();
    let settings = WebSocketConfig::default()
        .max_message_size(Some(MAX_PACKET_BYTES))
        .max_frame_size(Some(MAX_PACKET_BYTES))
        .max_write_buffer_size(MAX_PACKET_BYTES + 4096)
        .write_buffer_size(0);
    let mut socket = tungstenite::accept_hdr_with_config(
        stream,
        |request: &Request, response: Response| {
            let allowed = request.uri().path() == "/bridge"
                && request
                    .headers()
                    .get("Origin")
                    .and_then(|v| v.to_str().ok())
                    == Some(origin)
                && request.headers().get("Host").and_then(|v| v.to_str().ok())
                    == Some(host.as_str());
            if allowed {
                Ok(response)
            } else {
                Err(tungstenite::http::Response::builder()
                    .status(403)
                    .body(Some("pairing origin rejected".into()))
                    .unwrap())
            }
        },
        Some(settings),
    )
    .map_err(|_| "WebSocket handshake rejected".to_string())?;
    let hello = socket.read();
    if let Err(e) = &hello {
        let _ = send(
            &mut socket,
            &Control::Error {
                message: format!("pairing message could not be read: {e}"),
            },
        );
        return Err("pairing message could not be read".into());
    }
    let authenticated = match &hello {
        Ok(Message::Text(text)) if text.len() <= 4096 => {
            match serde_json::from_str::<Control>(text) {
                Ok(Control::Hello {
                    version,
                    token: candidate,
                }) => version == VERSION && tokens_match(&candidate, token),
                _ => false,
            }
        }
        _ => false,
    };
    if !authenticated {
        let _ = send(
            &mut socket,
            &Control::Error {
                message: "invalid pairing secret or protocol version".into(),
            },
        );
        return Err("pairing rejected".into());
    }
    drop(deadline);
    send(&mut socket, &Control::Ready { version: VERSION })?;
    socket
        .get_mut()
        .set_nonblocking(true)
        .map_err(|e| e.to_string())?;
    {
        let mut state = shared.state.lock().unwrap();
        state.connection = ConnectionState::Connected;
        shared.changed.notify_all();
    }
    let mut last_received = Instant::now();
    let mut last_sent = Instant::now();
    let mut revision = u64::MAX;
    let mut recorder: Option<terra_experiment::RunRecorder> = None;
    let mut run_id: Option<String> = None;
    while !shared.stop.load(Ordering::Acquire) {
        if last_received.elapsed() >= Duration::from_millis(LEASE_MS) {
            return Err("browser heartbeat expired".into());
        }
        if let Some(reason) = shared.state.lock().unwrap().fatal.take() {
            return Err(reason);
        }
        for _ in 0..64 {
            match socket.read() {
                Ok(Message::Text(text)) if text.len() <= 1024 * 1024 => {
                    match serde_json::from_str::<Control>(&text)
                        .map_err(|_| "invalid control message")?
                    {
                        Control::Heartbeat => last_received = Instant::now(),
                        Control::Manifest {
                            run_id: id,
                            manifest,
                        } if run_id.is_none() => {
                            let result = (|| {
                                std::fs::create_dir_all(directory)?;
                                let unique = random_id().map_err(io::Error::other)?;
                                terra_experiment::RunRecorder::create(
                                    &directory.join(format!("browser-{}.jsonl", &unique[..24])),
                                    manifest,
                                    8192,
                                )
                            })();
                            recorder = result.ok();
                            run_id = Some(id);
                            send(
                                &mut socket,
                                &Control::Recorder {
                                    healthy: recorder.as_ref().is_some_and(|r| !r.failed()),
                                },
                            )?;
                        }
                        Control::Record { event } => {
                            if event.get("run_id").and_then(|v| v.as_str()) != run_id.as_deref() {
                                return Err("record run mismatch".into());
                            }
                            if let Some(recorder) = &recorder {
                                let _ = recorder.record(event);
                            }
                        }
                        _ => return Err("unexpected control message".into()),
                    }
                }
                Ok(Message::Binary(bytes)) => {
                    let publication = Publication::decode(&bytes)?;
                    let mut state = shared.state.lock().unwrap();
                    if state
                        .subscriptions
                        .iter()
                        .any(|(expression, _)| matches_topic(expression, &publication.topic))
                    {
                        state.incoming.push(publication)?;
                        shared.changed.notify_all();
                    }
                }
                Ok(Message::Close(_)) => return Ok(()),
                Ok(Message::Ping(_) | Message::Pong(_)) => {}
                Ok(_) => return Err("unsupported WebSocket message".into()),
                Err(tungstenite::Error::Io(e)) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(_) => return Err("browser socket closed".into()),
            }
        }
        let subscriptions = {
            let state = shared.state.lock().unwrap();
            if state.subscription_revision != revision {
                revision = state.subscription_revision;
                Some(
                    state
                        .subscriptions
                        .iter()
                        .map(|(topic, _)| topic.clone())
                        .collect::<BTreeSet<_>>()
                        .into_iter()
                        .collect(),
                )
            } else {
                None
            }
        };
        if let Some(topics) = subscriptions {
            send(&mut socket, &Control::Subscribe { topics })?;
        }
        if last_sent.elapsed() >= Duration::from_millis(HEARTBEAT_MS) {
            send(&mut socket, &Control::Heartbeat)?;
            if run_id.is_some() {
                send(
                    &mut socket,
                    &Control::Recorder {
                        healthy: recorder.as_ref().is_some_and(|r| !r.failed()),
                    },
                )?;
            }
            last_sent = Instant::now();
        }
        for _ in 0..32 {
            let publication = shared.state.lock().unwrap().outgoing.pop();
            let Some(publication) = publication else {
                break;
            };
            // Any blocked control write terminates this session; nothing is
            // replayed on a later connection.
            socket
                .send(Message::Binary(publication.encode()?.into()))
                .map_err(|_| "browser write failed")?;
        }
        socket.flush().map_err(|_| "browser flush failed")?;
        thread::sleep(Duration::from_millis(5));
    }
    let _ = socket.close(None);
    Ok(())
}
