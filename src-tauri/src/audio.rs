//! Microphone capture (cpal) and TTS playback (rodio).
//!
//! Capture runs on its own thread because a cpal `Stream` is not `Send` on every
//! backend — the thread owns the stream for the whole recording and hands samples
//! to the async side through an unbounded channel.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc as std_mpsc;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::SampleFormat;
use tokio::sync::mpsc;

/// Rates Deepgram accepts for `linear16`. Asking for one of these means we never
/// have to resample — we just tell Deepgram what the mic actually runs at.
const PREFERRED_RATES: [u32; 4] = [16_000, 48_000, 32_000, 8_000];

pub struct Capture {
    pub rx: mpsc::UnboundedReceiver<Vec<i16>>,
    pub sample_rate: u32,
    stop: Arc<AtomicBool>,
    level: Arc<AtomicU32>,
}

impl Capture {
    /// A view of this capture for code that is not reading its samples.
    ///
    /// The stream task needs to meter and stop the microphone while the channel
    /// itself is borrowed by the receive that is draining it.
    pub fn handle(&self) -> CaptureHandle {
        CaptureHandle { stop: self.stop.clone(), level: self.level.clone() }
    }

    /// Start capturing. `device_match` is a case-insensitive substring of the
    /// device name; empty means the system default input.
    pub fn start(device_match: &str) -> Result<Capture> {
        let (tx, rx) = mpsc::unbounded_channel::<Vec<i16>>();
        let (meta_tx, meta_rx) = std_mpsc::channel::<Result<u32>>();
        let stop = Arc::new(AtomicBool::new(false));
        let level = Arc::new(AtomicU32::new(0));

        let want = device_match.trim().to_lowercase();
        let (thread_stop, thread_level) = (stop.clone(), level.clone());
        std::thread::spawn(move || {
            let built = build_stream(&want, tx, thread_stop.clone(), thread_level);
            match built {
                Ok((stream, rate)) => {
                    let _ = meta_tx.send(Ok(rate));
                    while !thread_stop.load(Ordering::Relaxed) {
                        std::thread::sleep(Duration::from_millis(15));
                    }
                    drop(stream); // closes the device
                }
                Err(e) => {
                    let _ = meta_tx.send(Err(e));
                }
            }
        });

        // Opening the device is instant except on macOS the first time, where
        // the permission dialog blocks the stream until it is answered. Six
        // seconds is long enough for a device and far too short for a person,
        // so the first dictation would fail with a timeout while the dialog was
        // still on screen — and look like the microphone is broken.
        let open_timeout = if cfg!(target_os = "macos") { 60 } else { 6 };
        let sample_rate = meta_rx
            .recv_timeout(Duration::from_secs(open_timeout))
            .map_err(|_| {
                anyhow!(
                    "the microphone did not open in time — if macOS is asking whether Orra may \
                     use the microphone, answer it and dictate again"
                )
            })??;
        Ok(Capture { rx, sample_rate, stop, level })
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// A running capture, minus its samples.
pub struct CaptureHandle {
    stop: Arc<AtomicBool>,
    level: Arc<AtomicU32>,
}

impl CaptureHandle {
    /// Peak amplitude of the most recent callback, 0.0..=1.0.
    pub fn level(&self) -> f32 {
        f32::from_bits(self.level.load(Ordering::Relaxed))
    }

    /// Stop the microphone. The device thread notices within a poll tick, drops
    /// the stream and closes the channel — so everything it captured is still
    /// readable from `Capture::rx` until then, and nothing already said is lost
    /// to the shutdown.
    pub fn shutdown(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn build_stream(
    want: &str,
    tx: mpsc::UnboundedSender<Vec<i16>>,
    stop: Arc<AtomicBool>,
    level: Arc<AtomicU32>,
) -> Result<(cpal::Stream, u32)> {
    let host = cpal::default_host();

    let device = if want.is_empty() {
        host.default_input_device()
            .ok_or_else(|| anyhow!("no default input device"))?
    } else {
        host.input_devices()
            .map_err(|e| anyhow!("cannot enumerate input devices: {e}"))?
            .find(|d| {
                d.description()
                    .map(|desc| desc.name().to_lowercase().contains(want))
                    .unwrap_or(false)
            })
            .ok_or_else(|| anyhow!("no input device matching {want:?}"))?
    };

    let chosen = pick_config(&device)?;
    let sample_format = chosen.sample_format();
    let sample_rate = chosen.sample_rate();
    let config: cpal::StreamConfig = chosen.into();

    // Shared tail: downmix whatever we were handed to mono i16 and meter it.
    let emit = move |mono: Vec<i16>, peak: f32| {
        level.store(peak.to_bits(), Ordering::Relaxed);
        if tx.send(mono).is_err() {
            // The consumer is gone — recording finished, so let the thread exit.
            stop.store(true, Ordering::Relaxed);
        }
    };

    let on_err = |e| eprintln!("[orra] capture stream error: {e}");
    let ch = config.channels as usize;

    let stream = match sample_format {
        SampleFormat::F32 => {
            device.build_input_stream(
                config,
                move |data: &[f32], _: &_| {
                    let (mono, peak) = downmix_f32(data, ch);
                    emit(mono, peak);
                },
                on_err,
                None,
            )?
        }
        SampleFormat::I16 => {
            device.build_input_stream(
                config,
                move |data: &[i16], _: &_| {
                    let (mono, peak) = downmix_i16(data, ch);
                    emit(mono, peak);
                },
                on_err,
                None,
            )?
        }
        other => {
            return Err(anyhow!(
                "input device uses unsupported sample format {other:?}; pick another mic in Settings"
            ))
        }
    };

    stream.play()?;
    Ok((stream, sample_rate))
}

/// Prefer a device-native config at a Deepgram-friendly rate; otherwise take the
/// device default and let Deepgram resample (it accepts any rate for linear16).
fn pick_config(device: &cpal::Device) -> Result<cpal::SupportedStreamConfig> {
    let supported: Vec<_> = device
        .supported_input_configs()
        .map(|it| it.collect())
        .unwrap_or_default();

    for rate in PREFERRED_RATES {
        let hit = supported.iter().find(|c| {
            c.min_sample_rate() <= rate
                && rate <= c.max_sample_rate()
                && matches!(c.sample_format(), SampleFormat::F32 | SampleFormat::I16)
        });
        if let Some(c) = hit {
            return Ok(c.with_sample_rate(rate));
        }
    }
    Ok(device.default_input_config()?)
}

fn downmix_f32(data: &[f32], channels: usize) -> (Vec<i16>, f32) {
    let mut out = Vec::with_capacity(data.len() / channels.max(1));
    let mut peak = 0.0f32;
    for frame in data.chunks(channels.max(1)) {
        let sum: f32 = frame.iter().sum();
        let v = sum / frame.len() as f32;
        peak = peak.max(v.abs());
        out.push((v.clamp(-1.0, 1.0) * i16::MAX as f32) as i16);
    }
    (out, peak.min(1.0))
}

fn downmix_i16(data: &[i16], channels: usize) -> (Vec<i16>, f32) {
    let mut out = Vec::with_capacity(data.len() / channels.max(1));
    let mut peak = 0.0f32;
    for frame in data.chunks(channels.max(1)) {
        let sum: i32 = frame.iter().map(|s| *s as i32).sum();
        let v = (sum / frame.len() as i32) as i16;
        peak = peak.max(v.unsigned_abs() as f32 / i16::MAX as f32);
        out.push(v);
    }
    (out, peak.min(1.0))
}

/// Names of every input device, for the Settings picker.
pub fn input_devices() -> Vec<String> {
    cpal::default_host()
        .input_devices()
        .map(|it| {
            it.filter_map(|d| d.description().ok().map(|desc| desc.name().to_string()))
                .collect()
        })
        .unwrap_or_default()
}

pub fn i16_to_le_bytes(samples: &[i16]) -> Vec<u8> {
    let mut b = Vec::with_capacity(samples.len() * 2);
    for s in samples {
        b.extend_from_slice(&s.to_le_bytes());
    }
    b
}

/// A 16-bit mono WAV around `samples`.
///
/// 16-bit mono PCM is what every endpoint this app sends audio to accepts —
/// whisper.cpp's server without `--convert` takes nothing else — and the header
/// is 44 bytes of well-known fields, so it is written here rather than pulling
/// in an encoder for it.
pub fn wav_bytes(samples: &[i16], rate: u32) -> Vec<u8> {
    let data = i16_to_le_bytes(samples);
    let mut wav = Vec::with_capacity(44 + data.len());
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&((36 + data.len()) as u32).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes()); // PCM header size
    wav.extend_from_slice(&1u16.to_le_bytes()); // format: PCM
    wav.extend_from_slice(&1u16.to_le_bytes()); // channels
    wav.extend_from_slice(&rate.to_le_bytes());
    wav.extend_from_slice(&(rate * 2).to_le_bytes()); // byte rate
    wav.extend_from_slice(&2u16.to_le_bytes()); // block align
    wav.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&(data.len() as u32).to_le_bytes());
    wav.extend_from_slice(&data);
    wav
}

/// Resample mono samples to `to` hertz.
///
/// Every endpoint that is not a live socket wants a fixed rate — 16 kHz is the
/// one speech models are trained at, and 24 kHz is what the Realtime socket
/// declares — while the microphone runs at whatever the device picked. Doing it
/// here means one resampler instead of a rule at each call site.
///
/// ponytail: linear interpolation, not a windowed-sinc filter. At the ratios
/// that matter here the integer case below is a proper box filter (which is the
/// right anti-aliasing for 48k→16k), and speech recognition is not sensitive to
/// the difference on the non-integer ones.
pub fn resample_to(samples: &[i16], from: u32, to: u32) -> Vec<i16> {
    if samples.is_empty() || from == 0 || to == 0 || from == to {
        return samples.to_vec();
    }
    let step = f64::from(from) / f64::from(to);
    let out_len = ((samples.len() as f64) / step).floor() as usize;
    let mut out = Vec::with_capacity(out_len);

    // An exact integer ratio — 48 kHz to 16 kHz is the one that actually
    // happens — averages the whole window each output sample covers, which
    // removes the frequencies that would otherwise fold back as noise.
    if from.is_multiple_of(to) {
        let k = (from / to) as usize;
        for i in 0..out_len {
            let start = i * k;
            let win = &samples[start..(start + k).min(samples.len())];
            let sum: i32 = win.iter().map(|s| i32::from(*s)).sum();
            out.push((sum / win.len().max(1) as i32) as i16);
        }
        return out;
    }

    // Otherwise interpolate between the two samples the position falls between.
    let last = samples.len() - 1;
    for i in 0..out_len {
        let pos = i as f64 * step;
        let a = pos.floor() as usize;
        let b = (a + 1).min(last);
        let t = pos - a as f64;
        let v = f64::from(samples[a]) * (1.0 - t) + f64::from(samples[b]) * t;
        out.push(v.round().clamp(f64::from(i16::MIN), f64::from(i16::MAX)) as i16);
    }
    out
}

// ---------------------------------------------------------------------------
// TTS playback
// ---------------------------------------------------------------------------

static TTS_STOP: AtomicBool = AtomicBool::new(false);

pub fn stop_speaking() {
    TTS_STOP.store(true, Ordering::Relaxed);
}

/// Play a WAV buffer. Blocking — call it from `spawn_blocking`.
pub fn play_wav(bytes: Vec<u8>) -> Result<()> {
    TTS_STOP.store(false, Ordering::Relaxed);
    let sink = rodio::DeviceSinkBuilder::open_default_sink()
        .map_err(|e| anyhow!("cannot open audio output: {e}"))?;
    let player = rodio::play(sink.mixer(), std::io::Cursor::new(bytes))
        .map_err(|e| anyhow!("cannot decode speech audio: {e}"))?;

    // Poll rather than `sleep_until_end` so barge-in can cut playback short.
    while !player.empty() {
        if TTS_STOP.load(Ordering::Relaxed) {
            player.stop();
            break;
        }
        std::thread::sleep(Duration::from_millis(40));
    }
    drop(sink);
    Ok(())
}

/// Short tone used as start/stop feedback. Fire and forget.
pub fn beep(freq: f32, ms: u64) {
    std::thread::spawn(move || {
        let Ok(sink) = rodio::DeviceSinkBuilder::open_default_sink() else { return };
        let Ok(player) = rodio::play(sink.mixer(), std::io::Cursor::new(tone_wav(freq, ms))) else {
            return;
        };
        player.sleep_until_end();
        drop(sink);
    });
}

/// Minimal 16-bit mono WAV around a sine tone, with an envelope so it does not click.
fn tone_wav(freq: f32, ms: u64) -> Vec<u8> {
    let rate = 44_100u32;
    let n = (rate as u64 * ms / 1000).max(1) as usize;
    let attack = (0.008 * rate as f32).max(1.0);
    let release = (0.030 * rate as f32).max(1.0);

    let mut samples = Vec::with_capacity(n);
    for i in 0..n {
        let level = (i as f32 / attack).min(1.0).min((n - i) as f32 / release);
        let wave = (i as f32 / rate as f32 * freq * std::f32::consts::TAU).sin();
        samples.push((wave * level * 0.22 * i16::MAX as f32) as i16);
    }

    wav_bytes(&samples, rate)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tone_wav_has_a_well_formed_header() {
        let wav = tone_wav(440.0, 80);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(&wav[36..40], b"data");
        // 80 ms at 44.1 kHz, 16-bit mono.
        let expected = (44_100 * 80 / 1000) * 2;
        assert_eq!(u32::from_le_bytes([wav[40], wav[41], wav[42], wav[43]]) as usize, expected);
        assert_eq!(wav.len(), 44 + expected);
    }

    #[test]
    fn stereo_downmix_averages_and_meters() {
        // 2 frames, 2 channels: (1000, 3000) and (-1000, -3000)
        let (mono, peak) = downmix_i16(&[1000, 3000, -1000, -3000], 2);
        assert_eq!(mono, vec![2000, -2000]);
        assert!((peak - 2000.0 / i16::MAX as f32).abs() < 1e-6);
    }

    #[test]
    fn mono_passthrough_is_lossless() {
        let (mono, _) = downmix_i16(&[5, -5, 32767], 1);
        assert_eq!(mono, vec![5, -5, 32767]);
    }

    #[test]
    fn pcm_encoding_is_little_endian() {
        assert_eq!(i16_to_le_bytes(&[1, -1]), vec![0x01, 0x00, 0xFF, 0xFF]);
    }

    #[test]
    fn float_samples_clamp_instead_of_wrapping() {
        let (mono, peak) = downmix_f32(&[1.8, -1.8], 1);
        assert_eq!(mono, vec![i16::MAX, -i16::MAX]);
        assert!(peak <= 1.0);
    }

    #[test]
    fn a_wav_header_describes_the_samples_it_carries() {
        let samples = vec![0i16, 1000, -1000, 32767];
        let wav = wav_bytes(&samples, 16_000);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(&wav[36..40], b"data");
        // Mono, 16 kHz, 16-bit: the three fields a server reads to decide
        // whether it can take the audio at all.
        assert_eq!(u16::from_le_bytes([wav[22], wav[23]]), 1);
        assert_eq!(u32::from_le_bytes([wav[24], wav[25], wav[26], wav[27]]), 16_000);
        assert_eq!(u16::from_le_bytes([wav[34], wav[35]]), 16);
        let body = u32::from_le_bytes([wav[40], wav[41], wav[42], wav[43]]) as usize;
        assert_eq!(body, samples.len() * 2);
        assert_eq!(wav.len(), 44 + body);
        // The samples themselves survive, little-endian.
        assert_eq!(&wav[44..48], &[0x00, 0x00, 0xE8, 0x03]);
    }

    #[test]
    fn resampling_to_the_same_rate_changes_nothing() {
        let samples: Vec<i16> = (0..100).map(|i| (i * 13) as i16).collect();
        assert_eq!(resample_to(&samples, 16_000, 16_000), samples);
        // Nothing to resample, and nothing to divide by.
        assert_eq!(resample_to(&[], 48_000, 16_000), Vec::<i16>::new());
    }

    /// The case that actually happens: a device running at 48 kHz and an
    /// endpoint that wants 16 kHz.
    #[test]
    fn a_third_of_the_samples_come_back_and_the_signal_survives() {
        let silence = vec![0i16; 48_000];
        let out = resample_to(&silence, 48_000, 16_000);
        assert_eq!(out.len(), 16_000);
        assert!(out.iter().all(|s| *s == 0), "silence must stay silent");

        // A steady level is unchanged by averaging, which is what says the
        // decimation is a filter rather than a drop-every-third-sample.
        let steady = vec![1_000i16; 48_000];
        let out = resample_to(&steady, 48_000, 16_000);
        assert!(out.iter().all(|s| (*s - 1_000).abs() <= 1), "level moved: {:?}", &out[..4]);

        // A second of audio is a second of audio whatever the rate, including
        // on the non-integer ratio the Realtime socket needs.
        let tone: Vec<i16> = (0..16_000)
            .map(|i| ((i as f32 / 16_000.0 * 440.0 * std::f32::consts::TAU).sin() * 10_000.0) as i16)
            .collect();
        let upsampled = resample_to(&tone, 16_000, 24_000);
        assert_eq!(upsampled.len(), 24_000);
        assert!(upsampled.iter().any(|s| s.abs() > 5_000), "the tone was flattened");
    }
}
