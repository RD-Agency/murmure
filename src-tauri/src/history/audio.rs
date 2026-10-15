// SPDX-License-Identifier: AGPL-3.0-or-later
//! Audio des dictées : Opus mono 16 kHz, 20 kbit/s VBR, dans un conteneur Ogg (RFC 7845).
//!
//! libopus est compilée depuis ses sources et liée statiquement (`audiopus_sys`, feature `static`,
//! `LIBOPUS_NO_PKG` dans `.cargo/config.toml`) : aucune dépendance système. Environ 9 Mo par heure
//! de parole. Le décodage sert à l'écoute dans l'historique (WAV en mémoire) et aux tests.

use anyhow::{anyhow, bail, Context};
use audiopus_sys as opus;
use std::io::Cursor;

pub const SAMPLE_RATE: i32 = 16_000;
/// Débit cible : voix large bande transparente pour la réécoute et la réutilisation en corpus.
pub const BITRATE: i32 = 20_000;
/// Trames de 20 ms.
const FRAME: usize = 320;
/// L'horodatage Ogg Opus est toujours exprimé à 48 kHz.
const GRANULE_PER_SAMPLE: u64 = 3;
const SERIAL: u32 = 0x4d75_726d; // « Murm »
const MAX_PACKET: usize = 4000;

struct Encoder(*mut opus::OpusEncoder);

impl Drop for Encoder {
    fn drop(&mut self) {
        // SAFETY : pointeur obtenu de opus_encoder_create, libéré une seule fois.
        unsafe { opus::opus_encoder_destroy(self.0) }
    }
}

struct Decoder(*mut opus::OpusDecoder);

impl Drop for Decoder {
    fn drop(&mut self) {
        // SAFETY : pointeur obtenu de opus_decoder_create, libéré une seule fois.
        unsafe { opus::opus_decoder_destroy(self.0) }
    }
}

fn check(code: i32, what: &str) -> anyhow::Result<()> {
    if code < 0 {
        bail!("{what} : erreur Opus {code}");
    }
    Ok(())
}

fn opus_head(pre_skip: u16) -> Vec<u8> {
    let mut h = Vec::with_capacity(19);
    h.extend_from_slice(b"OpusHead");
    h.push(1); // version
    h.push(1); // canaux
    h.extend_from_slice(&pre_skip.to_le_bytes());
    h.extend_from_slice(&(SAMPLE_RATE as u32).to_le_bytes());
    h.extend_from_slice(&0i16.to_le_bytes()); // gain
    h.push(0); // famille de mappage : mono/stéréo
    h
}

fn opus_tags() -> Vec<u8> {
    let vendor = b"Murmure (libopus)";
    let mut t = Vec::with_capacity(16 + vendor.len());
    t.extend_from_slice(b"OpusTags");
    t.extend_from_slice(&(vendor.len() as u32).to_le_bytes());
    t.extend_from_slice(vendor);
    t.extend_from_slice(&0u32.to_le_bytes()); // aucun commentaire
    t
}

