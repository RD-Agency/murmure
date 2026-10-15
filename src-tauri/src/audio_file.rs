// SPDX-License-Identifier: AGPL-3.0-or-later
//! Fichiers audio (dictaphone) : décodage en mono 16 kHz et découpage en segments sur silence.
//!
//! Décodage en Rust pur : Symphonia pour WAV (PCM, ADPCM), MP3, AAC et ALAC en M4A/MP4, Ogg
//! Vorbis et FLAC ; Opus (Ogg), absent de Symphonia, par `opus-decoder`, directement à 16 kHz. Les
//! autres flux sont mixés en mono puis rééchantillonnés au fil de l'eau (rubato), sans garder le
//! signal d'origine en mémoire.
//!
//! Découpage ([`split_on_silence`]) : segments d'environ 20 s, coupés au point le plus calme entre
//! 15 et 25 s, pour rester dans la fenêtre de 30 s de Whisper sans couper un mot. Ce découpage est
//! propre à la transcription de fichiers.

use anyhow::{Context, anyhow, bail};
use rubato::{FftFixedIn, Resampler};
use std::fs::File;
use std::ops::Range;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::codecs::audio::well_known::CODEC_ID_OPUS;
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, TrackType};
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::packet::Packet;

/// Taux attendu par Whisper.
pub const RATE: u32 = 16_000;
/// Extensions proposées dans la boîte de dialogue.
pub const EXTENSIONS: &[&str] = &["wav", "mp3", "m4a", "aac", "mp4", "ogg", "oga", "opus", "flac"];
/// Au-delà, le fichier est refusé (environ 700 Mo de signal en mémoire).
pub const MAX_DURATION_SECS: u64 = 3 * 3600;

/// Segments visés, et marge de recherche du point de coupe de part et d'autre.
pub const SEGMENT_TARGET: usize = 20 * RATE as usize;
pub const SEGMENT_WINDOW: usize = 5 * RATE as usize;
/// Trame d'analyse de l'énergie pour trouver le point de coupe (30 ms).
const FRAME: usize = 480;

/// Opus est toujours codé à 48 kHz ; le délai d'amorce (`pre_skip`) s'exprime à ce taux.
const OPUS_RATE: u32 = 48_000;
/// Paquet Opus le plus long : 120 ms.
const OPUS_MAX_FRAME_16K: usize = 1920;

/// Décode un fichier en mono 16 kHz f32. `on_progress` reçoit l'avancement de 0 à 1 (si la durée
/// est connue). Interrompu (erreur « annulé ») dès que `cancel` passe à vrai.
pub fn decode_file(path: &Path, cancel: &AtomicBool, on_progress: impl FnMut(f32)) -> anyhow::Result<Vec<f32>> {
    let file = File::open(path).with_context(|| format!("Impossible d'ouvrir {}", path.display()))?;
    let ext = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
    decode(Box::new(file), ext.as_deref(), cancel, on_progress)
}

