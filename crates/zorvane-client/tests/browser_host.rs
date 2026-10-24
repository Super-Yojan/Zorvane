use std::{sync::mpsc, time::Duration};
use tungstenite::{Message, client::IntoClientRequest};
use zorvane_browser_protocol::{Control, Publication, VERSION};
use zorvane_client::{BrowserHostConfig, Client};

fn socket(client: &Client, token: Option<&str>) -> tungstenite::WebSocket<std::net::TcpStream> {
    let pairing = url::Url::parse(&client.pairing_url().unwrap()).unwrap();
    let fields: std::collections::BTreeMap<_, _> =
        url::form_urlencoded::parse(pairing.fragment().unwrap().as_bytes())
            .into_owned()
            .collect();
    let mut request = fields["endpoint"].as_str().into_client_request().unwrap();
    request.headers_mut().insert(
        "Origin",
        pairing.origin().ascii_serialization().parse().unwrap(),
    );
    let address = url::Url::parse(&fields["endpoint"]).unwrap();
    let stream =
        std::net::TcpStream::connect((address.host_str().unwrap(), address.port().unwrap()))
            .unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let (mut socket, _) = tungstenite::client(request, stream).unwrap();
    socket
        .send(Message::text(
            serde_json::to_string(&Control::Hello {
                version: VERSION,
                token: token.unwrap_or(&fields["token"]).into(),
            })
            .unwrap(),
        ))
        .unwrap();
    socket
}

fn text(socket: &mut tungstenite::WebSocket<std::net::TcpStream>) -> Control {
    loop {
        match socket.read().unwrap() {
            Message::Text(s) => {
                let message = serde_json::from_str(&s).unwrap();
                if matches!(message, Control::Heartbeat) {
                    socket
                        .send(Message::text(
                            serde_json::to_string(&Control::Heartbeat).unwrap(),
                        ))
                        .unwrap();
                } else {
                    return message;
                }
            }
            _ => panic!("expected control message"),
        }
    }
}

#[test]
fn paired_browser_exchanges_topics_and_camera_bytes() {
    let client = Client::host_browser(BrowserHostConfig::default()).unwrap();
    let (tx, rx) = mpsc::channel();
    client
        .subscribe_pose(7, move |pose| {
            tx.send(pose).unwrap();
        })
        .unwrap();
    let mut browser = socket(&client, None);
    let ready = text(&mut browser);
    assert!(
        matches!(ready, Control::Ready { version: VERSION }),
        "{ready:?}"
    );
    assert!(
        matches!(text(&mut browser), Control::Subscribe { topics } if topics == ["terra/rover/7/pose"])
    );
    client.wait_connected(Duration::from_secs(1)).unwrap();
    browser
        .send(Message::Binary(
            Publication::new(
                "terra/rover/7/pose",
                "application/json",
                br#"{"x":3,"y":-4,"heading":0.5}"#.to_vec(),
            )
            .encode()
            .unwrap()
            .into(),
        ))
        .unwrap();
    let pose = rx.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!((pose.x, pose.y, pose.heading), (3.0, -4.0, 0.5));
    client.set_velocity(7, 0.4, -0.2).unwrap();
    loop {
        if let Message::Binary(bytes) = browser.read().unwrap() {
            let publication = Publication::decode(&bytes).unwrap();
            assert_eq!(publication.topic, "terra/rover/7/cmd_vel");
            let value: serde_json::Value = serde_json::from_slice(&publication.payload).unwrap();
            assert_eq!(value, serde_json::json!({"linear":0.4,"angular":-0.2}));
            break;
        }
    }
    let (tx, rx) = mpsc::channel();
    client
        .subscribe("terra/rover/*/camera/depth", move |sample| {
            tx.send(sample.payload).unwrap();
        })
        .unwrap();
    assert!(matches!(text(&mut browser), Control::Subscribe { .. }));
    browser
        .send(Message::Binary(
            Publication::new(
                "terra/rover/7/camera/depth",
                "application/octet-stream",
                vec![0, 255, 10, 128],
            )
            .encode()
            .unwrap()
            .into(),
        ))
        .unwrap();
    assert_eq!(
        rx.recv_timeout(Duration::from_secs(1)).unwrap(),
        vec![0, 255, 10, 128]
    );
}