/// Encode un signal mono 16 kHz f32 en Ogg Opus.
pub fn encode_ogg_opus(samples: &[f32]) -> anyhow::Result<Vec<u8>> {
    let mut err = 0;
    // SAFETY : paramètres valides (16 kHz, mono, application VOIP) ; `err` reçoit le code d'erreur.
    let enc = Encoder(unsafe { opus::opus_encoder_create(SAMPLE_RATE, 1, opus::OPUS_APPLICATION_VOIP, &mut err) });
    check(err, "création de l'encodeur")?;
    if enc.0.is_null() {
        bail!("création de l'encodeur Opus impossible");
    }
    let mut lookahead: i32 = 0;
    // SAFETY : requêtes CTL documentées, arguments du type attendu (opus_int32 ou opus_int32*).
    unsafe {
        check(opus::opus_encoder_ctl(enc.0, opus::OPUS_SET_BITRATE_REQUEST, BITRATE), "débit")?;
        check(opus::opus_encoder_ctl(enc.0, opus::OPUS_SET_VBR_REQUEST, 1i32), "VBR")?;
        check(opus::opus_encoder_ctl(enc.0, opus::OPUS_SET_COMPLEXITY_REQUEST, 10i32), "complexité")?;
        check(opus::opus_encoder_ctl(enc.0, opus::OPUS_SET_SIGNAL_REQUEST, opus::OPUS_SIGNAL_VOICE), "signal")?;
        check(opus::opus_encoder_ctl(enc.0, opus::OPUS_GET_LOOKAHEAD_REQUEST, &mut lookahead as *mut i32), "latence")?;
    }
    let pre_skip = (lookahead.max(0) as u64 * GRANULE_PER_SAMPLE) as u16;

    let mut out = Cursor::new(Vec::with_capacity(samples.len() / 6 + 1024));
    let mut w = ogg::writing::PacketWriter::new(&mut out);
    use ogg::writing::PacketWriteEndInfo as End;
    w.write_packet(opus_head(pre_skip), SERIAL, End::EndPage, 0)?;
    w.write_packet(opus_tags(), SERIAL, End::EndPage, 0)?;

    // Trames supplémentaires pour vider la latence de l'encodeur : le décodeur doit restituer
    // `pre_skip` + tous les échantillons.
    let frames = (samples.len() + lookahead.max(0) as usize).div_ceil(FRAME).max(1);
    let end_granule = pre_skip as u64 + samples.len() as u64 * GRANULE_PER_SAMPLE;
    let mut pcm = [0f32; FRAME];
    let mut packet = vec![0u8; MAX_PACKET];
    for i in 0..frames {
        let chunk = samples.get(i * FRAME..((i + 1) * FRAME).min(samples.len())).unwrap_or(&[]);
        pcm.fill(0.0);
        for (d, s) in pcm.iter_mut().zip(chunk) {
            *d = s.clamp(-1.0, 1.0);
        }
        // SAFETY : `pcm` contient FRAME échantillons, `packet` MAX_PACKET octets.
        let n = unsafe {
            opus::opus_encode_float(enc.0, pcm.as_ptr(), FRAME as i32, packet.as_mut_ptr(), MAX_PACKET as i32)
        };
        check(n, "encodage")?;
        let last = i + 1 == frames;
        let granule = if last {
            end_granule
        } else {
            pre_skip as u64 + ((i + 1) * FRAME) as u64 * GRANULE_PER_SAMPLE
        };
        // Une page par seconde environ : reprise propre d'un fichier tronqué.
        let info = if last {
            End::EndStream
        } else if (i + 1) % 50 == 0 {
            End::EndPage
        } else {
            End::NormalPacket
        };
        w.write_packet(packet[..n as usize].to_vec(), SERIAL, info, granule)?;
    }
    drop(w);
    Ok(out.into_inner())
}

/// Décode un fichier Ogg Opus produit par [`encode_ogg_opus`] en mono 16 kHz f32.
pub fn decode_ogg_opus(bytes: &[u8]) -> anyhow::Result<Vec<f32>> {
    let mut reader = ogg::reading::PacketReader::new(Cursor::new(bytes));
    let head = reader.read_packet()?.ok_or_else(|| anyhow!("fichier Ogg vide"))?;
    if head.data.len() < 19 || &head.data[..8] != b"OpusHead" {
        bail!("en-tête Opus absent");
    }
    let channels = head.data[9] as i32;
    if channels != 1 {
        bail!("audio non mono ({channels} canaux)");
    }
    let pre_skip = u16::from_le_bytes([head.data[10], head.data[11]]) as u64;
    reader.read_packet()?.ok_or_else(|| anyhow!("commentaires Opus absents"))?;

    let mut err = 0;
    // SAFETY : 16 kHz, mono ; `err` reçoit le code d'erreur.
    let dec = Decoder(unsafe { opus::opus_decoder_create(SAMPLE_RATE, 1, &mut err) });
    check(err, "création du décodeur")?;
    if dec.0.is_null() {
        bail!("création du décodeur Opus impossible");
    }
    let mut out = Vec::new();
    let mut end_granule = None;
    // 120 ms à 16 kHz : durée maximale d'un paquet Opus.
    let mut pcm = vec![0f32; 1920];
    while let Some(p) = reader.read_packet().context("lecture Ogg")? {
        // SAFETY : `pcm` peut recevoir 1920 échantillons, la taille passée.
        let n = unsafe {
            opus::opus_decode_float(dec.0, p.data.as_ptr(), p.data.len() as i32, pcm.as_mut_ptr(), pcm.len() as i32, 0)
        };
        check(n, "décodage")?;
        out.extend_from_slice(&pcm[..n as usize]);
        if p.last_in_stream() {
            end_granule = Some(p.absgp_page());
        }
    }
    let skip = (pre_skip / GRANULE_PER_SAMPLE) as usize;
    let total = end_granule
        .map(|g| (g.saturating_sub(pre_skip) / GRANULE_PER_SAMPLE) as usize)
        .unwrap_or(out.len().saturating_sub(skip));
    let start = skip.min(out.len());
    let end = (start + total).min(out.len());
    Ok(out[start..end].to_vec())
}

