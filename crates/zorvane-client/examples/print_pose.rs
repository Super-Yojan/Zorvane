//! Print `x y heading` for rover 0.
//!
//! ```sh
//! cargo run -p zorvane-client --example print_pose -- --seconds 10
//! ```

use std::env;
use std::time::Duration;

use zorvane_client::Client;

fn main() {
    let mut rover = 0u64;
    let mut seconds: Option<f64> = None;
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
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
    let client = Client::connect().expect("Zenoh connect");
    client
        .subscribe_pose(rover, |pose| {
            println!("{:.4} {:.4} {:.4}", pose.x, pose.y, pose.heading);
        })
        .expect("subscribe");
    match seconds {
        Some(seconds) => std::thread::sleep(Duration::from_secs_f32(seconds as f32)),
        None => loop {
            std::thread::sleep(Duration::from_secs(3600));
        },
    }
}
