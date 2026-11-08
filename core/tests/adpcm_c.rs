//! The firmware's C encoder must produce the same bytes as the Rust reference.

use stv_core::adpcm;

extern "C" {
    fn stv_adpcm_encode(pcm: *const i16, count: usize, index: *mut u8, out: *mut u8) -> usize;
}

fn encode_c(pcm: &[i16], index: &mut u8) -> Vec<u8> {
    let mut out = vec![0u8; 3 + pcm.len().div_ceil(2)];
    let len = unsafe { stv_adpcm_encode(pcm.as_ptr(), pcm.len(), index, out.as_mut_ptr()) };
    out.truncate(len);
    out
}

#[test]
fn c_encoder_matches_rust_across_varied_signals() {
    let mut seed = 99u64;
    let noise: Vec<i16> = (0..32_000)
        .map(|_| {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (seed >> 48) as i16
        })
        .collect();
    let tone: Vec<i16> = (0..32_000).map(|i| ((i as f32 * 0.09).sin() * 12_000.0) as i16).collect();
    let square: Vec<i16> = (0..32_000).map(|i| if i % 40 < 20 { i16::MAX } else { i16::MIN }).collect();

    for signal in [noise, tone, square, vec![0; 3200]] {
        let (mut rust_index, mut c_index) = (0u8, 0u8);
        for frame in signal.chunks(320) {
            assert_eq!(encode_c(frame, &mut c_index), adpcm::encode(frame, &mut rust_index));
            assert_eq!(c_index, rust_index);
        }
    }
}
