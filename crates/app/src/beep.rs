//! The startup beep: a PC speaker square wave, played by whatever the machine
//! already uses for sound.
//!
//! There is no audio stack in this project on purpose — a writing terminal has
//! no business linking ALSA. Instead the beep is written as a tiny WAV and handed
//! to the first player found on the PATH. No player, no beep: silence is a fine
//! failure mode for flavour.

use std::io::Write;
use std::process::{Command, Stdio};

/// Players to try, in order. All of them take a file name.
const PLAYERS: [&str; 4] = ["paplay", "pw-play", "aplay", "play"];

const SAMPLE_RATE: u32 = 44_100;
const FREQUENCY: f32 = 880.0;
const SECONDS: f32 = 0.09;

/// Beep once, without blocking the caller: the file is written and the player
/// runs on a thread of its own, so a slow sound server never stalls a frame.
pub fn play() {
    std::thread::spawn(|| {
        let path = std::env::temp_dir().join("cread-beep.wav");
        if write_wav(&path).is_err() {
            return;
        }
        for player in PLAYERS {
            let played = Command::new(player)
                .arg(&path)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            if played.is_ok() {
                return;
            }
        }
    });
}

fn write_wav(path: &std::path::Path) -> std::io::Result<()> {
    let mut file = std::fs::File::create(path)?;
    file.write_all(&wav())
}

/// The beep as a 16-bit mono WAV: a square wave with a short fade at both ends,
/// because a hard edge on a square wave clicks.
pub fn wav() -> Vec<u8> {
    let total = (SAMPLE_RATE as f32 * SECONDS) as usize;
    let period = SAMPLE_RATE as f32 / FREQUENCY;
    let fade = (SAMPLE_RATE as f32 * 0.004) as usize;
    let mut pcm = Vec::with_capacity(total * 2);
    for index in 0..total {
        let high = ((index as f32 / period) as u32).is_multiple_of(2);
        let base = if high { 0.25 } else { -0.25 };
        let envelope = if index < fade {
            index as f32 / fade as f32
        } else if index + fade > total {
            (total - index) as f32 / fade as f32
        } else {
            1.0
        };
        let sample = (base * envelope.clamp(0.0, 1.0) * i16::MAX as f32) as i16;
        pcm.extend_from_slice(&sample.to_le_bytes());
    }

    let mut out = Vec::with_capacity(44 + pcm.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&((36 + pcm.len()) as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // fmt chunk size
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    out.extend_from_slice(&(SAMPLE_RATE * 2).to_le_bytes()); // byte rate
    out.extend_from_slice(&2u16.to_le_bytes()); // block align
    out.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(pcm.len() as u32).to_le_bytes());
    out.extend_from_slice(&pcm);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_beep_is_a_valid_wav_header() {
        let bytes = wav();
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WAVE");
        assert_eq!(&bytes[12..16], b"fmt ");
        assert_eq!(u16::from_le_bytes([bytes[20], bytes[21]]), 1, "PCM");
        assert_eq!(u16::from_le_bytes([bytes[22], bytes[23]]), 1, "mono");
        assert_eq!(
            u32::from_le_bytes([bytes[24], bytes[25], bytes[26], bytes[27]]),
            SAMPLE_RATE
        );
        assert_eq!(u16::from_le_bytes([bytes[34], bytes[35]]), 16, "16-bit");
        let data = u32::from_le_bytes([bytes[40], bytes[41], bytes[42], bytes[43]]);
        assert_eq!(bytes.len(), 44 + data as usize);
        assert_eq!(data as usize, 2 * (SAMPLE_RATE as f32 * SECONDS) as usize);
        // The riff size covers everything after its own field.
        assert_eq!(
            u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as usize,
            bytes.len() - 8
        );
    }

    #[test]
    fn the_wave_starts_and_ends_quiet_and_swings_in_between() {
        let bytes = wav();
        let samples: Vec<i16> = bytes[44..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| i16::from_le_bytes(*c))
            .collect();
        assert_eq!(samples[0], 0, "no click at the start");
        let loudest = samples.iter().map(|s| s.abs()).max().unwrap();
        assert!(loudest > 8000, "audible: {loudest}");
        assert!(samples[samples.len() - 1].abs() < 200, "fades out");
    }
}