/// Décode depuis n'importe quelle source (fichier, mémoire pour les tests).
pub fn decode(
    source: Box<dyn MediaSource>,
    extension: Option<&str>,
    cancel: &AtomicBool,
    mut on_progress: impl FnMut(f32),
) -> anyhow::Result<Vec<f32>> {
    let mss = MediaSourceStream::new(source, Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = extension {
        hint.with_extension(ext);
    }
    let mut format = symphonia::default::get_probe()
        .probe(&hint, mss, FormatOptions::default(), MetadataOptions::default())
        .map_err(|e| anyhow!("Format audio non reconnu ({e})"))?;
    let track = format.default_track(TrackType::Audio).ok_or_else(|| anyhow!("Aucune piste audio dans ce fichier"))?;
    let track_id = track.id;
    let params = track
        .codec_params
        .as_ref()
        .and_then(|p| p.audio())
        .cloned()
        .ok_or_else(|| anyhow!("Piste audio illisible"))?;
    let total_frames = track.num_frames.filter(|&n| n > 0);
    let track_delay = track.delay.unwrap_or(0);

    let mut sink = if params.codec == CODEC_ID_OPUS {
        let channels = params.channels.as_ref().map_or(1, |c| c.count()).clamp(1, 2);
        // Le décodeur ramène directement en mono 16 kHz.
        let decoder = opus_decoder::OpusDecoder::new(RATE, 1).map_err(|e| anyhow!("Décodeur Opus : {e}"))?;
        let skip = (track_delay as usize) * RATE as usize / OPUS_RATE as usize;
        log::debug!("fichier Opus, {channels} canal(aux), amorce {skip} échantillons");
        Sink::Opus { decoder, skip, buf: vec![0.0; OPUS_MAX_FRAME_16K] }
    } else {
        let decoder = symphonia::default::get_codecs()
            .make_audio_decoder(&params, &AudioDecoderOptions::default())
            .map_err(|e| anyhow!("Codec audio non pris en charge ({e})"))?;
        Sink::Symphonia { decoder, interleaved: Vec::new(), resampler: None }
    };

    let mut out: Vec<f32> = Vec::new();
    let mut frames_seen: u64 = 0;
    let mut last_progress = -1.0f32;
    let max_samples = MAX_DURATION_SECS as usize * RATE as usize;
    loop {
        if cancel.load(Ordering::Relaxed) {
            bail!(CANCELLED);
        }
        let packet = match format.next_packet() {
            Ok(Some(p)) => p,
            Ok(None) | Err(SymError::ResetRequired) => break,
            Err(SymError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(anyhow!("Lecture du fichier interrompue : {e}")),
        };
        if packet.track_id != track_id {
            continue;
        }
        frames_seen += sink.push(&packet, &mut out)?;
        if out.len() > max_samples {
            bail!("Fichier trop long : plus de {} heures", MAX_DURATION_SECS / 3600);
        }
        if let Some(total) = total_frames {
            let p = (frames_seen as f32 / total as f32).min(1.0);
            if p - last_progress >= 0.01 {
                last_progress = p;
                on_progress(p);
            }
        }
    }
    sink.finish(&mut out)?;
    on_progress(1.0);
    Ok(out)
}

/// Message d'erreur d'une opération annulée (reconnu par l'appelant).
pub const CANCELLED: &str = "Transcription annulée";

// Une seule instance par fichier : la différence de taille entre variantes est sans importance.
#[allow(clippy::large_enum_variant)]
enum Sink {
    Opus { decoder: opus_decoder::OpusDecoder, skip: usize, buf: Vec<f32> },
    Symphonia {
        decoder: Box<dyn symphonia::core::codecs::audio::AudioDecoder>,
        interleaved: Vec<f32>,
        resampler: Option<StreamResampler>,
    },
}

impl Sink {
    /// Décode un paquet, ajoute le signal mono 16 kHz à `out` ; renvoie le nombre de trames lues
    /// (au taux de la piste, pour la progression).
    fn push(&mut self, packet: &Packet, out: &mut Vec<f32>) -> anyhow::Result<u64> {
        match self {
            Sink::Opus { decoder, skip, buf } => {
                let n = match decoder.decode_float(&packet.data, buf, false) {
                    Ok(n) => n.min(buf.len()),
                    Err(e) => {
                        log::debug!("paquet Opus ignoré : {e}");
                        return Ok(0);
                    }
                };
                let drop = (*skip).min(n);
                *skip -= drop;
                out.extend_from_slice(&buf[drop..n]);
                Ok((n * (OPUS_RATE / RATE) as usize) as u64)
            }
            Sink::Symphonia { decoder, interleaved, resampler } => {
                let decoded = match decoder.decode(packet) {
                    Ok(d) => d,
                    Err(SymError::DecodeError(e)) => {
                        log::debug!("paquet ignoré : {e}");
                        return Ok(0);
                    }
                    Err(SymError::IoError(e)) => {
                        log::debug!("paquet ignoré : {e}");
                        return Ok(0);
                    }
                    Err(e) => return Err(anyhow!("Décodage impossible : {e}")),
                };
                let channels = decoded.spec().channels().count().max(1);
                let rate = decoded.spec().rate();
                let frames = decoded.frames();
                decoded.copy_to_vec_interleaved(interleaved);
                let mono = downmix(interleaved, channels);
                let rs = match resampler {
                    Some(r) => r,
                    None => resampler.insert(StreamResampler::new(rate)?),
                };
                rs.push(&mono, out)?;
                Ok(frames as u64)
            }
        }
    }

    fn finish(&mut self, out: &mut Vec<f32>) -> anyhow::Result<()> {
        if let Sink::Symphonia { resampler: Some(rs), .. } = self {
            rs.finish(out)?;
        }
        Ok(())
    }
}

/// Entrelacé → mono (moyenne des canaux).
pub fn downmix(interleaved: &[f32], channels: usize) -> Vec<f32> {
    if channels <= 1 {
        return interleaved.to_vec();
    }
    let inv = 1.0 / channels as f32;
    interleaved.chunks_exact(channels).map(|f| f.iter().sum::<f32>() * inv).collect()
}

/// Rééchantillonnage vers 16 kHz par blocs, latence du filtre compensée à la fin.
pub struct StreamResampler {
    inner: Option<FftFixedIn<f32>>,
    rate: u32,
    pending: Vec<f32>,
    buf: Vec<Vec<f32>>,
    consumed: u64,
    produced: u64,
    /// Échantillons de latence encore à retirer en tête.
    to_skip: usize,
}

impl StreamResampler {
    pub fn new(rate: u32) -> anyhow::Result<Self> {
        if rate == 0 {
            bail!("Taux d'échantillonnage invalide");
        }
        let inner = if rate == RATE {
            None
        } else {
            Some(
                FftFixedIn::<f32>::new(rate as usize, RATE as usize, 1024, 2, 1)
                    .map_err(|e| anyhow!("Rééchantillonneur {rate} Hz → 16 kHz : {e}"))?,
            )
        };
        let (buf, to_skip) = match &inner {
            Some(rs) => (rs.output_buffer_allocate(true), rs.output_delay()),
            None => (Vec::new(), 0),
        };
        Ok(Self { inner, rate, pending: Vec::new(), buf, consumed: 0, produced: 0, to_skip })
    }

    pub fn push(&mut self, input: &[f32], out: &mut Vec<f32>) -> anyhow::Result<()> {
        self.consumed += input.len() as u64;
        let Some(rs) = self.inner.as_mut() else {
            out.extend_from_slice(input);
            self.produced += input.len() as u64;
            return Ok(());
        };
        self.pending.extend_from_slice(input);
        let mut pos = 0;
        while self.pending.len() - pos >= rs.input_frames_next() {
            let n = rs.input_frames_next();
            let (_, produced) = rs
                .process_into_buffer(&[&self.pending[pos..pos + n]], &mut self.buf, None)
                .map_err(|e| anyhow!("Rééchantillonnage : {e}"))?;
            pos += n;
            emit(&self.buf[0][..produced], &mut self.to_skip, &mut self.produced, out);
        }
        self.pending.drain(..pos);
        Ok(())
    }

    /// Vide le filtre ; la sortie totale vaut `entrée × 16000 / taux`.
    pub fn finish(&mut self, out: &mut Vec<f32>) -> anyhow::Result<()> {
        let expected = self.consumed * u64::from(RATE) / u64::from(self.rate);
        if self.inner.is_none() {
            return Ok(());
        }
        let pending = std::mem::take(&mut self.pending);
        if !pending.is_empty() {
            let produced = self.partial(Some(&pending))?;
            emit(&self.buf[0][..produced], &mut self.to_skip, &mut self.produced, out);
        }
        while self.produced < expected {
            let produced = self.partial(None)?;
            if produced == 0 {
                break;
            }
            emit(&self.buf[0][..produced], &mut self.to_skip, &mut self.produced, out);
        }
        let excess = self.produced.saturating_sub(expected) as usize;
        out.truncate(out.len() - excess.min(out.len()));
        self.produced -= excess as u64;
        Ok(())
    }

    fn partial(&mut self, input: Option<&[f32]>) -> anyhow::Result<usize> {
        let Some(rs) = self.inner.as_mut() else { return Ok(0) };
        let res = match input {
            Some(i) => rs.process_partial_into_buffer(Some(&[i]), &mut self.buf, None),
            None => rs.process_partial_into_buffer(None::<&[&[f32]]>, &mut self.buf, None),
        };
        res.map(|(_, produced)| produced).map_err(|e| anyhow!("Rééchantillonnage : {e}"))
    }
}

/// Ajoute un bloc rééchantillonné à `out`, après avoir retiré la latence restante du filtre.
fn emit(chunk: &[f32], to_skip: &mut usize, produced: &mut u64, out: &mut Vec<f32>) {
    let skip = (*to_skip).min(chunk.len());
    *to_skip -= skip;
    out.extend_from_slice(&chunk[skip..]);
    *produced += (chunk.len() - skip) as u64;
}

/// Découpe `samples` (16 kHz) en segments d'environ `target` échantillons, chacun coupé au point le
/// plus calme à ± `window` de la cible. Un signal plus court que `target + window` reste entier.
pub fn split_on_silence(samples: &[f32], target: usize, window: usize) -> Vec<Range<usize>> {
    let mut segments = Vec::new();
    let mut start = 0;
    let window = window.min(target.saturating_sub(FRAME));
    while samples.len() - start > target + window {
        let lo = start + target - window;
        let hi = start + target + window;
        let mut best = (f32::INFINITY, start + target);
        let mut pos = lo;
        while pos + FRAME <= hi {
            let energy: f32 = samples[pos..pos + FRAME].iter().map(|s| s * s).sum();
            let center = pos + FRAME / 2;
            let closer = center.abs_diff(start + target) < best.1.abs_diff(start + target);
            // À énergie égale (silence numérique), on garde le point le plus proche de la cible.
            if energy < best.0 * 0.999 || (energy <= best.0 && closer) {
                best = (energy, center);
            }
            pos += FRAME / 2;
        }
        segments.push(start..best.1);
        start = best.1;
    }
    if start < samples.len() {
        segments.push(start..samples.len());
    }
    segments
}

/// Segment sans parole (mêmes seuils que la dictée) : inutile de le transcrire.
pub fn is_silent(samples: &[f32]) -> bool {
    if samples.is_empty() {
        return true;
    }
    let mut peak = 0f32;
    let mut sum = 0f64;
    for &s in samples {
        peak = peak.max(s.abs());
        sum += f64::from(s) * f64::from(s);
    }
    let rms = (sum / samples.len() as f64).sqrt() as f32;
    rms < 0.002 && peak < 0.01
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn sine(rate: u32, secs: f32, freq: f32) -> Vec<f32> {
        (0..(rate as f32 * secs) as usize)
            .map(|i| (2.0 * std::f32::consts::PI * freq * i as f32 / rate as f32).sin() * 0.5)
            .collect()
    }

    fn wav(rate: u32, channels: u16, frames: &[f32]) -> Vec<u8> {
        let spec = hound::WavSpec { channels, sample_rate: rate, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
        let mut cursor = Cursor::new(Vec::new());
        {
            let mut w = hound::WavWriter::new(&mut cursor, spec).unwrap();
            for &s in frames {
                for _ in 0..channels {
                    w.write_sample((s * f32::from(i16::MAX)) as i16).unwrap();
                }
            }
            w.finalize().unwrap();
        }
        cursor.into_inner()
    }

    fn decode_bytes(bytes: Vec<u8>, ext: &str) -> anyhow::Result<Vec<f32>> {
        let mut steps = Vec::new();
        let out = decode(Box::new(Cursor::new(bytes)), Some(ext), &AtomicBool::new(false), |p| steps.push(p))?;
        assert_eq!(steps.last().copied(), Some(1.0));
        Ok(out)
    }

    fn rms(s: &[f32]) -> f32 {
        (s.iter().map(|x| x * x).sum::<f32>() / s.len() as f32).sqrt()
    }

    #[test]
    fn wav_stereo_44k1_vers_mono_16k() {
        let out = decode_bytes(wav(44_100, 2, &sine(44_100, 3.0, 440.0)), "wav").unwrap();
        assert_eq!(out.len(), 48_000);
        // Énergie conservée (sinus 0,5 → RMS ≈ 0,354), latence du filtre compensée.
        let r = rms(&out[2000..46_000]);
        assert!((r - 0.354).abs() < 0.02, "rms = {r}");
        assert!(out[..200].iter().all(|s| s.abs() < 0.6));
    }

    #[test]
    fn wav_16k_sans_reechantillonnage() {
        let input = sine(16_000, 1.5, 300.0);
        let out = decode_bytes(wav(16_000, 1, &input), "wav").unwrap();
        assert_eq!(out.len(), input.len());
        assert!((out[1000] - input[1000]).abs() < 1e-3);
    }

    #[test]
    fn reechantillonnage_par_blocs_comme_en_une_fois() {
        let input = sine(48_000, 2.0, 220.0);
        let mut by_blocks = Vec::new();
        let mut rs = StreamResampler::new(48_000).unwrap();
        for chunk in input.chunks(1152) {
            rs.push(chunk, &mut by_blocks).unwrap();
        }
        rs.finish(&mut by_blocks).unwrap();
        let whole = crate::audio::resample_to_16k(&input, 48_000).unwrap();
        assert_eq!(by_blocks.len(), whole.len());
        let max_diff = by_blocks.iter().zip(&whole).map(|(a, b)| (a - b).abs()).fold(0.0, f32::max);
        assert!(max_diff < 1e-3, "écart {max_diff}");
    }

    /// Fichiers d'une seconde (sinus 440 Hz) produits par ffmpeg : `testdata/tone.*`.
    #[test]
    fn formats_de_dictaphone() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata");
        for name in ["tone.mp3", "tone.m4a", "tone.ogg", "tone.opus", "tone.flac", "tone-adpcm.wav"] {
            let out = decode_file(&dir.join(name), &AtomicBool::new(false), |_| {})
                .unwrap_or_else(|e| panic!("{name} : {e:#}"));
            let secs = out.len() as f32 / RATE as f32;
            // Les codecs à trames ajoutent un peu de remplissage en fin (MP3, AAC).
            assert!((0.95..=1.15).contains(&secs), "{name} : {secs} s");
            let r = rms(&out[1600..out.len().min(14_400)]);
            // Sinus de ffmpeg (amplitude 1/8) à -6 dB : RMS ≈ 0,031, à la perte du codec près.
            assert!((0.025..=0.037).contains(&r), "{name} : rms {r}");
        }
    }

    #[test]
    fn fichier_illisible() {
        let err = decode_bytes(b"pas de l'audio du tout".to_vec(), "mp3").unwrap_err();
        assert!(err.to_string().contains("Format audio non reconnu"), "{err}");
    }

    #[test]
    fn annulation() {
        let bytes = wav(16_000, 1, &sine(16_000, 2.0, 300.0));
        let err = decode(Box::new(Cursor::new(bytes)), Some("wav"), &AtomicBool::new(true), |_| {}).unwrap_err();
        assert_eq!(err.to_string(), CANCELLED);
    }

    #[test]
    fn mixage_mono() {
        assert_eq!(downmix(&[1.0, 0.0, 0.5, 0.5], 2), vec![0.5, 0.5]);
        assert_eq!(downmix(&[0.2, 0.4], 1), vec![0.2, 0.4]);
    }

    /// Parole simulée (sinus) entrecoupée de silences aux instants donnés (en secondes).
    fn speech_with_gaps(total_secs: usize, gaps: &[f32]) -> Vec<f32> {
        let mut s = sine(RATE, total_secs as f32, 200.0);
        for &g in gaps {
            let at = (g * RATE as f32) as usize;
            for x in &mut s[at - 2400..at + 2400] {
                *x = 0.0;
            }
        }
        s
    }

    #[test]
    fn decoupage_sur_les_silences() {
        let samples = speech_with_gaps(70, &[18.0, 41.0, 60.0]);
        let segs = split_on_silence(&samples, SEGMENT_TARGET, SEGMENT_WINDOW);
        let cuts: Vec<f32> = segs.iter().skip(1).map(|r| r.start as f32 / RATE as f32).collect();
        assert_eq!(cuts.len(), 3, "{cuts:?}");
        for (cut, gap) in cuts.iter().zip([18.0, 41.0, 60.0]) {
            assert!((cut - gap).abs() < 0.16, "coupe à {cut} s, silence à {gap} s");
        }
        // Couverture exacte, sans trou ni chevauchement, segments sous 30 s.
        assert_eq!(segs.first().unwrap().start, 0);
        assert_eq!(segs.last().unwrap().end, samples.len());
        assert!(segs.windows(2).all(|w| w[0].end == w[1].start));
        assert!(segs.iter().all(|r| r.len() <= 25 * RATE as usize));
    }

    #[test]
    fn decoupage_sans_silence_et_court() {
        // Pas de silence : coupe à la cible (20 s), segments réguliers.
        let samples = sine(RATE, 50.0, 200.0);
        let segs = split_on_silence(&samples, SEGMENT_TARGET, SEGMENT_WINDOW);
        assert!(segs.iter().all(|r| r.len() <= 25 * RATE as usize), "{segs:?}");
        assert_eq!(segs.last().unwrap().end, samples.len());
        // Fichier court : un seul segment.
        let short = vec![0.1; 24 * RATE as usize];
        assert_eq!(split_on_silence(&short, SEGMENT_TARGET, SEGMENT_WINDOW), vec![0..short.len()]);
        assert!(split_on_silence(&[], SEGMENT_TARGET, SEGMENT_WINDOW).is_empty());
    }

    #[test]
    fn segment_silencieux() {
        assert!(is_silent(&[]));
        assert!(is_silent(&vec![0.0005; 16_000]));
        assert!(!is_silent(&sine(RATE, 1.0, 200.0)));
    }
}
