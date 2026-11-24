use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use anyhow::Result;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, Stream};
use tracing::{info, warn};

pub struct AudioDeviceManager {
    is_muted: Arc<AtomicBool>,
    _input_stream: Option<Stream>,
    _output_stream: Option<Stream>,
    output_buffer: Arc<Mutex<VecDeque<f32>>>,
    input_buffer: Arc<Mutex<VecDeque<i16>>>,
    input_primed: AtomicBool,
}

impl std::fmt::Debug for AudioDeviceManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioDeviceManager")
            .field("is_muted", &self.is_muted)
            .finish()
    }
}

// Safety: cpal::Stream handles audio device callbacks. The stream handles can be safely held and dropped across threads.
unsafe impl Send for AudioDeviceManager {}
unsafe impl Sync for AudioDeviceManager {}


#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioDeviceInfo {
    pub id: String,
    pub description: String,
}

impl AudioDeviceManager {
    /// Lists available local audio output devices (PipeWire / PulseAudio sinks with CPAL fallback).
    /// Excludes network AirPlay (`raop_sink.`) devices to keep the list focused on local hardware.
    pub fn list_output_devices() -> Vec<AudioDeviceInfo> {
        let mut devices = vec![AudioDeviceInfo {
            id: "default".to_string(),
            description: "System Default".to_string(),
        }];

        if let Ok(output) = std::process::Command::new("pactl")
            .args(["list", "sinks"])
            .env("LC_ALL", "C")
            .output()
        {
            if output.status.success() {
                let text = String::from_utf8_lossy(&output.stdout);
                let mut current_name: Option<String> = None;
                for line in text.lines() {
                    let trimmed = line.trim();
                    if let Some(rest) = trimmed.strip_prefix("Name:") {
                        current_name = Some(rest.trim().to_string());
                    } else if let Some(rest) = trimmed.strip_prefix("Description:") {
                        if let Some(name) = current_name.take() {
                            if !name.starts_with("raop_sink.") {
                                devices.push(AudioDeviceInfo {
                                    id: name,
                                    description: rest.trim().to_string(),
                                });
                            }
                        }
                    }
                }
            }
        }

        if devices.len() == 1 {
            let host = cpal::default_host();
            if let Ok(out_devs) = host.output_devices() {
                for d in out_devs {
                    if let Ok(name) = d.name() {
                        if name != "default" && !name.starts_with("null") {
                            devices.push(AudioDeviceInfo {
                                id: name.clone(),
                                description: name,
                            });
                        }
                    }
                }
            }
        }

        devices
    }

    /// Lists available local microphone input devices (PipeWire / PulseAudio sources with CPAL fallback).
    /// Excludes `.monitor` loopback sources and network AirPlay (`raop_sink.`) devices.
    pub fn list_input_devices() -> Vec<AudioDeviceInfo> {
        let mut devices = vec![AudioDeviceInfo {
            id: "default".to_string(),
            description: "System Default".to_string(),
        }];

        if let Ok(output) = std::process::Command::new("pactl")
            .args(["list", "sources"])
            .env("LC_ALL", "C")
            .output()
        {
            if output.status.success() {
                let text = String::from_utf8_lossy(&output.stdout);
                let mut current_name: Option<String> = None;
                for line in text.lines() {
                    let trimmed = line.trim();
                    if let Some(rest) = trimmed.strip_prefix("Name:") {
                        current_name = Some(rest.trim().to_string());
                    } else if let Some(rest) = trimmed.strip_prefix("Description:") {
                        if let Some(name) = current_name.take() {
                            if !name.ends_with(".monitor") && !name.starts_with("raop_sink.") {
                                devices.push(AudioDeviceInfo {
                                    id: name,
                                    description: rest.trim().to_string(),
                                });
                            }
                        }
                    }
                }
            }
        }

        if devices.len() == 1 {
            let host = cpal::default_host();
            if let Ok(in_devs) = host.input_devices() {
                for d in in_devs {
                    if let Ok(name) = d.name() {
                        if name != "default" && !name.starts_with("null") {
                            devices.push(AudioDeviceInfo {
                                id: name.clone(),
                                description: name,
                            });
                        }
                    }
                }
            }
        }

        devices
    }

