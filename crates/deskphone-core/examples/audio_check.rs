//! Opens speaker + microphone exactly like a call would (plays silence only, discards mic data)
//! and reports formats and whether microphone samples arrive at 8 kHz.
use std::time::Duration;
fn main() {
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::new("info")).init();
    let Some(dev) = deskphone_core::rtp::AudioDeviceManager::try_init() else {
        println!("Keine Audio-Hardware gefunden");
        return;
    };
    // Wie der RTP-Sender: alle 20 ms einen 160-Sample-Frame abholen (2 s lang)
    let start = std::time::Instant::now();
    let (mut ok, mut empty) = (0, 0);
    for i in 1..=100u32 {
        if dev.read_mic_samples(160).is_some() { ok += 1 } else { empty += 1 }
        let target = start + Duration::from_millis(20 * i as u64);
        if let Some(d) = target.checked_duration_since(std::time::Instant::now()) { std::thread::sleep(d); }
    }
    println!("Mikrofon: {} Frames mit Audio, {} leer (Vorpuffer) von 100 x 20 ms", ok, empty);
}
