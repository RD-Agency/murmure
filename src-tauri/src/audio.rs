//! Capture micro (cpal) → mono 16 kHz f32, avec niveau RMS pour la pastille.
//!
//! Le `cpal::Stream` n'est pas `Send` sur macOS : il vit entièrement dans un thread
//! dédié (« murmure-audio »), piloté par channel. Ce thread :
//! - reçoit du callback temps réel des blocs mono au taux natif ;
//! - calcule le niveau et appelle `on_level` ~30 fois/s (donc hors thread temps réel) ;
//! - au `stop()`, coupe le flux puis rééchantillonne le tout en 16 kHz (rubato).

use anyhow::{anyhow, bail, Context};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample, StreamConfig};
use rubato::{FftFixedIn, Resampler};
use serde::Serialize;
use std::sync::mpsc::{self, RecvTimeoutError, TryRecvError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// Taux d'échantillonnage attendu par Whisper.
pub const TARGET_RATE: u32 = 16_000;
/// Période d'émission du niveau (~30 Hz).
const LEVEL_PERIOD: Duration = Duration::from_millis(33);
/// Plage dB projetée sur 0..1 (-60 dB = silence, -10 dB = voix forte).
const LEVEL_FLOOR_DB: f32 = -60.0;
const LEVEL_CEIL_DB: f32 = -10.0;

#[derive(Clone, Debug, Serialize)]
pub struct AudioDevice {
    pub name: String,
    pub is_default: bool,
}

/// Liste les périphériques d'entrée (micros) du système.
pub fn list_input_devices() -> Vec<AudioDevice> {
    let host = cpal::default_host();
    let default_name = host
        .default_input_device()
        .and_then(|d| d.name().ok())
        .unwrap_or_default();
    let mut devices: Vec<AudioDevice> = Vec::new();
    match host.input_devices() {
        Ok(iter) => {
            for d in iter {
                let Ok(name) = d.name() else { continue };
                if devices.iter().any(|x| x.name == name) {
                    continue; // certains hôtes listent des doublons
                }
                devices.push(AudioDevice { is_default: name == default_name, name });
            }
        }
        Err(e) => log::warn!("Énumération des micros impossible : {e}"),
    }
    devices
}

/// Linux (PulseAudio / PipeWire) : nom de la source par défaut du serveur son quand c'est le
/// moniteur d'une sortie (ex. `alsa_output.pci-0000_02_00.1.hdmi-stereo.monitor`). Le micro
/// « default » d'ALSA enregistre alors ce qui sort des haut-parleurs, donc du silence.
pub fn default_source_monitor() -> Option<String> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    let pactl = |args: &[&str]| {
        std::process::Command::new("pactl")
            .args(args)
            .env("LC_ALL", "C")
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
    };
    // `get-default-source` : PulseAudio ≥ 15 et PipeWire ; sinon la ligne de `pactl info`.
    let source = pactl(&["get-default-source"])
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| pactl(&["info"]).and_then(|info| parse_default_source(&info)))?;
    is_monitor_source(&source).then_some(source)
}

fn parse_default_source(pactl_info: &str) -> Option<String> {
    pactl_info
        .lines()
        .find_map(|l| l.trim().strip_prefix("Default Source:"))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn is_monitor_source(name: &str) -> bool {
    name.ends_with(".monitor")
}

/// Micro choisi qui suit la source par défaut du serveur son (aucun choix, ou périphérique ALSA
/// virtuel routé vers PulseAudio / PipeWire).
pub fn follows_server_default(device: Option<&str>) -> bool {
    match device.map(str::trim) {
        None | Some("") => true,
        Some(d) => matches!(d, "default" | "pulse" | "pipewire" | "sysdefault"),
    }
}

/// Linux : le périphérique « default » d'ALSA n'est pas routé vers le serveur son (ex. PipeWire
/// sans `pipewire-alsa`). On essaie les périphériques ALSA du serveur son, s'ils existent.
fn server_fallback(host: &cpal::Host) -> Option<(cpal::Device, cpal::SupportedStreamConfig)> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    let devices: Vec<cpal::Device> = host.input_devices().ok()?.collect();
    ["pipewire", "pulse"].iter().find_map(|want| {
        let d = devices.iter().find(|d| d.name().ok().as_deref() == Some(*want))?;
        let c = d.default_input_config().ok()?;
        Some((d.clone(), c))
    })
}