    /// Attempts to initialize the default audio input and output devices.
    /// Returns None if no audio hardware is available (e.g. headless container).
    pub fn try_init() -> Option<Self> {
        let host = cpal::default_host();
        let in_dev = host.default_input_device();
        let out_dev = host.default_output_device();

        if in_dev.is_none() && out_dev.is_none() {
            warn!("No audio hardware devices found (headless mode)");
            return None;
        }

        info!(
            "Audio hardware detected: Input={:?}, Output={:?}",
            in_dev.as_ref().and_then(|d| d.name().ok()),
            out_dev.as_ref().and_then(|d| d.name().ok())
        );

        let mut mgr = Self {
            is_muted: Arc::new(AtomicBool::new(false)),
            _input_stream: None,
            _output_stream: None,
            output_buffer: Arc::new(Mutex::new(VecDeque::with_capacity(16000))),
            input_buffer: Arc::new(Mutex::new(VecDeque::with_capacity(16000))),
            input_primed: AtomicBool::new(false),
        };

        // Try starting playback and recording streams
        if let Err(e) = mgr.start_playback() {
            warn!("Could not start audio playback stream: {}", e);
        }
        if let Err(e) = mgr.start_recording() {
            warn!("Could not start microphone recording stream: {}", e);
        }

        Some(mgr)
    }

    /// Sets microphone mute state
    pub fn set_muted(&self, muted: bool) {
        self.is_muted.store(muted, Ordering::SeqCst);
    }

    pub fn is_muted(&self) -> bool {
        self.is_muted.load(Ordering::SeqCst)
    }

    /// Feeds incoming decoded 8 kHz PCM audio (from RTP) into the speaker playback buffer
    pub fn play_pcm_samples(&self, samples: &[i16]) {
        if let Ok(mut buf) = self.output_buffer.lock() {
            // Latenz begrenzen: mehr als 200 ms im Puffer -> auf 100 ms kürzen
            if buf.len() > MAX_BUFFER_SAMPLES {
                let excess = buf.len() - TRIM_TARGET_SAMPLES;
                buf.drain(..excess);
            }
            for &s in samples {
                buf.push_back(s as f32 / 32768.0);
            }
        }
    }

    /// Drains `count` samples (8 kHz) of recorded microphone audio for transmission via RTP.
    /// Returns `None` while the jitter buffer is still filling (caller sends silence).
    pub fn read_mic_samples(&self, count: usize) -> Option<Vec<i16>> {
        if self.is_muted.load(Ordering::SeqCst) {
            return Some(vec![0i16; count]);
        }

        let mut buf = self.input_buffer.lock().ok()?;
        if !self.input_primed.load(Ordering::SeqCst) {
            if buf.len() < count * 2 {
                return None;
            }
            self.input_primed.store(true, Ordering::SeqCst);
        }
        if buf.len() >= count {
            return Some(buf.drain(..count).collect());
        }
        // Unterlauf: Rest auffüllen und Puffer neu vorfüllen lassen
        let mut samples: Vec<i16> = buf.drain(..).collect();
        samples.resize(count, 0);
        self.input_primed.store(false, Ordering::SeqCst);
        Some(samples)
    }

