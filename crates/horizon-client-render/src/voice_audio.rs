//! Voice chat's sound: the microphone, cut into 20 ms frames of 48 kHz mono, and a playback
//! stream that mixes what other players say (crates/horizon-client/src/voicechat.rs).
//! Its own streams, beside the game's mixer, so voice never waits on game sound.
use std::collections::{HashMap, VecDeque};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};

/// 48 kHz: Opus's rate, and the frames' and the playback queues'.
pub const RATE: u32 = 48_000;
/// 20 ms at 48 kHz.
pub const FRAME: usize = 960;
/// At most this much of one speaker waits to play (200 ms); older sound is dropped, so a late
/// burst doesn't keep a speaker behind.
const MAX_QUEUED: usize = RATE as usize / 5;

/// What each speaker has left to play, at 48 kHz.
#[derive(Clone, Default)]
pub struct Playback(Arc<Mutex<HashMap<u128, VecDeque<f32>>>>);

impl Playback {
    /// Queue `pcm` (48 kHz mono) from `speaker`, scaled by `gain`.
    pub fn push(&self, speaker: u128, pcm: &[i16], gain: f32) {
        let Ok(mut queues) = self.0.lock() else {
            return;
        };
        let queue = queues.entry(speaker).or_default();
        queue.extend(pcm.iter().map(|&s| f32::from(s) / 32768.0 * gain));
        let excess = queue.len().saturating_sub(MAX_QUEUED);
        queue.drain(..excess);
    }

    /// Mix the next `out.len() / channels` frames at `rate`: every speaker summed, resampled
    /// linearly from 48 kHz, the same on every channel.
    #[cfg_attr(not(feature = "audio"), allow(dead_code))]
    fn mix(&self, out: &mut [f32], channels: usize, rate: u32, phase: &mut f64) {
        let step = f64::from(RATE) / f64::from(rate.max(1));
        let Ok(mut queues) = self.0.lock() else {
            out.fill(0.0);
            return;
        };
        for frame in out.chunks_mut(channels.max(1)) {
            let t = *phase as f32;
            let mut sample = 0.0;
            for q in queues.values() {
                let a = q.front().copied().unwrap_or(0.0);
                let b = q.get(1).copied().unwrap_or(a);
                sample += a + (b - a) * t;
            }
            frame.fill(sample.clamp(-1.0, 1.0));
            *phase += step;
            while *phase >= 1.0 {
                *phase -= 1.0;
                for q in queues.values_mut() {
                    q.pop_front();
                }
            }
        }
        queues.retain(|_, q| !q.is_empty());
    }
}

/// Cuts a device's input into 48 kHz mono frames.
#[derive(Default)]
#[cfg_attr(not(feature = "audio"), allow(dead_code))]
struct Framer {
    phase: f64,
    last: f32,
    frame: Vec<i16>,
}

#[cfg_attr(not(feature = "audio"), allow(dead_code))]
impl Framer {
    /// Interleaved `input` at `rate` with `channels`: the whole frames it completes.
    fn feed(&mut self, input: &[f32], channels: usize, rate: u32) -> Vec<Vec<i16>> {
        let step = f64::from(rate) / f64::from(RATE);
        let mut done = Vec::new();
        for frame in input.chunks(channels.max(1)) {
            let mono = frame.iter().sum::<f32>() / frame.len() as f32;
            // Each 48 kHz sample falls between the last input sample and this one.
            while self.phase < 1.0 {
                let s = self.last + (mono - self.last) * self.phase as f32;
                self.frame.push((s.clamp(-1.0, 1.0) * 32767.0) as i16);
                if self.frame.len() == FRAME {
                    done.push(std::mem::replace(
                        &mut self.frame,
                        Vec::with_capacity(FRAME),
                    ));
                }
                self.phase += step;
            }
            self.phase -= 1.0;
            self.last = mono;
        }
        done
    }
}

/// The open microphone and speaker streams; both stop when dropped.
pub struct VoiceAudio {
    pub playback: Playback,
    /// The microphone's frames, None without one.
    pub frames: Option<Receiver<Vec<i16>>>,
    #[cfg(feature = "audio")]
    _streams: Vec<cpal::Stream>,
}

