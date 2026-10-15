//! Print `x y heading` for rover 0.
//!
//! ```sh
//! cargo run -p zorvane-client --example print_pose
//! ```
//!
//! Open the printed pairing URL. `--native` joins a Zenoh simulator instead.

use std::env;
use std::time::Duration;

use zorvane_client::{BrowserHostConfig, Client};

fn main() {
    let mut rover = 0u64;
    let mut seconds: Option<f64> = None;
    let mut browser = true;
    let mut play_url = None;
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--browser" => browser = true,
            "--native" => browser = false,
            "--play-url" => play_url = Some(args.next().expect("--play-url needs a URL")),
            "--rover" => {
                rover = args
                    .next()
                    .expect("--rover needs an id")
                    .parse()
                    .expect("rover id");
            }
            "--seconds" => {
                seconds = Some(
                    args.next()
                        .expect("--seconds needs a number")
                        .parse()
                        .expect("seconds"),
                );
            }
            other => panic!("unknown argument {other}"),
        }
    }
    let client = if browser {
        let mut config = BrowserHostConfig::default();
        if let Some(url) = play_url {
            config.play_url = url;
        }
        let client = Client::host_browser(config).expect("start browser host");
        eprintln!(
            "Open this pairing URL, then select a world and press Start:\n{}",
            client.pairing_url().unwrap()
        );
        client
    } else {
        Client::connect().expect("Zenoh connect")
    };
    client
        .subscribe_pose(rover, |pose| {
            println!("{:.4} {:.4} {:.4}", pose.x, pose.y, pose.heading);
        })
        .expect("subscribe");
    if browser {
        client
            .wait_connected(Duration::from_secs(120))
            .expect("browser pairing timed out");
    }
    match seconds {
        Some(seconds) => std::thread::sleep(Duration::from_secs_f32(seconds as f32)),
        None => loop {
            std::thread::sleep(Duration::from_secs(3600));
        },
    }
}
