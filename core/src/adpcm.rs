//! IMA ADPCM, 4 bits per sample. Each frame carries its starting state, so a lost frame cannot
//! damage the next one. Payload: predictor i16, step index u8, then samples, low nibble first.

const STATE_LEN: usize = 3;

const INDEX_TABLE: [i32; 16] = [-1, -1, -1, -1, 2, 4, 6, 8, -1, -1, -1, -1, 2, 4, 6, 8];
const STEP_TABLE: [i32; 89] = [
    7, 8, 9, 10, 11, 12, 13, 14, 16, 17, 19, 21, 23, 25, 28, 31, 34, 37, 41, 45, 50, 55, 60, 66, 73, 80, 88, 97, 107,
    118, 130, 143, 157, 173, 190, 209, 230, 253, 279, 307, 337, 371, 408, 449, 494, 544, 598, 658, 724, 796, 876, 963,
    1060, 1166, 1282, 1411, 1552, 1707, 1878, 2066, 2272, 2499, 2749, 3024, 3327, 3660, 4026, 4428, 4871, 5358, 5894,
    6484, 7132, 7845, 8630, 9493, 10442, 11487, 12635, 13899, 15289, 16818, 18500, 20350, 22385, 24623, 27086, 29794,
    32767,
];

fn apply(nibble: u8, predictor: &mut i32, index: &mut i32) {
    let step = STEP_TABLE[*index as usize];
    let mut diff = step >> 3;
    if nibble & 4 != 0 {
        diff += step;
    }
    if nibble & 2 != 0 {
        diff += step >> 1;
    }
    if nibble & 1 != 0 {
        diff += step >> 2;
    }
    *predictor = (*predictor + if nibble & 8 != 0 { -diff } else { diff }).clamp(-32768, 32767);
    *index = (*index + INDEX_TABLE[nibble as usize]).clamp(0, 88);
}

/// Encodes one frame. An encoder keeps `index` between frames so the step size stays adapted.
pub fn encode(pcm: &[i16], index: &mut u8) -> Vec<u8> {
    let first = pcm.first().copied().unwrap_or(0);
    let mut out = Vec::with_capacity(STATE_LEN + pcm.len().div_ceil(2));
    out.extend_from_slice(&first.to_le_bytes());
    out.push(*index);
    let mut predictor = first as i32;
    let mut idx = *index as i32;
    for (i, &sample) in pcm.iter().enumerate() {
        let step = STEP_TABLE[idx as usize];
        let mut delta = sample as i32 - predictor;
        let mut nibble = 0u8;
        if delta < 0 {
            nibble = 8;
            delta = -delta;
        }
        if delta >= step {
            nibble |= 4;
            delta -= step;
        }
        if delta >= step >> 1 {
            nibble |= 2;
            delta -= step >> 1;
        }
        if delta >= step >> 2 {
            nibble |= 1;
        }
        apply(nibble, &mut predictor, &mut idx);
        if i % 2 == 0 {
            out.push(nibble);
        } else {
            *out.last_mut().unwrap() |= nibble << 4;
        }
    }
    *index = idx as u8;
    out
}

/// Decodes a frame's payload, or returns None if the state is invalid. Always yields an even
/// number of samples, because the frame does not record an odd final nibble.
pub fn decode(payload: &[u8]) -> Option<Vec<i16>> {
    if payload.len() < STATE_LEN || payload[2] > 88 {
        return None;
    }
    let mut predictor = i16::from_le_bytes([payload[0], payload[1]]) as i32;
    let mut index = payload[2] as i32;
    let mut out = Vec::with_capacity((payload.len() - STATE_LEN) * 2);
    for byte in &payload[STATE_LEN..] {
        for nibble in [byte & 0x0F, byte >> 4] {
            apply(nibble, &mut predictor, &mut index);
            out.push(predictor as i16);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn speechlike(n: usize) -> Vec<i16> {
        (0..n)
            .map(|i| {
                let t = i as f32 / 16_000.0;
                let envelope = 0.5 + 0.5 * (t * 2.0 * std::f32::consts::PI * 3.0).sin();
                let voice = (t * 1382.0).sin() * 6000.0 + (t * 4147.0).sin() * 2500.0 + (t * 9000.0).sin() * 800.0;
                (voice * envelope) as i16
            })
            .collect()
    }

    fn snr_db(reference: &[i16], decoded: &[i16]) -> f64 {
        let signal: f64 = reference.iter().map(|&s| (s as f64).powi(2)).sum();
        let noise: f64 = reference.iter().zip(decoded).map(|(&a, &b)| (a as f64 - b as f64).powi(2)).sum();
        10.0 * (signal / noise.max(1.0)).log10()
    }

    fn round_trip(pcm: &[i16]) -> Vec<i16> {
        let mut index = 0;
        pcm.chunks(320).flat_map(|c| decode(&encode(c, &mut index)).unwrap()).collect()
    }

    #[test]
    fn a_20ms_frame_is_163_bytes() {
        assert_eq!(encode(&[0; 320], &mut 0).len(), 163);
    }

    #[test]
    fn speech_like_audio_survives_with_good_snr() {
        let pcm = speechlike(16_000);
        let decoded = round_trip(&pcm);
        assert_eq!(decoded.len(), pcm.len());
        let snr = snr_db(&pcm, &decoded);
        assert!(snr > 20.0, "snr {snr}");
    }

    #[test]
    fn a_lost_frame_does_not_damage_later_frames() {
        let pcm = speechlike(3200);
        let mut index = 0;
        let frames: Vec<Vec<u8>> = pcm.chunks(320).map(|c| encode(c, &mut index)).collect();
        assert_eq!(decode(&frames[5]).unwrap(), round_trip(&pcm)[1600..1920]);
    }

    #[test]
    fn extreme_input_does_not_panic() {
        let pcm: Vec<i16> = (0..3200).map(|i| if i % 2 == 0 { i16::MAX } else { i16::MIN }).collect();
        assert_eq!(round_trip(&pcm).len(), 3200);
    }

    #[test]
    fn rejects_invalid_state() {
        assert_eq!(decode(&[0, 0]), None);
        assert_eq!(decode(&[0, 0, 89, 0]), None);
    }
}