    /// Starts speaker playback stream using CPAL (resampling 8 kHz -> device rate)
    pub fn start_playback(&mut self) -> Result<()> {
        let host = cpal::default_host();
        let device = match host.default_output_device() {
            Some(d) => d,
            None => return Ok(()),
        };

        let supported = device.default_output_config()?;
        let format = supported.sample_format();
        let config: cpal::StreamConfig = supported.into();
        info!(
            "Speaker: {:?} {} Hz, {} Kanal/Kanäle, {:?}",
            device.name().ok(),
            config.sample_rate.0,
            config.channels,
            format
        );

        let stream = match format {
            SampleFormat::F32 => build_output::<f32>(&device, &config, self.output_buffer.clone())?,
            SampleFormat::I16 => build_output::<i16>(&device, &config, self.output_buffer.clone())?,
            SampleFormat::U16 => build_output::<u16>(&device, &config, self.output_buffer.clone())?,
            SampleFormat::I32 => build_output::<i32>(&device, &config, self.output_buffer.clone())?,
            SampleFormat::F64 => build_output::<f64>(&device, &config, self.output_buffer.clone())?,
            other => {
                warn!("Nicht unterstütztes Ausgabe-Sampleformat {:?}", other);
                return Ok(());
            }
        };

        stream.play()?;
        self._output_stream = Some(stream);
        info!("Speaker playback stream started");
        Ok(())
    }

    /// Starts microphone recording stream using CPAL (resampling device rate -> 8 kHz)
    pub fn start_recording(&mut self) -> Result<()> {
        let host = cpal::default_host();
        let device = match host.default_input_device() {
            Some(d) => d,
            None => return Ok(()),
        };

        let supported = device.default_input_config()?;
        let format = supported.sample_format();
        let config: cpal::StreamConfig = supported.into();
        info!(
            "Mikrofon: {:?} {} Hz, {} Kanal/Kanäle, {:?}",
            device.name().ok(),
            config.sample_rate.0,
            config.channels,
            format
        );

        let buffer = self.input_buffer.clone();
        let muted = self.is_muted.clone();
        let stream = match format {
            SampleFormat::F32 => build_input::<f32>(&device, &config, buffer, muted)?,
            SampleFormat::I16 => build_input::<i16>(&device, &config, buffer, muted)?,
            SampleFormat::U16 => build_input::<u16>(&device, &config, buffer, muted)?,
            SampleFormat::I32 => build_input::<i32>(&device, &config, buffer, muted)?,
            SampleFormat::F64 => build_input::<f64>(&device, &config, buffer, muted)?,
            other => {
                warn!("Nicht unterstütztes Eingabe-Sampleformat {:?}", other);
                return Ok(());
            }
        };

        stream.play()?;
        self._input_stream = Some(stream);
        info!("Microphone recording stream started");
        Ok(())
    }
}

/// Telephony sample rate (G.711)
const TELEPHONY_RATE: f64 = 8000.0;
/// Jitter buffer pre-fill before playback starts (60 ms @ 8 kHz)
const PLAYBACK_PREFILL_SAMPLES: usize = 480;
/// Upper bound of buffered audio (200 ms @ 8 kHz)
const MAX_BUFFER_SAMPLES: usize = 1600;
/// Buffer level after trimming (100 ms @ 8 kHz)
const TRIM_TARGET_SAMPLES: usize = 800;

fn build_output<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    buffer: Arc<Mutex<VecDeque<f32>>>,
) -> Result<Stream>
where
    T: cpal::SizedSample + cpal::FromSample<f32>,
{
    let channels = config.channels.max(1) as usize;
    let step = TELEPHONY_RATE / config.sample_rate.0.max(1) as f64;

    // Linearer Interpolations-Resampler 8 kHz -> Geräterate
    let mut primed = false;
    let mut pos = 0.0f64;
    let mut cur = 0.0f32;
    let mut next = 0.0f32;

    let stream = device.build_output_stream(
        config,
        move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
            let mut buf = match buffer.lock() {
                Ok(g) => g,
                Err(_) => {
                    for s in data.iter_mut() {
                        *s = T::from_sample(0.0f32);
                    }
                    return;
                }
            };
            for frame in data.chunks_mut(channels) {
                let value = if primed {
                    let v = cur + (next - cur) * pos as f32;
                    pos += step;
                    while pos >= 1.0 {
                        pos -= 1.0;
                        cur = next;
                        next = match buf.pop_front() {
                            Some(s) => s,
                            None => {
                                primed = false;
                                0.0
                            }
                        };
                    }
                    v
                } else {
                    if buf.len() >= PLAYBACK_PREFILL_SAMPLES {
                        primed = true;
                        pos = 0.0;
                        next = buf.pop_front().unwrap_or(0.0);
                    }
                    0.0
                };
                let out = T::from_sample(value.clamp(-1.0, 1.0));
                for s in frame.iter_mut() {
                    *s = out;
                }
            }
        },
        |err| warn!("Playback stream error: {}", err),
        None,
    )?;
    Ok(stream)
}