/// Version de libopus liée (diagnostic).
pub fn opus_version() -> String {
    // SAFETY : chaîne statique de libopus.
    unsafe { std::ffi::CStr::from_ptr(opus::opus_get_version_string()) }.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Signal proche de la voix : fondamentale modulée et harmoniques, avec des pauses.
    fn voice_like(seconds: f32) -> Vec<f32> {
        let n = (seconds * SAMPLE_RATE as f32) as usize;
        (0..n)
            .map(|i| {
                let t = i as f32 / SAMPLE_RATE as f32;
                let f0 = 140.0 + 30.0 * (t * 3.0).sin();
                let env = ((t * 2.5).sin() * 0.5 + 0.5).powi(2);
                let s: f32 = (1..6).map(|h| (2.0 * std::f32::consts::PI * f0 * h as f32 * t).sin() / h as f32).sum();
                0.25 * env * s
            })
            .collect()
    }

    #[test]
    fn aller_retour_ogg_opus() {
        let input = voice_like(3.0);
        let ogg = encode_ogg_opus(&input).unwrap();
        assert_eq!(&ogg[..4], b"OggS");
        let back = decode_ogg_opus(&ogg).unwrap();
        // Longueur exacte (pre-skip et granule de fin respectés).
        assert_eq!(back.len(), input.len());
        // Alignement : corrélation maximale à moins de 0,5 ms près (pre-skip correct).
        let corr = |lag: i64| -> f64 {
            (200..input.len() - 200)
                .map(|i| f64::from(input[i]) * f64::from(back[(i as i64 + lag) as usize]))
                .sum()
        };
        let best = (-120..=120).max_by(|&a, &b| corr(a).total_cmp(&corr(b))).unwrap();
        assert!(best.abs() <= 8, "décalage de {best} échantillons");
        // Fidélité de l'enveloppe d'énergie par trame de 20 ms (un codec perceptuel ne conserve
        // pas la forme d'onde exacte).
        let rms = |s: &[f32]| s.chunks(FRAME).map(|c| (c.iter().map(|x| x * x).sum::<f32>() / c.len() as f32).sqrt()).collect::<Vec<_>>();
        let (ea, eb) = (rms(&input), rms(&back));
        let err: f32 = ea.iter().zip(&eb).map(|(a, b)| (a - b).abs()).sum::<f32>() / ea.iter().sum::<f32>();
        println!("décalage {best}, écart d'énergie {:.1} %", err * 100.0);
        assert!(err < 0.25, "enveloppe trop différente : {:.1} %", err * 100.0);
    }

    #[test]
    fn taille_par_heure() {
        let input = voice_like(30.0);
        let ogg = encode_ogg_opus(&input).unwrap();
        let per_hour_mb = ogg.len() as f64 * 120.0 / 1e6;
        println!("Opus {} : {} octets pour 30 s, soit {per_hour_mb:.1} Mo/h", opus_version(), ogg.len());
        // 20 kbit/s ≈ 9 Mo/h ; le VBR peut descendre nettement sur un signal simple.
        assert!(per_hour_mb < 12.0, "{per_hour_mb:.1} Mo/h");
    }

    #[test]
    fn signal_tres_court_ou_vide() {
        for len in [0, 1, 319, 320, 321] {
            let input = vec![0.1f32; len];
            let back = decode_ogg_opus(&encode_ogg_opus(&input).unwrap()).unwrap();
            assert_eq!(back.len(), len);
        }
    }

    #[test]
    fn fichier_invalide() {
        assert!(decode_ogg_opus(b"pas un ogg").is_err());
    }
}
