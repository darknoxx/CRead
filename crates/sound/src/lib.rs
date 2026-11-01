//! The machine's sound output, in one place.
//!
//! Both the game and the music deck want the sound server's PCM rather than a
//! hardware device, so the choice lives here instead of in a program crate:
//! the shell must not depend on a module to play music.

use rodio::{OutputStream, OutputStreamBuilder};

/// Open the output the way a desktop expects it.
///
/// Not `open_default_stream`: that one falls back to the first *hardware*
/// device it can find when the default refuses to open, which is how a game
/// ends up on the speakers while the system is set to headphones. The sound
/// server's own PCM (`pipewire`, `pulse`) is the one that follows the system
/// default and moves a stream when you switch output, so it is asked for by
/// name; plain `default` is the next choice, hardware never.
pub fn open_output() -> Option<OutputStream> {
    use rodio::cpal::traits::{DeviceTrait, HostTrait};
    use rodio::{Device, cpal};
    let host = cpal::default_host();
    let mut fallback: Option<Device> = None;
    if let Ok(devices) = host.output_devices() {
        for device in devices {
            let Ok(name) = device.name() else { continue };
            match name.to_ascii_lowercase().as_str() {
                "pipewire" | "pulse" => {
                    return OutputStreamBuilder::from_device(device)
                        .ok()?
                        .open_stream_or_fallback()
                        .ok();
                }
                "default" if fallback.is_none() => fallback = Some(device),
                _ => {}
            }
        }
    }
    let device = fallback.or_else(|| host.default_output_device())?;
    OutputStreamBuilder::from_device(device)
        .ok()?
        .open_stream_or_fallback()
        .ok()
}