impl VoiceAudio {
    #[cfg(not(feature = "audio"))]
    pub fn start() -> Result<Self, String> {
        Err("this build has no sound".into())
    }

    /// Open the default output, and the default input when there is one.
    #[cfg(feature = "audio")]
    pub fn start() -> Result<Self, String> {
        use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
        if let Some(why) = crate::audio::output_disabled(|n| std::env::var_os(n)) {
            return Err(why);
        }
        let host = cpal::default_host();
        let playback = Playback::default();
        let mut streams = Vec::new();
        let output = host.default_output_device().ok_or("no sound output")?;
        let config = output.default_output_config().map_err(|e| e.to_string())?;
        let (rate, channels) = (config.sample_rate().0, usize::from(config.channels()));
        let mix = playback.clone();
        let mut phase = 0.0;
        let mut scratch = Vec::new();
        let stream = output
            .build_output_stream(
                &config.config(),
                move |data: &mut [f32], _| {
                    scratch.resize(data.len(), 0.0);
                    mix.mix(&mut scratch, channels, rate, &mut phase);
                    data.copy_from_slice(&scratch);
                },
                |e| log::warn!("Heisen: voice output: {e}"),
                None,
            )
            .map_err(|e| format!("voice output: {e}"))?;
        stream.play().map_err(|e| e.to_string())?;
        streams.push(stream);
        let mut frames = None;
        match host
            .default_input_device()
            .map(|d| (d.default_input_config(), d))
        {
            Some((Ok(config), input)) => {
                let (rate, channels) = (config.sample_rate().0, usize::from(config.channels()));
                let (tx, rx) = std::sync::mpsc::channel();
                let mut framer = Framer::default();
                let error = |e| log::warn!("Heisen: microphone: {e}");
                let stream = match config.sample_format() {
                    cpal::SampleFormat::I16 => {
                        let mut scratch = Vec::new();
                        input.build_input_stream(
                            &config.config(),
                            move |data: &[i16], _| {
                                scratch.clear();
                                scratch.extend(data.iter().map(|&s| f32::from(s) / 32768.0));
                                for frame in framer.feed(&scratch, channels, rate) {
                                    let _ = tx.send(frame);
                                }
                            },
                            error,
                            None,
                        )
                    }
                    _ => input.build_input_stream(
                        &config.config(),
                        move |data: &[f32], _| {
                            for frame in framer.feed(data, channels, rate) {
                                let _ = tx.send(frame);
                            }
                        },
                        error,
                        None,
                    ),
                };
                match stream
                    .map_err(|e| e.to_string())
                    .and_then(|s| s.play().map(|()| s).map_err(|e| e.to_string()))
                {
                    Ok(stream) => {
                        streams.push(stream);
                        frames = Some(rx);
                    }
                    Err(e) => log::warn!("Heisen: no microphone for voice chat: {e}"),
                }
            }
            Some((Err(e), _)) => log::warn!("Heisen: no microphone for voice chat: {e}"),
            None => log::info!("Heisen: no microphone; voice chat only listens"),
        }
        Ok(Self {
            playback,
            frames,
            _streams: streams,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn framer_cuts_48_khz_frames_from_other_rates() {
        let mut f = Framer::default();
        // One second of stereo at 44.1 kHz: 50 frames of 20 ms, give or take the last.
        let input = vec![0.5f32; 44_100 * 2];
        let frames = f.feed(&input, 2, 44_100);
        assert!((49..=50).contains(&frames.len()), "{}", frames.len());
        assert!(frames.iter().all(|f| f.len() == FRAME));
        assert!(frames[10].iter().all(|&s| (s - 16383).abs() <= 1));
    }

    #[test]
    fn playback_mixes_speakers_and_drains_them() {
        let p = Playback::default();
        p.push(1, &[16384; 480], 1.0);
        p.push(2, &[8192; 480], 1.0);
        let mut out = vec![0.0; 960];
        let mut phase = 0.0;
        p.mix(&mut out[..480], 1, RATE, &mut phase);
        assert!((out[100] - 0.75).abs() < 1e-3);
        p.mix(&mut out[480..], 1, RATE, &mut phase);
        assert_eq!(out[900], 0.0);
        assert!(p.0.lock().unwrap().is_empty());
    }
}
