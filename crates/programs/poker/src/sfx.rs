//! Short, generated arcade sounds. No sample files, no external assets.
//!
//! Pocketboi already opens `cread-sound` for its speaker, so the poker machine
//! follows the same path: the shell chooses the device, the program owns the
//! little stream that belongs to its screen.

use cread_sound::open_output;
use rodio::{OutputStream, Sink, buffer::SamplesBuffer};

const RATE: u32 = 44_100;
const VOLUME: f32 = 0.32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Effect {
    Deal,
    Hold,
    Win,
    Loss,
    Coin,
    RiskWin,
    RiskLoss,
}

pub(super) struct Sfx {
    stream: Option<OutputStream>,
    sink: Option<Sink>,
    muted: bool,
}

impl Sfx {
    pub(super) fn new() -> Self {
        Self {
            stream: None,
            sink: None,
            muted: false,
        }
    }

    pub(super) fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
        if let Some(sink) = &self.sink {
            sink.set_volume(if muted { 0.0 } else { VOLUME });
        }
    }

    fn ensure_open(&mut self) -> Option<&Sink> {
        if self.sink.is_none() {
            let stream = open_output()?;
            let sink = Sink::connect_new(stream.mixer());
            sink.set_volume(if self.muted { 0.0 } else { VOLUME });
            self.stream = Some(stream);
            self.sink = Some(sink);
        }
        self.sink.as_ref()
    }

    pub(super) fn play(&mut self, effect: Effect) {
        if self.muted {
            return;
        }
        let samples = match effect {
            Effect::Deal => sequence(&[(880.0, 18), (1_100.0, 18), (1_320.0, 22)], 7, 0.35),
            Effect::Hold => sequence(&[(660.0, 26)], 0, 0.3),
            Effect::Win => sequence(&[(880.0, 55), (1_108.0, 55), (1_318.0, 95)], 18, 0.55),
            Effect::Loss => sequence(&[(110.0, 170), (98.0, 230)], 12, 0.6),
            Effect::Coin => sequence(&[(1_568.0, 35), (2_093.0, 65)], 8, 0.45),
            Effect::RiskWin => sequence(&[(988.0, 45), (1_318.0, 70)], 10, 0.5),
            Effect::RiskLoss => sequence(&[(123.0, 150), (110.0, 220)], 10, 0.55),
        };
        if samples.is_empty() {
            return;
        }
        if let Some(sink) = self.ensure_open() {
            sink.append(SamplesBuffer::new(1, RATE, samples));
        }
    }
}

fn sequence(notes: &[(f32, u32)], gap_ms: u32, volume: f32) -> Vec<f32> {
    let mut out = Vec::new();
    for (index, &(frequency, ms)) in notes.iter().enumerate() {
        if index > 0 {
            out.extend(std::iter::repeat_n(0.0, samples_for(gap_ms)));
        }
        out.extend(tone(frequency, ms, volume));
    }
    out
}

fn samples_for(ms: u32) -> usize {
    (RATE as u64 * u64::from(ms) / 1_000) as usize
}

fn tone(frequency: f32, ms: u32, volume: f32) -> Vec<f32> {
    let count = samples_for(ms);
    let period = RATE as f32 / frequency.max(1.0);
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let phase = (i as f32 / period).fract();
        let wave = if phase < 0.5 { 1.0 } else { -1.0 };
        let at = i as f32 / count.max(1) as f32;
        let envelope = if at < 0.06 {
            at / 0.06
        } else if at > 0.82 {
            (1.0 - at) / 0.18
        } else {
            1.0
        };
        out.push(wave * volume * envelope);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_tones_are_in_range() {
        let tone = tone(440.0, 20, 0.3);
        assert_eq!(tone.len(), samples_for(20));
        assert!(tone.iter().all(|sample| sample.abs() <= 0.31));
        assert!(tone.iter().any(|sample| sample.abs() > 0.01));
    }

    #[test]
    fn a_sequence_has_every_note_plus_gaps() {
        let samples = sequence(&[(440.0, 10), (880.0, 10)], 5, 0.2);
        assert_eq!(samples.len(), samples_for(10) * 2 + samples_for(5));
    }
}