/// Commande envoyée au thread de capture.
enum Command {
    /// Arrêt normal : on renvoie l'audio.
    Stop,
}

/// Enregistrement en cours. `Send` : ne contient qu'un channel et un JoinHandle.
pub struct Recorder {
    cmd_tx: Option<mpsc::Sender<Command>>,
    worker: Option<JoinHandle<anyhow::Result<Vec<f32>>>>,
}

impl Recorder {
    /// Démarre la capture. `on_level` est appelé ~30 fois/s avec un niveau 0..1 (hors thread temps réel).
    pub fn start(
        device: Option<&str>,
        on_level: impl Fn(f32) + Send + 'static,
    ) -> anyhow::Result<Recorder> {
        let device = device.map(str::to_owned).filter(|d| !d.trim().is_empty());
        let (cmd_tx, cmd_rx) = mpsc::channel::<Command>();
        let (ready_tx, ready_rx) = mpsc::sync_channel::<anyhow::Result<()>>(1);

        let worker = thread::Builder::new()
            .name("murmure-audio".into())
            .spawn(move || capture_thread(device, on_level, cmd_rx, ready_tx))
            .context("Impossible de lancer le thread audio")?;

        // On attend que le flux soit réellement démarré pour remonter les erreurs tôt.
        match ready_rx.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(())) => Ok(Recorder { cmd_tx: Some(cmd_tx), worker: Some(worker) }),
            Ok(Err(e)) => {
                let _ = worker.join();
                Err(e)
            }
            Err(_) => {
                drop(cmd_tx); // le thread s'arrêtera de lui-même s'il finit par démarrer
                bail!("Le micro ne répond pas (délai dépassé à l'ouverture)")
            }
        }
    }

    /// Arrête et renvoie l'audio mono 16 kHz f32.
    pub fn stop(mut self) -> anyhow::Result<Vec<f32>> {
        if let Some(tx) = self.cmd_tx.take() {
            let _ = tx.send(Command::Stop);
        }
        let worker = self.worker.take().ok_or_else(|| anyhow!("Enregistrement déjà arrêté"))?;
        worker
            .join()
            .map_err(|_| anyhow!("Le thread audio s'est arrêté brutalement"))?
    }
}

impl Drop for Recorder {
    /// Abandon sans `stop()` (annulation) : fermer le channel suffit, le thread
    /// coupe le flux et sort sans rééchantillonner.
    fn drop(&mut self) {
        self.cmd_tx.take();
        self.worker.take();
    }
}

