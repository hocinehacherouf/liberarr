//! Audio preparation for local transcription: Ogg Opus to mono 16 kHz samples, then loudness normalisation.
use liberarr_core::audio::ogg::opus_packets;
use liberarr_core::{Error, Result};

const RATE: usize = 16_000;
/// Longest Opus packet: 120 ms.
const MAX_PACKET_SAMPLES: usize = RATE * 120 / 1000;

fn undecodable() -> Error {
    Error::Unsupported("audio could not be decoded".into())
}

/// Mono f32 samples at 16 kHz in [-1, 1], pre-skip removed (pre_skip is in 48 kHz samples: drop pre_skip / 3).
pub fn decode_opus_16k(data: &[u8]) -> Result<Vec<f32>> {
    let stream = opus_packets(data).map_err(|_| undecodable())?;
    // libopus down-mixes a stereo stream to the mono decoder.
    let mut decoder =
        opus::Decoder::new(RATE as u32, opus::Channels::Mono).map_err(|_| undecodable())?;
    let mut out = Vec::new();
    let mut frame = [0f32; MAX_PACKET_SAMPLES];
    for packet in &stream.packets {
        // An empty packet makes libopus synthesise 120 ms of concealment audio, shifting every later timestamp.
        if packet.is_empty() {
            continue;
        }
        let n = decoder
            .decode_float(packet, &mut frame, false)
            .map_err(|_| undecodable())?;
        out.extend_from_slice(&frame[..n]);
    }
    out.drain(..(usize::from(stream.pre_skip) / 3).min(out.len()));
    Ok(out)
}

const FRAME: usize = RATE * 150 / 1000;
const WINDOW: usize = 15;
const TARGET_PEAK: f32 = 0.95;
const MAX_GAIN: f32 = 10.0;
const SILENCE: f32 = 1e-4;

/// Dynamic normaliser (like ffmpeg dynaudnorm f=150 g=15, which measurably helped Whisper): a gain per
/// 150 ms frame, a minimum filter then a Gaussian over 15 frames (so a quiet frame's high gain never
/// spills onto loud neighbours), linearly interpolated between frame centres. A static gain for the whole
/// file measured worse than the raw audio.
pub fn normalize_dynamic(samples: &mut [f32]) {
    let gains: Vec<f32> = samples
        .chunks(FRAME)
        .map(|f| {
            let peak = f.iter().fold(0f32, |m, s| m.max(s.abs()));
            if peak < SILENCE {
                MAX_GAIN
            } else {
                (TARGET_PEAK / peak).min(MAX_GAIN)
            }
        })
        .collect();
    let half = WINDOW / 2;
    let around = |i: usize| i.saturating_sub(half)..(i + half + 1).min(gains.len());
    let minimal: Vec<f32> = (0..gains.len())
        .map(|i| gains[around(i)].iter().fold(f32::MAX, |m, g| m.min(*g)))
        .collect();
    let sigma = half as f32 / 3.0;
    let smooth: Vec<f32> = (0..minimal.len())
        .map(|i| {
            let (mut sum, mut weight) = (0.0, 0.0);
            for j in around(i) {
                let d = j as f32 - i as f32;
                let w = (-d * d / (2.0 * sigma * sigma)).exp();
                sum += w * minimal[j];
                weight += w;
            }
            sum / weight
        })
        .collect();
    let Some(last) = smooth.len().checked_sub(1) else {
        return;
    };
    for (i, s) in samples.iter_mut().enumerate() {
        // Position in frame units, 0.0 at the first frame centre.
        let x = ((i as f32 + 0.5) / FRAME as f32 - 0.5).clamp(0.0, last as f32);
        let k = (x as usize).min(last);
        let next = (k + 1).min(last);
        let gain = smooth[k] + (smooth[next] - smooth[k]) * (x - k as f32);
        // Last safety net only: the gains already keep peaks near 0.95.
        *s = (*s * gain).clamp(-1.0, 1.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Vec<u8> {
        std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/sine-440hz-2s.opus"
        ))
        .unwrap()
    }

    fn sine(amplitude: f32, seconds: usize) -> Vec<f32> {
        (0..seconds * RATE)
            .map(|i| {
                amplitude * (2.0 * std::f32::consts::PI * 440.0 * i as f32 / RATE as f32).sin()
            })
            .collect()
    }

    fn peak(s: &[f32]) -> f32 {
        s.iter().fold(0f32, |m, v| m.max(v.abs()))
    }

    #[test]
    fn decodes_the_sine_fixture() {
        let s = decode_opus_16k(&fixture()).unwrap();
        assert!(s.len().abs_diff(32_000) <= 640, "length {}", s.len());
        let crossings = s
            .windows(2)
            .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
            .count();
        assert!(crossings.abs_diff(1760) <= 88, "crossings {crossings}");
        let rms = (s.iter().map(|v| v * v).sum::<f32>() / s.len() as f32).sqrt();
        assert!(rms > 0.05, "rms {rms}");
    }

    #[test]
    fn decode_rejects_garbage() {
        assert!(decode_opus_16k(&[0x5a; 4096]).is_err());
        assert!(decode_opus_16k(&[]).is_err());
        let f = fixture();
        assert!(decode_opus_16k(&f[..f.len() / 2]).is_err());
    }

    #[test]
    fn normaliser_lifts_quiet_without_clipping() {
        let mut s = sine(0.02, 5);
        s.extend(sine(0.9, 5));
        s.extend(vec![0.0; RATE]);
        let input = s.clone();
        normalize_dynamic(&mut s);
        assert!(peak(&s[2 * RATE..5 * RATE]) > 0.15);
        assert!(peak(&s[5 * RATE..10 * RATE]) <= 0.95 + 5e-3);
        // The quiet frames' gain of 10 must not spill onto the loud half (a clamp would hide it in the peak).
        for i in 6 * RATE + RATE / 2..10 * RATE {
            if input[i].abs() > 0.5 {
                assert!(s[i].abs() / input[i].abs() <= 1.06 + 0.01, "gain at {i}");
            }
        }
        assert!(s.iter().all(|v| v.abs() < 1.0));
        assert!(s[10 * RATE..].iter().all(|v| *v == 0.0));
    }
}
