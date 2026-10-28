//! Zorvane lobby server. A Zenoh router that hosts connect out to: each host
//! opens a room under `zorvane/room/<code>` and answers `zorvane/rooms/<code>`
//! with its name, phase and free rovers, so players on the same lobby can list
//! the rooms and join one. Nobody but the lobby needs an open port.
//!
//! ```sh
//! cargo run --release -p zorvane-lobby -- --listen tcp/0.0.0.0:7447 --name "GMU lobby"
//! ```
//!
//! `--config <file>` (or `ZORVANE_LOBBY_CONFIG`) takes a full Zenoh router
//! configuration instead, for TLS and user/password.
use std::time::Duration;
use zenoh::Wait;

const ROOMS: &str = "zorvane/rooms";

struct Args {
    listen: String,
    name: String,
    config: Option<String>,
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args {
        listen: std::env::var("ZORVANE_LOBBY_LISTEN").unwrap_or_else(|_| "tcp/0.0.0.0:7447".into()),
        name: std::env::var("ZORVANE_LOBBY_NAME").unwrap_or_else(|_| "Zorvane lobby".into()),
        config: std::env::var("ZORVANE_LOBBY_CONFIG")
            .ok()
            .filter(|path| !path.is_empty()),
    };
    let mut words = std::env::args().skip(1);
    while let Some(word) = words.next() {
        let mut value = || words.next().ok_or(format!("{word} needs a value"));
        match word.as_str() {
            "--listen" => args.listen = value()?,
            "--name" => args.name = value()?,
            "--config" => args.config = Some(value()?),
            "-h" | "--help" => {
                println!(
                    "zorvane-lobby [--listen tcp/0.0.0.0:7447] [--name NAME] [--config zenoh.json5]"
                );
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument {other}")),
        }
    }
    Ok(args)
}

fn config(args: &Args) -> zenoh::Result<zenoh::Config> {
    if let Some(path) = &args.config {
        return zenoh::Config::from_file(path);
    }
    let mut config = zenoh::Config::default();
    config.insert_json5("mode", "\"router\"")?;
    config.insert_json5(
        "listen/endpoints",
        &serde_json::to_string(&[&args.listen]).unwrap(),
    )?;
    config.insert_json5("scouting/multicast/enabled", "false")?;
    Ok(config)
}

fn main() {
    let args = parse_args().unwrap_or_else(|error| {
        eprintln!("zorvane-lobby: {error}");
        std::process::exit(2);
    });
    let session = config(&args)
        .and_then(|config| zenoh::open(config).wait())
        .unwrap_or_else(|error| {
            eprintln!("zorvane-lobby: could not start: {error}");
            std::process::exit(1);
        });
    // Hosts hold a liveliness token per room; log rooms as they open and close.
    let _rooms = session
        .liveliness()
        .declare_subscriber(format!("{ROOMS}/*"))
        .history(true)
        .callback(|sample| {
            let room = sample
                .key_expr()
                .as_str()
                .rsplit('/')
                .next()
                .unwrap_or("?")
                .to_owned();
            match sample.kind() {
                zenoh::sample::SampleKind::Put => eprintln!("room {room} opened"),
                zenoh::sample::SampleKind::Delete => eprintln!("room {room} closed"),
            }
        })
        .wait()
        .expect("room subscriber");
    let info = serde_json::to_vec(&serde_json::json!({
        "schema_version": 1,
        "name": args.name,
        "version": env!("CARGO_PKG_VERSION"),
    }))
    .unwrap();
    let _info = session
        .declare_queryable("zorvane/lobby")
        .callback(move |query| {
            let key = query.key_expr().clone();
            let _ = query.reply(key, info.clone()).wait();
        })
        .wait()
        .expect("lobby queryable");
    match &args.config {
        Some(path) => eprintln!("{}: Zorvane lobby from {path}", args.name),
        None => eprintln!("{}: Zorvane lobby on {}", args.name, args.listen),
    }
    loop {
        std::thread::sleep(Duration::from_secs(3600));
    }
}
