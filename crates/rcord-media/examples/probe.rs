//! Plays a file headless and reports what came out: `cargo run -p rcord-media --example probe -- file.mp4`.
use std::time::{Duration, Instant};

fn main() {
    let path = std::env::args().nth(1).expect("a file");
    let (player, ch) = rcord_media::Player::open(std::fs::read(path).expect("readable"));
    let start = Instant::now();
    let ready = ch.events.recv_blocking().expect("event");
    println!("{ready:?}");
    let first = ch.frames.recv_blocking().expect("first frame");
    println!(
        "first frame {}x{} at {} ms",
        first.width, first.height, first.at_ms
    );
    player.play();
    let (mut n, mut last, mut seeked) = (1, 0, false);
    while start.elapsed() < Duration::from_secs(20) {
        if let Ok(e) = ch.events.try_recv() {
            println!(
                "{e:?} after {:?}, {n} frames, last at {last} ms",
                start.elapsed()
            );
            if seeked {
                return;
            }
            seeked = true;
            n = 0;
            player.seek(1500);
            player.play();
        }
        match ch.frames.try_recv() {
            Ok(f) => {
                if n == 0 {
                    println!("after seek, first frame at {} ms", f.at_ms);
                }
                n += 1;
                last = f.at_ms;
            }
            Err(_) => std::thread::sleep(Duration::from_millis(2)),
        }
    }
    println!("timed out, {n} frames, last at {last} ms");
}