#[test]
fn wrong_secret_is_rejected_and_unpaired_commands_are_not_queued() {
    let client = Client::host_browser(BrowserHostConfig::default()).unwrap();
    assert!(client.set_velocity(0, 1.0, 0.0).is_err());
    let mut browser = socket(&client, Some("wrong"));
    assert!(matches!(text(&mut browser), Control::Error { .. }));
    assert!(client.wait_connected(Duration::from_millis(30)).is_err());
    drop(browser);
    let mut browser = socket(&client, None);
    assert!(matches!(text(&mut browser), Control::Ready { .. }));
}

#[test]
fn dropping_host_closes_listener_and_session() {
    let client = Client::host_browser(BrowserHostConfig::default()).unwrap();
    let mut browser = socket(&client, None);
    assert!(matches!(text(&mut browser), Control::Ready { .. }));
    drop(client);
    let mut closed = false;
    for _ in 0..5 {
        match browser.read() {
            Err(_) | Ok(Message::Close(_)) => {
                closed = true;
                break;
            }
            _ => {}
        }
    }
    assert!(closed);
}

#[test]
fn host_records_browser_events_and_acknowledges_health() {
    let directory = std::env::temp_dir().join(format!(
        "zorvane-recording-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let client = Client::host_browser(BrowserHostConfig {
        recording_dir: directory.clone(),
        ..Default::default()
    })
    .unwrap();
    let mut browser = socket(&client, None);
    assert!(matches!(text(&mut browser), Control::Ready { .. }));
    assert!(matches!(text(&mut browser), Control::Subscribe { .. }));
    browser
        .send(Message::text(
            serde_json::to_string(&Control::Manifest {
                run_id: "test-run".into(),
                manifest: serde_json::json!({"run_id":"test-run","platform":"browser"}),
            })
            .unwrap(),
        ))
        .unwrap();
    assert!(matches!(
        text(&mut browser),
        Control::Recorder { healthy: true }
    ));
    browser
        .send(Message::text(
            serde_json::to_string(&Control::Record {
                event: serde_json::json!({"run_id":"test-run","kind":"control_tick","sequence":7}),
            })
            .unwrap(),
        ))
        .unwrap();
    // Processing a later heartbeat proves the event was consumed before shutdown.
    browser
        .send(Message::text(
            serde_json::to_string(&Control::Heartbeat).unwrap(),
        ))
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    loop {
        let recorded = std::fs::read_dir(&directory)
            .unwrap()
            .filter_map(Result::ok)
            .any(|file| {
                std::fs::read_to_string(file.path())
                    .unwrap_or_default()
                    .lines()
                    .any(|line| {
                        serde_json::from_str::<serde_json::Value>(line)
                            .ok()
                            .is_some_and(|v| v["sequence"] == 7)
                    })
            });
        if recorded {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "browser record was not persisted"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    drop(client);
    let files: Vec<_> = std::fs::read_dir(&directory)
        .unwrap()
        .map(|f| f.unwrap().path())
        .collect();
    assert_eq!(files.len(), 1);
    let records = std::fs::read_to_string(&files[0]).unwrap();
    let records: Vec<serde_json::Value> = records
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    assert_eq!(records[0]["kind"], "manifest");
    assert!(records.iter().any(|r| r["sequence"] == 7));
    assert_eq!(records.last().unwrap()["kind"], "run_end");
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn heartbeat_expiry_disconnects_and_reconnect_does_not_replay_commands() {
    let client = Client::host_browser(BrowserHostConfig::default()).unwrap();
    let mut browser = socket(&client, None);
    assert!(matches!(text(&mut browser), Control::Ready { .. }));
    assert!(matches!(text(&mut browser), Control::Subscribe { .. }));
    std::thread::sleep(Duration::from_millis(650));
    assert!(client.set_velocity(0, 1.0, 0.0).is_err());
    drop(browser);
    let mut browser = socket(&client, None);
    assert!(matches!(text(&mut browser), Control::Ready { .. }));
    assert!(matches!(text(&mut browser), Control::Subscribe { .. }));
    client.cancel_goal(2).unwrap();
    loop {
        if let Message::Binary(bytes) = browser.read().unwrap() {
            let publication = Publication::decode(&bytes).unwrap();
            assert_eq!(publication.topic, "terra/rover/2/goal");
            assert_eq!(publication.payload, br#"{"cancel":true}"#);
            break;
        }
    }
}

#[test]
fn recorder_failure_is_reported_and_untrusted_origins_cannot_pair() {
    let client = Client::host_browser(BrowserHostConfig {
        recording_dir: std::path::PathBuf::from("/dev/null/recordings"),
        ..Default::default()
    })
    .unwrap();
    let pairing = url::Url::parse(&client.pairing_url().unwrap()).unwrap();
    let fields: std::collections::BTreeMap<_, _> =
        url::form_urlencoded::parse(pairing.fragment().unwrap().as_bytes())
            .into_owned()
            .collect();
    let mut request = fields["endpoint"].as_str().into_client_request().unwrap();
    request
        .headers_mut()
        .insert("Origin", "https://untrusted.invalid".parse().unwrap());
    let address = url::Url::parse(&fields["endpoint"]).unwrap();
    let stream =
        std::net::TcpStream::connect((address.host_str().unwrap(), address.port().unwrap()))
            .unwrap();
    assert!(tungstenite::client(request, stream).is_err());
    let mut browser = socket(&client, None);
    assert!(matches!(text(&mut browser), Control::Ready { .. }));
    assert!(matches!(text(&mut browser), Control::Subscribe { .. }));
    browser
        .send(Message::text(
            serde_json::to_string(&Control::Manifest {
                run_id: "failed-run".into(),
                manifest: serde_json::json!({"run_id":"failed-run"}),
            })
            .unwrap(),
        ))
        .unwrap();
    assert!(matches!(
        text(&mut browser),
        Control::Recorder { healthy: false }
    ));
}

#[test]
fn protocol_version_mismatch_and_competing_tabs_are_rejected() {
    let client = Client::host_browser(BrowserHostConfig::default()).unwrap();
    let mut browser = socket(&client, None);
    assert!(matches!(text(&mut browser), Control::Ready { .. }));
    assert!(matches!(text(&mut browser), Control::Subscribe { .. }));
    let pairing = url::Url::parse(&client.pairing_url().unwrap()).unwrap();
    let fields: std::collections::BTreeMap<_, _> =
        url::form_urlencoded::parse(pairing.fragment().unwrap().as_bytes())
            .into_owned()
            .collect();
    let mut request = fields["endpoint"].as_str().into_client_request().unwrap();
    request.headers_mut().insert(
        "Origin",
        pairing.origin().ascii_serialization().parse().unwrap(),
    );
    let address = url::Url::parse(&fields["endpoint"]).unwrap();
    let stream =
        std::net::TcpStream::connect((address.host_str().unwrap(), address.port().unwrap()))
            .unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(1)))
        .unwrap();
    assert!(tungstenite::client(request.clone(), stream).is_err());
    browser.close(None).unwrap();
    std::thread::sleep(Duration::from_millis(30));
    let stream =
        std::net::TcpStream::connect((address.host_str().unwrap(), address.port().unwrap()))
            .unwrap();
    let (mut browser, _) = tungstenite::client(request, stream).unwrap();
    browser
        .send(Message::text(
            serde_json::to_string(&Control::Hello {
                version: VERSION + 1,
                token: fields["token"].clone(),
            })
            .unwrap(),
        ))
        .unwrap();
    assert!(matches!(text(&mut browser), Control::Error { .. }));
}

#[test]
fn unauthenticated_slow_peer_cannot_block_host_shutdown() {
    use std::io::Write;
    let client = Client::host_browser(BrowserHostConfig::default()).unwrap();
    let pairing = url::Url::parse(&client.pairing_url().unwrap()).unwrap();
    let fields: std::collections::BTreeMap<_, _> =
        url::form_urlencoded::parse(pairing.fragment().unwrap().as_bytes())
            .into_owned()
            .collect();
    let address = url::Url::parse(&fields["endpoint"]).unwrap();
    let mut stream =
        std::net::TcpStream::connect((address.host_str().unwrap(), address.port().unwrap()))
            .unwrap();
    let drip = std::thread::spawn(move || {
        for _ in 0..20 {
            if stream.write_all(b"G").is_err() {
                break;
            }
            std::thread::sleep(Duration::from_millis(80));
        }
    });
    std::thread::sleep(Duration::from_millis(30));
    let before = std::time::Instant::now();
    drop(client);
    assert!(
        before.elapsed() < Duration::from_millis(500),
        "shutdown waited on an unauthenticated peer"
    );
    drip.join().unwrap();
}

#[test]
fn pairing_url_is_the_loopback_page_not_the_public_site() {
    let client = Client::host_browser(BrowserHostConfig::default()).unwrap();
    let pairing = url::Url::parse(&client.pairing_url().unwrap()).unwrap();
    assert_eq!(pairing.scheme(), "http");
    assert_eq!(pairing.host_str(), Some("127.0.0.1"));
    assert_eq!(pairing.path(), "/Zorvane/play/webgpu/");
    let endpoint = url::form_urlencoded::parse(pairing.fragment().unwrap().as_bytes())
        .find(|(key, _)| key == "endpoint")
        .unwrap()
        .1;
    assert!(endpoint.starts_with(&format!(
        "ws://127.0.0.1:{}/bridge",
        pairing.port().unwrap()
    )));
}

#[test]
fn gateway_serves_the_simulator_page_and_rejects_path_escape() {
    let fixture = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    fixture.set_nonblocking(true).unwrap();
    let fixture_addr = fixture.local_addr().unwrap();
    let fixture_thread = std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while std::time::Instant::now() < deadline {
            match fixture.accept() {
                Ok((mut stream, _)) => {
                    stream
                        .set_read_timeout(Some(Duration::from_secs(2)))
                        .unwrap();
                    let mut request = [0u8; 2048];
                    let count = std::io::Read::read(&mut stream, &mut request).unwrap_or(0);
                    let request = String::from_utf8_lossy(&request[..count]);
                    let body = if request.contains("index.html")
                        || request.starts_with("GET /Zorvane/play/webgpu/ HTTP")
                    {
                        b"page-ok".to_vec()
                    } else if request.contains("app.js") {
                        b"script-ok".to_vec()
                    } else {
                        b"nope".to_vec()
                    };
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = std::io::Write::write_all(&mut stream, response.as_bytes());
                    let _ = std::io::Write::write_all(&mut stream, &body);
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(_) => break,
            }
        }
    });
    let client = Client::host_browser(BrowserHostConfig {
        play_url: format!("http://{fixture_addr}/Zorvane/play/webgpu/"),
        force_proxy: true,
        ..BrowserHostConfig::default()
    })
    .unwrap();
    let pairing = url::Url::parse(&client.pairing_url().unwrap()).unwrap();
    assert_eq!(pairing.scheme(), "http");
    assert_ne!(pairing.port().unwrap(), fixture_addr.port());
    let page = http_get(&format!(
        "http://127.0.0.1:{}{}",
        pairing.port().unwrap(),
        pairing.path()
    ));
    assert!(page.contains("page-ok"), "{page}");
    let script = http_get(&format!(
        "http://127.0.0.1:{}/Zorvane/play/webgpu/app.js",
        pairing.port().unwrap()
    ));
    assert!(script.contains("script-ok"), "{script}");
    let escaped = http_get(&format!(
        "http://127.0.0.1:{}/Zorvane/play/webgpu/../../secret",
        pairing.port().unwrap()
    ));
    assert!(escaped.starts_with("HTTP/1.1 404"), "{escaped}");
    fixture_thread.join().unwrap();
}

fn http_get(url: &str) -> String {
    use std::io::{Read, Write};
    let url = url::Url::parse(url).unwrap();
    let mut stream =
        std::net::TcpStream::connect((url.host_str().unwrap(), url.port().unwrap())).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let path = format!(
        "{}{}",
        url.path(),
        url.query()
            .map(|query| format!("?{query}"))
            .unwrap_or_default()
    );
    stream
        .write_all(
            format!(
                "GET {path} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
                url.host_str().unwrap()
            )
            .as_bytes(),
        )
        .unwrap();
    let mut body = String::new();
    let _ = stream.read_to_string(&mut body);
    body
}
