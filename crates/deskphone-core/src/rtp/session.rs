use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use anyhow::{anyhow, Result};
use bytes::Bytes;
use tokio::net::UdpSocket;
use tokio::sync::watch;
use tokio::time::{sleep, MissedTickBehavior};
use tracing::{debug, info, warn};

use super::audio_device::AudioDeviceManager;
use super::codec::G711;
use super::packet::RtpPacket;

/// 20 ms G.711 frame at 8 kHz
const FRAME_SAMPLES: usize = 160;

#[derive(Clone, Debug)]
pub struct RtpSession {
    pub local_port: u16,
    socket: Arc<UdpSocket>,
    remote_addr: Arc<Mutex<Option<SocketAddr>>>,
    is_running: Arc<AtomicBool>,
    is_paused: Arc<AtomicBool>,
    ssrc: u32,
    payload_type: Arc<AtomicU8>,
    dtmf_payload_type: Arc<AtomicU8>,
    audio_device: Arc<Mutex<Option<Arc<AudioDeviceManager>>>>,
    shutdown: Arc<watch::Sender<bool>>,
    muted: Arc<AtomicBool>,
}

impl RtpSession {
    /// Binds an RTP UDP socket on an available (even) port WITHOUT a remote target yet.
    /// The local port can be placed into an SDP offer; the remote endpoint is set later
    /// via [`RtpSession::set_remote`] once the SDP answer arrives.
    /// Audio hardware is only opened when [`RtpSession::start`] is called.
    pub async fn bind() -> Result<Self> {
        let mut port = 10000;
        let socket = loop {
            let addr = format!("0.0.0.0:{}", port);
            match UdpSocket::bind(&addr).await {
                Ok(s) => break s,
                Err(_) => {
                    port += 2;
                    if port > 20000 {
                        return Err(anyhow!("No available RTP ports in range"));
                    }
                }
            }
        };
        info!("RTP session bound to local port {}", port);
        let (shutdown, _) = watch::channel(false);

        Ok(Self {
            local_port: port,
            socket: Arc::new(socket),
            remote_addr: Arc::new(Mutex::new(None)),
            is_running: Arc::new(AtomicBool::new(false)),
            is_paused: Arc::new(AtomicBool::new(false)),
            ssrc: rand::random::<u32>(),
            payload_type: Arc::new(AtomicU8::new(8)),
            dtmf_payload_type: Arc::new(AtomicU8::new(super::super::sip::sdp::DEFAULT_DTMF_PT)),
            audio_device: Arc::new(Mutex::new(None)),
            shutdown: Arc::new(shutdown),
            muted: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Binds an RTP UDP socket on an available port and targets the remote endpoint.
    pub async fn new(remote_addr: SocketAddr, payload_type: u8) -> Result<Self> {
        let s = Self::bind().await?;
        s.set_remote(remote_addr, payload_type);
        Ok(s)
    }

    /// Binds an RTP UDP socket and attaches an optional AudioDeviceManager for live mic and speaker audio
    pub async fn new_with_audio(
        remote_addr: SocketAddr,
        payload_type: u8,
        audio_device: Option<Arc<AudioDeviceManager>>,
    ) -> Result<Self> {
        let s = Self::new(remote_addr, payload_type).await?;
        if let Ok(mut dev) = s.audio_device.lock() {
            *dev = audio_device;
        }
        Ok(s)
    }

    /// Sets / updates the remote RTP endpoint and payload type (SDP answer, re-INVITE, ...)
    pub fn set_remote(&self, remote_addr: SocketAddr, payload_type: u8) {
        if let Ok(mut r) = self.remote_addr.lock() {
            if *r != Some(remote_addr) {
                info!("RTP port {} -> remote {} (PT {})", self.local_port, remote_addr, payload_type);
            }
            *r = Some(remote_addr);
        }
        self.payload_type.store(payload_type, Ordering::SeqCst);
    }

    /// Sets the negotiated RFC 4733 telephone-event payload type
    pub fn set_dtmf_payload_type(&self, pt: u8) {
        self.dtmf_payload_type.store(pt, Ordering::SeqCst);
    }

    pub fn remote_addr(&self) -> Option<SocketAddr> {
        self.remote_addr.lock().ok().and_then(|r| *r)
    }

    pub fn payload_type(&self) -> u8 {
        self.payload_type.load(Ordering::SeqCst)
    }

    /// Starts streaming audio (live microphone) and receiving remote audio (playback to speakers).
    /// Calling `start` more than once is a no-op.
    pub fn start(&self) {
        if self.is_running.swap(true, Ordering::SeqCst) {
            return;
        }

        // Audio-Hardware erst jetzt öffnen (nicht schon beim Port-Reservieren)
        let audio = {
            let mut guard = self.audio_device.lock().unwrap_or_else(|e| e.into_inner());
            if guard.is_none() {
                // In Tests (oder per DESKPHONE_NO_AUDIO=1) keine echte Audio-Hardware öffnen
                let audio_disabled = cfg!(test) || std::env::var_os("DESKPHONE_NO_AUDIO").is_some();
                if !audio_disabled {
                    *guard = AudioDeviceManager::try_init().map(Arc::new);
                    if guard.is_none() {
                        warn!("Keine Audio-Hardware verfügbar - Gespräch läuft ohne Ton");
                    }
                }
            }
            guard.clone()
        };
        if let Some(ref dev) = audio {
            dev.set_muted(self.muted.load(Ordering::SeqCst));
        }

        // Receiver Task
        let rx_socket = self.socket.clone();
        let rx_audio = audio.clone();
        let mut rx_shutdown = self.shutdown.subscribe();
        let local_port = self.local_port;

        tokio::spawn(async move {
            let mut buf = vec![0u8; 2048];
            let mut first_packet = true;
            loop {
                tokio::select! {
                    _ = rx_shutdown.changed() => break,
                    res = rx_socket.recv_from(&mut buf) => {
                        let (len, src) = match res {
                            Ok(r) => r,
                            Err(e) => {
                                debug!("RTP recv error on port {}: {}", local_port, e);
                                continue;
                            }
                        };
                        let Ok(pkt) = RtpPacket::parse(Bytes::copy_from_slice(&buf[..len])) else {
                            continue;
                        };
                        if first_packet {
                            info!("Erstes RTP-Paket auf Port {} von {} (PT {})", local_port, src, pkt.payload_type);
                            first_packet = false;
                        }
                        let Some(ref dev) = rx_audio else { continue };
                        match pkt.payload_type {
                            8 => {
                                let pcm: Vec<i16> = pkt.payload.iter().map(|&b| G711::alaw_to_linear(b)).collect();
                                dev.play_pcm_samples(&pcm);
                            }
                            0 => {
                                let pcm: Vec<i16> = pkt.payload.iter().map(|&b| G711::mulaw_to_linear(b)).collect();
                                dev.play_pcm_samples(&pcm);
                            }
                            // telephone-event (DTMF), Comfort Noise etc. ignorieren
                            _ => {}
                        }
                    }
                }
            }
            debug!("RTP receiver on port {} stopped", local_port);
        });

        // Transmitter Task (20ms frames = 50 packets per second, 160 samples per frame)
        let tx_socket = self.socket.clone();
        let tx_paused = self.is_paused.clone();
        let tx_audio = audio;
        let tx_remote = self.remote_addr.clone();
        let tx_pt = self.payload_type.clone();
        let mut tx_shutdown = self.shutdown.subscribe();
        let ssrc = self.ssrc;

        tokio::spawn(async move {
            let mut seq = rand::random::<u16>();
            let mut timestamp = rand::random::<u32>();
            let mut marker = true;
            let mut ticker = tokio::time::interval(Duration::from_millis(20));
            ticker.set_missed_tick_behavior(MissedTickBehavior::Burst);

            loop {
                tokio::select! {
                    _ = tx_shutdown.changed() => break,
                    _ = ticker.tick() => {}
                }

                let remote = tx_remote.lock().ok().and_then(|r| *r);
                let Some(remote) = remote else {
                    timestamp = timestamp.wrapping_add(FRAME_SAMPLES as u32);
                    continue;
                };
                if remote.port() == 0 || remote.ip().is_unspecified() || tx_paused.load(Ordering::SeqCst) {
                    timestamp = timestamp.wrapping_add(FRAME_SAMPLES as u32);
                    continue;
                }

                let pt = tx_pt.load(Ordering::SeqCst);
                let samples = tx_audio
                    .as_ref()
                    .and_then(|dev| dev.read_mic_samples(FRAME_SAMPLES))
                    .unwrap_or_else(|| vec![0i16; FRAME_SAMPLES]);
                let payload: Vec<u8> = if pt == 0 {
                    samples.into_iter().map(G711::linear_to_mulaw).collect()
                } else {
                    samples.into_iter().map(G711::linear_to_alaw).collect()
                };

                let mut packet = RtpPacket::new(pt, seq, timestamp, ssrc, Bytes::from(payload));
                packet.marker = marker;
                marker = false;
                if let Err(e) = tx_socket.send_to(&packet.serialize(), remote).await {
                    debug!("RTP send error: {}", e);
                }

                seq = seq.wrapping_add(1);
                timestamp = timestamp.wrapping_add(FRAME_SAMPLES as u32);
            }
            debug!("RTP transmitter stopped");
        });
    }

    /// Pauses audio transmission (used during Hold)
    pub fn pause(&self) {
        self.is_paused.store(true, Ordering::SeqCst);
        info!("RTP session on port {} paused (Hold)", self.local_port);
    }

    /// Resumes audio transmission (used during Unhold)
    pub fn resume(&self) {
        self.is_paused.store(false, Ordering::SeqCst);
        info!("RTP session on port {} resumed (Unhold)", self.local_port);
    }

    /// Returns true if audio transmission is currently paused
    pub fn is_paused(&self) -> bool {
        self.is_paused.load(Ordering::SeqCst)
    }

    /// Stops both RTP tasks and releases microphone / speaker
    pub fn stop(&self) {
        let was_running = self.is_running.swap(false, Ordering::SeqCst);
        let _ = self.shutdown.send(true);
        if let Ok(mut dev) = self.audio_device.lock() {
            *dev = None;
        }
        if was_running {
            info!("RTP session on port {} stopped", self.local_port);
        }
    }

    /// Sets microphone mute state on the audio device
    pub fn set_muted(&self, muted: bool) {
        self.muted.store(muted, Ordering::SeqCst);
        if let Ok(dev) = self.audio_device.lock() {
            if let Some(ref d) = *dev {
                d.set_muted(muted);
            }
        }
    }

    /// Returns whether the microphone is currently muted
    pub fn is_muted(&self) -> bool {
        self.muted.load(Ordering::SeqCst)
    }

    /// Sends an RFC 4733 DTMF tone sequence over the active RTP session
    pub async fn send_dtmf(&self, digit: char) -> Result<()> {
        let sequence = super::dtmf::DtmfEvent::generate_keypress_sequence(digit)
            .ok_or_else(|| anyhow!("Unsupported DTMF digit: {}", digit))?;
        let remote = self
            .remote_addr()
            .ok_or_else(|| anyhow!("RTP remote endpoint not yet known"))?;

        info!("Sending DTMF tone '{}' over RTP", digit);

        let mut seq = rand::random::<u16>();
        let timestamp = rand::random::<u32>();
        let ssrc = self.ssrc;
        let pt = self.dtmf_payload_type.load(Ordering::SeqCst);

        for (i, payload) in sequence.into_iter().enumerate() {
            let mut pkt = RtpPacket::new(pt, seq, timestamp, ssrc, payload);
            pkt.marker = i == 0;
            let raw = pkt.serialize();
            let _ = self.socket.send_to(&raw, remote).await;
            seq = seq.wrapping_add(1);
            sleep(Duration::from_millis(20)).await;
        }

        Ok(())
    }
}