/// Corps du thread audio : ouvre le flux, collecte, émet le niveau, puis rééchantillonne.
fn capture_thread(
    device_name: Option<String>,
    on_level: impl Fn(f32),
    cmd_rx: mpsc::Receiver<Command>,
    ready_tx: mpsc::SyncSender<anyhow::Result<()>>,
) -> anyhow::Result<Vec<f32>> {
    let (data_tx, data_rx) = mpsc::channel::<Vec<f32>>();
    let (stream, native_rate) = match open_stream(device_name.as_deref(), data_tx) {
        Ok(v) => v,
        Err(e) => {
            let msg = format!("{e:#}");
            let _ = ready_tx.send(Err(e));
            return Err(anyhow!(msg));
        }
    };
    let _ = ready_tx.send(Ok(()));

    let mut native: Vec<f32> = Vec::with_capacity(native_rate as usize * 30);
    let mut meter = LevelMeter::default();
    let mut last_emit = Instant::now();
    let stopped = loop {
        match cmd_rx.try_recv() {
            Ok(Command::Stop) => break true,
            Err(TryRecvError::Disconnected) => break false,
            Err(TryRecvError::Empty) => {}
        }
        match data_rx.recv_timeout(Duration::from_millis(10)) {
            Ok(chunk) => {
                meter.feed(&chunk);
                native.extend_from_slice(&chunk);
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break true,
        }
        if last_emit.elapsed() >= LEVEL_PERIOD {
            last_emit = Instant::now();
            on_level(meter.take_level());
        }
    };

    // Coupe le flux puis récupère ce qui restait dans le channel.
    drop(stream);
    if !stopped {
        return Ok(Vec::new()); // annulation : inutile de rééchantillonner
    }
    while let Ok(chunk) = data_rx.try_recv() {
        native.extend_from_slice(&chunk);
    }
    on_level(0.0);
    resample_to_16k(&native, native_rate)
}

/// Ouvre le périphérique (par nom, sinon défaut) et démarre un flux mono f32.
fn open_stream(
    device_name: Option<&str>,
    tx: mpsc::Sender<Vec<f32>>,
) -> anyhow::Result<(cpal::Stream, u32)> {
    let host = cpal::default_host();
    let device = match device_name {
        Some(name) => {
            let found = host
                .input_devices()
                .context("Énumération des micros impossible")?
                .find(|d| d.name().ok().as_deref() == Some(name));
            match found {
                Some(d) => d,
                None => {
                    // Micro débranché : on retombe sur le défaut plutôt que d'échouer.
                    log::warn!("Micro « {name} » introuvable, utilisation du micro par défaut");
                    host.default_input_device().ok_or_else(|| anyhow!("Aucun micro disponible"))?
                }
            }
        }
        None => host.default_input_device().ok_or_else(|| anyhow!("Aucun micro disponible"))?,
    };

    let (device, supported) = match device.default_input_config() {
        Ok(c) => (device, c),
        Err(e) => match server_fallback(&host).filter(|_| follows_server_default(device_name)) {
            Some((d, c)) => {
                log::warn!(
                    "micro par défaut d'ALSA inutilisable ({e}) : repli sur « {} » \
                     (installez pipewire-alsa ou le plugin ALSA pulse pour corriger)",
                    d.name().unwrap_or_default()
                );
                (d, c)
            }
            None => {
                return Err(anyhow::Error::new(e)
                    .context("Configuration du micro illisible (accès au micro refusé ?)"));
            }
        },
    };
    let format = supported.sample_format();
    let config: StreamConfig = supported.config();
    let rate = config.sample_rate.0;
    log::info!(
        "Micro « {} » : {} Hz, {} canal(aux), {:?}",
        device.name().unwrap_or_default(),
        rate,
        config.channels,
        format
    );

    let stream = match format {
        SampleFormat::F32 => build_stream::<f32>(&device, &config, tx),
        SampleFormat::F64 => build_stream::<f64>(&device, &config, tx),
        SampleFormat::I8 => build_stream::<i8>(&device, &config, tx),
        SampleFormat::I16 => build_stream::<i16>(&device, &config, tx),
        SampleFormat::I32 => build_stream::<i32>(&device, &config, tx),
        SampleFormat::I64 => build_stream::<i64>(&device, &config, tx),
        SampleFormat::U8 => build_stream::<u8>(&device, &config, tx),
        SampleFormat::U16 => build_stream::<u16>(&device, &config, tx),
        SampleFormat::U32 => build_stream::<u32>(&device, &config, tx),
        SampleFormat::U64 => build_stream::<u64>(&device, &config, tx),
        other => bail!("Format d'échantillons non pris en charge : {other:?}"),
    }
    .context("Ouverture du flux micro impossible")?;
    stream.play().context("Démarrage du micro impossible")?;
    Ok((stream, rate))
}

/// Construit le flux : le callback temps réel se contente de mixer en mono et d'envoyer.
fn build_stream<T>(
    device: &cpal::Device,
    config: &StreamConfig,
    tx: mpsc::Sender<Vec<f32>>,
) -> Result<cpal::Stream, cpal::BuildStreamError>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let channels = usize::from(config.channels.max(1));
    device.build_input_stream(
        config,
        move |data: &[T], _: &cpal::InputCallbackInfo| {
            let _ = tx.send(downmix::<T>(data, channels));
        },
        |err| log::warn!("Erreur du flux micro : {err}"),
        None,
    )
}

