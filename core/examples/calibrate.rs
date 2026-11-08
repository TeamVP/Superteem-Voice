//! Voice similarity separation on WAVs named <voice>-<n>.wav:
//! cargo run --release --example calibrate -- <seconds> files...
use stv_core::embed::{similarity, voice, Voice};
fn read(path: &str) -> Vec<i16> {
    let d = std::fs::read(path).unwrap();
    let at = d.windows(4).position(|w| w == b"data").unwrap() + 8;
    d[at..].chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect()
}
fn main() {
    let mut args = std::env::args().skip(1);
    let secs: f32 = args.next().unwrap().parse().unwrap();
    let chunk = (16_000.0 * secs) as usize;
    let mut voices: Vec<(String, Voice)> = Vec::new();
    for p in args {
        let name = std::path::Path::new(&p).file_stem().unwrap().to_string_lossy().split('-').next().unwrap().to_string();
        for c in read(&p).chunks(chunk) {
            if c.len() == chunk { if let Some(v) = voice(c, 16_000) { voices.push((name.clone(), v)); } }
        }
    }
    let (mut same, mut diff) = (Vec::new(), Vec::new());
    for i in 0..voices.len() { for j in i + 1..voices.len() {
        let s = similarity(&voices[i].1, &voices[j].1);
        if voices[i].0 == voices[j].0 { same.push(s) } else { diff.push(s) }
    }}
    let pct = |v: &mut Vec<f32>, q: f32| { v.sort_by(f32::total_cmp); v[((v.len() - 1) as f32 * q) as usize] };
    println!("{} chunks; same p10 {:.2} p50 {:.2}; diff p50 {:.2} p90 {:.2} max {:.2}", voices.len(),
        pct(&mut same, 0.1), pct(&mut same, 0.5), pct(&mut diff, 0.5), pct(&mut diff, 0.9), pct(&mut diff, 1.0));
    let auc = same.iter().map(|s| diff.iter().filter(|d| *d < s).count()).sum::<usize>() as f64 / (same.len() * diff.len()) as f64;
    println!("AUC {auc:.3}");
}
