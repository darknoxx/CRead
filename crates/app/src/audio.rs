//! Music playback: rodio in a corner of the shell.
//!
//! This lives in `app` rather than in the Riff program because music has to keep
//! playing while another program is in front, and because `riff play` has to work
//! from any prompt — even from the boot screen, before Riff was ever opened.

use std::fs::File;
use std::path::Path;
use std::time::Duration;

use rodio::{Decoder, OutputStream, Sink, Source};

/// The output device and one sink, replaced per track.
///
/// A sink is created fresh for every track instead of reusing one: `Sink::stop`
/// is terminal, and dropping a sink already silences what it plays.
pub struct Audio {
    stream: OutputStream,
    sink: Sink,
    length: Duration,
    volume: u8,
}

impl Audio {
    /// Open the default output. Fails cleanly when the machine has no sound.
    pub fn open(volume: u8) -> Result<Self, String> {
        let stream = cread_sound::open_output().ok_or_else(|| "no sound device".to_string())?;
        let audio = Self {
            sink: Sink::connect_new(stream.mixer()),
            stream,
            length: Duration::ZERO,
            volume: volume.min(100),
        };
        audio.apply_volume();
        Ok(audio)
    }

    fn fresh_sink(&self) -> Sink {
        let sink = Sink::connect_new(self.stream.mixer());
        sink.set_volume(self.volume as f32 / 100.0);
        sink
    }

    fn apply_volume(&self) {
        self.sink.set_volume(self.volume as f32 / 100.0);
    }

    /// Start a track, replacing whatever was playing. Returns its length when
    /// the file carries one.
    pub fn play(&mut self, path: &Path) -> Result<Duration, String> {
        let file = File::open(path).map_err(|e| format!("cannot open {}: {e}", path.display()))?;
        let decoder =
            Decoder::try_from(file).map_err(|e| format!("cannot play {}: {e}", path.display()))?;
        let length = decoder.total_duration().unwrap_or(Duration::ZERO);
        self.sink = self.fresh_sink();
        self.sink.append(decoder);
        self.length = length;
        Ok(length)
    }

    /// Silence and forget the current track.
    pub fn stop(&mut self) {
        self.sink = self.fresh_sink();
        self.length = Duration::ZERO;
    }

    pub fn pause(&self) {
        self.sink.pause();
    }

    pub fn resume(&self) {
        self.sink.play();
    }

    pub fn is_paused(&self) -> bool {
        self.sink.is_paused()
    }

    /// True when nothing is left in the sink: the track ended, or nothing was
    /// started at all.
    pub fn finished(&self) -> bool {
        self.sink.empty()
    }

    pub fn position(&self) -> Duration {
        self.sink.get_pos()
    }

    pub fn length(&self) -> Duration {
        self.length
    }

    pub fn set_volume(&mut self, volume: u8) {
        self.volume = volume.min(100);
        self.apply_volume();
    }
}

/// `m:ss`, the readout a tape counter would give.
pub fn clock(time: Duration) -> String {
    let total = time.as_secs();
    format!("{:02}:{:02}", total / 60, total % 60)
}