/// Entrelacé multi-canaux (tout format) → mono f32 (moyenne des canaux).
fn downmix<T>(data: &[T], channels: usize) -> Vec<f32>
where
    T: Sample,
    f32: FromSample<T>,
{
    if channels == 1 {
        return data.iter().map(|&s| s.to_sample::<f32>()).collect();
    }
    let inv = 1.0 / channels as f32;
    data.chunks_exact(channels)
        .map(|frame| frame.iter().map(|&s| s.to_sample::<f32>()).sum::<f32>() * inv)
        .collect()
}

/// Mesure RMS sur la fenêtre courante, projetée en dB perceptif 0..1, avec retombée douce.
#[derive(Default)]
struct LevelMeter {
    sum_sq: f64,
    count: usize,
    level: f32,
}

impl LevelMeter {
    fn feed(&mut self, samples: &[f32]) {
        self.sum_sq += samples.iter().map(|&s| f64::from(s) * f64::from(s)).sum::<f64>();
        self.count += samples.len();
    }

    fn take_level(&mut self) -> f32 {
        let rms = if self.count == 0 { 0.0 } else { (self.sum_sq / self.count as f64).sqrt() as f32 };
        self.sum_sq = 0.0;
        self.count = 0;
        let target = rms_to_level(rms);
        // Attaque immédiate, relâchement progressif (évite le scintillement de la pastille).
        self.level = if target >= self.level { target } else { self.level * 0.7 + target * 0.3 };
        self.level
    }
}

/// RMS linéaire → niveau 0..1 sur une échelle en dB.
pub(crate) fn rms_to_level(rms: f32) -> f32 {
    let db = 20.0 * rms.max(1e-7).log10();
    ((db - LEVEL_FLOOR_DB) / (LEVEL_CEIL_DB - LEVEL_FLOOR_DB)).clamp(0.0, 1.0)
}