impl AudioDeviceManager {
    /// Plays a gentle, pleasant built-in dual-tone phone chime directly via CPAL.
    /// Works inside Flatpak sandboxes and across all Linux desktops without external
    /// sound files or CLI tools (`pw-play`/`paplay`).
    pub fn play_ringtone_burst(
        keep_playing: Option<&Arc<AtomicBool>>,
        volume: f32,
    ) -> bool {
        // Two gentle rising chimes (C5 + E5, then E5 + G5) with soft exponential decay
        let segments: &[(f64, f64, u64, f32)] = &[
            (523.25, 659.25, 220, 1.0),
            (0.0, 0.0, 70, 0.0),
            (659.25, 783.99, 280, 1.0),
            (0.0, 0.0, 120, 0.0),
            (523.25, 659.25, 220, 1.0),
            (0.0, 0.0, 70, 0.0),
            (659.25, 783.99, 320, 1.0),
        ];
        play_synthesized_tone_sequence(segments, volume.clamp(0.02, 0.45), keep_playing)
    }

    /// Plays a short, quiet call-waiting double beep directly via CPAL.
    pub fn play_call_waiting_tone(volume: f32) -> bool {
        let segments: &[(f64, f64, u64, f32)] = &[
            (425.0, 0.0, 120, 1.0),
            (0.0, 0.0, 90, 0.0),
            (425.0, 0.0, 120, 1.0),
        ];
        play_synthesized_tone_sequence(segments, volume.clamp(0.02, 0.25), None)
    }
}