/// Rééchantillonne un signal mono vers 16 kHz (FFT avec filtre anti-repliement).
/// La longueur de sortie vaut `len * 16000 / rate` (latence du filtre compensée).
pub(crate) fn resample_to_16k(input: &[f32], rate: u32) -> anyhow::Result<Vec<f32>> {
    if rate == TARGET_RATE || input.is_empty() {
        return Ok(input.to_vec());
    }
    if rate == 0 {
        bail!("Taux d'échantillonnage du micro invalide");
    }
    let mut rs = FftFixedIn::<f32>::new(rate as usize, TARGET_RATE as usize, 1024, 2, 1)
        .map_err(|e| anyhow!("Rééchantillonneur {rate} Hz → 16 kHz : {e}"))?;
    let expected = (input.len() as u64 * u64::from(TARGET_RATE) / u64::from(rate)) as usize;
    let delay = rs.output_delay();
    let mut out: Vec<f32> = Vec::with_capacity(expected + delay + rs.output_frames_max());
    let mut buf = rs.output_buffer_allocate(true);

    let mut pos = 0;
    while input.len() - pos >= rs.input_frames_next() {
        let n = rs.input_frames_next();
        let (_, produced) = rs
            .process_into_buffer(&[&input[pos..pos + n]], &mut buf, None)
            .map_err(|e| anyhow!("Rééchantillonnage : {e}"))?;
        out.extend_from_slice(&buf[0][..produced]);
        pos += n;
    }
    if pos < input.len() {
        let (_, produced) = rs
            .process_partial_into_buffer(Some(&[&input[pos..]]), &mut buf, None)
            .map_err(|e| anyhow!("Rééchantillonnage : {e}"))?;
        out.extend_from_slice(&buf[0][..produced]);
    }
    // Vidange du filtre jusqu'à couvrir la latence.
    while out.len() < expected + delay {
        let (_, produced) = rs
            .process_partial_into_buffer(None::<&[&[f32]]>, &mut buf, None)
            .map_err(|e| anyhow!("Rééchantillonnage : {e}"))?;
        if produced == 0 {
            break;
        }
        out.extend_from_slice(&buf[0][..produced]);
    }
    let end = (delay + expected).min(out.len());
    Ok(out[delay.min(end)..end].to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monitor_source_detection() {
        assert!(is_monitor_source("alsa_output.pci-0000_02_00.1.pro-output-3.monitor"));
        assert!(!is_monitor_source("alsa_input.usb-046d_HD_Pro_Webcam_C920-02.analog-stereo"));
        let info = "Server Name: pulseaudio\nDefault Sink: alsa_output.x\nDefault Source: alsa_output.x.monitor\n";
        assert_eq!(parse_default_source(info).as_deref(), Some("alsa_output.x.monitor"));
        assert_eq!(parse_default_source("Server Name: x\n"), None);
        assert!(follows_server_default(None));
        assert!(follows_server_default(Some("default")));
        assert!(follows_server_default(Some("pipewire")));
        assert!(!follows_server_default(Some("sysdefault:CARD=C920")));
    }

    fn sine(rate: u32, freq: f32, secs: f32) -> Vec<f32> {
        let n = (rate as f32 * secs) as usize;
        (0..n)
            .map(|i| (2.0 * std::f32::consts::PI * freq * i as f32 / rate as f32).sin() * 0.5)
            .collect()
    }

    #[test]
    fn reechantillonnage_48k_vers_16k() {
        let input = sine(48_000, 440.0, 2.0);
        let out = resample_to_16k(&input, 48_000).unwrap();
        assert_eq!(out.len(), 32_000);
        // L'énergie est conservée (sinus 0,5 → RMS ≈ 0,354), latence compensée.
        let rms = (out[1000..31_000].iter().map(|s| s * s).sum::<f32>() / 30_000.0).sqrt();
        assert!((rms - 0.354).abs() < 0.02, "rms = {rms}");
    }

    #[test]
    fn reechantillonnage_44k1_et_court() {
        let out = resample_to_16k(&sine(44_100, 300.0, 1.0), 44_100).unwrap();
        assert_eq!(out.len(), 16_000);
        // Clip plus court qu'un bloc FFT.
        let out = resample_to_16k(&vec![0.1; 300], 48_000).unwrap();
        assert_eq!(out.len(), 100);
        assert_eq!(resample_to_16k(&[0.2; 10], 16_000).unwrap().len(), 10);
    }

    #[test]
    fn mixage_mono() {
        let st: Vec<i16> = vec![i16::MAX, 0, i16::MIN, 0];
        let m = downmix::<i16>(&st, 2);
        assert_eq!(m.len(), 2);
        assert!((m[0] - 0.5).abs() < 0.01 && (m[1] + 0.5).abs() < 0.01);
        let u: Vec<u16> = vec![32768, 32768];
        assert!(downmix::<u16>(&u, 1).iter().all(|s| s.abs() < 1e-3));
    }

    #[test]
    fn niveau_perceptif() {
        assert_eq!(rms_to_level(0.0), 0.0);
        assert_eq!(rms_to_level(1.0), 1.0);
        let mid = rms_to_level(0.01); // -40 dB
        assert!(mid > 0.3 && mid < 0.5, "{mid}");
    }
}