fn play_synthesized_tone_sequence(
    segments: &[(f64, f64, u64, f32)],
    master_volume: f32,
    keep_playing: Option<&Arc<AtomicBool>>,
) -> bool {
    let host = cpal::default_host();
    let Some(device) = host.default_output_device() else {
        return false;
    };
    let Ok(supported) = device.default_output_config() else {
        return false;
    };
    let format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();
    let sample_rate = config.sample_rate.0.max(8000) as f64;

    // Pre-synthesize the mono f32 waveform with smooth 15 ms fade-in/out envelopes (zero clicks)
    let mut mono_samples: Vec<f32> = Vec::new();
    let fade_samples = (sample_rate * 0.015) as usize;
    for &(f1, f2, ms, gain) in segments {
        let count = ((sample_rate * (ms as f64 / 1000.0)) as usize).max(1);
        for i in 0..count {
            let s = if gain <= 0.0 || f1 <= 0.0 {
                0.0
            } else {
                let t = i as f64 / sample_rate;
                let wave = if f2 > 0.0 {
                    0.5 * (2.0 * std::f64::consts::PI * f1 * t).sin()
                        + 0.5 * (2.0 * std::f64::consts::PI * f2 * t).sin()
                } else {
                    (2.0 * std::f64::consts::PI * f1 * t).sin()
                };
                let env_in = if i < fade_samples {
                    i as f32 / fade_samples.max(1) as f32
                } else {
                    1.0
                };
                let env_out = if i + fade_samples > count {
                    (count - i) as f32 / fade_samples.max(1) as f32
                } else {
                    1.0
                };
                let decay = (-2.2 * t).exp() as f32;
                (wave as f32) * gain * master_volume * env_in * env_out * decay
            };
            mono_samples.push(s);
        }
    }

    let total_frames = mono_samples.len();
    let total_ms = ((total_frames as f64 / sample_rate) * 1000.0).ceil() as u64;
    let cursor = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let samples_arc = Arc::new(mono_samples);

    let stream = match format {
        SampleFormat::F32 => build_tone_stream::<f32>(&device, &config, samples_arc, cursor.clone()),
        SampleFormat::I16 => build_tone_stream::<i16>(&device, &config, samples_arc, cursor.clone()),
        SampleFormat::U16 => build_tone_stream::<u16>(&device, &config, samples_arc, cursor.clone()),
        _ => return false,
    };

    let Ok(stream) = stream else {
        return false;
    };
    if stream.play().is_err() {
        return false;
    }

    let mut elapsed = 0u64;
    while elapsed < total_ms + 80 {
        if let Some(flag) = keep_playing {
            if !flag.load(Ordering::SeqCst) {
                break;
            }
        }
        if cursor.load(Ordering::Relaxed) >= total_frames {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
        elapsed += 25;
    }
    drop(stream);
    true
}

fn build_tone_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    samples: Arc<Vec<f32>>,
    cursor: Arc<std::sync::atomic::AtomicUsize>,
) -> Result<Stream>
where
    T: cpal::SizedSample + cpal::FromSample<f32>,
{
    let channels = config.channels.max(1) as usize;
    let stream = device.build_output_stream(
        config,
        move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
            let mut pos = cursor.load(Ordering::Relaxed);
            for frame in data.chunks_mut(channels) {
                let val = if pos < samples.len() {
                    let s = samples[pos];
                    pos += 1;
                    T::from_sample(s)
                } else {
                    T::from_sample(0.0f32)
                };
                for ch in frame.iter_mut() {
                    *ch = val;
                }
            }
            cursor.store(pos, Ordering::Relaxed);
        },
        |err| warn!("Ringtone stream error: {}", err),
        None,
    )?;
    Ok(stream)
}

fn build_input<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    buffer: Arc<Mutex<VecDeque<i16>>>,
    is_muted: Arc<AtomicBool>,
) -> Result<Stream>
where
    T: cpal::SizedSample,
    f32: cpal::FromSample<T>,
{
    let channels = config.channels.max(1) as usize;
    let ratio = TELEPHONY_RATE / config.sample_rate.0.max(1) as f64;

    // Mittelwert-Dezimierung Geräterate -> 8 kHz (einfacher Anti-Aliasing-Tiefpass)
    let mut phase = 0.0f64;
    let mut acc = 0.0f32;
    let mut n = 0u32;

    let stream = device.build_input_stream(
        config,
        move |data: &[T], _: &cpal::InputCallbackInfo| {
            if is_muted.load(Ordering::SeqCst) {
                return;
            }
            let mut out: Vec<i16> = Vec::with_capacity(data.len() / channels / 4 + 2);
            for frame in data.chunks(channels) {
                let mono: f32 =
                    frame.iter().map(|&s| <f32 as cpal::Sample>::from_sample(s)).sum::<f32>() / frame.len() as f32;
                acc += mono;
                n += 1;
                phase += ratio;
                if phase >= 1.0 {
                    phase -= 1.0;
                    let avg = acc / n as f32;
                    out.push((avg.clamp(-1.0, 1.0) * 32767.0) as i16);
                    acc = 0.0;
                    n = 0;
                }
            }
            if let Ok(mut buf) = buffer.lock() {
                buf.extend(out);
                if buf.len() > MAX_BUFFER_SAMPLES {
                    let excess = buf.len() - TRIM_TARGET_SAMPLES;
                    buf.drain(..excess);
                }
            }
        },
        |err| warn!("Recording stream error: {}", err),
        None,
    )?;
    Ok(stream)
}
