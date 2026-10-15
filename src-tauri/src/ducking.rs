// SPDX-License-Identifier: AGPL-3.0-or-later
//! Baisse du volume de sortie pendant la dictée (réglage `duck_volume`).
//!
//! [`Duck::start`] baisse le volume du périphérique de sortie par défaut ; le volume est rétabli
//! quand le [`Duck`] est détruit. Le pipeline le confie à la fonction de niveau du `Recorder` : il
//! vit donc exactement aussi longtemps que la capture, et le volume revient à la fin de
//! l'enregistrement, à l'annulation comme après une erreur.
//!
//! - La baisse attend [`START_DELAY`] pour laisser passer le son de début de dictée.
//! - Le volume d'origine n'est rétabli que si personne ne l'a changé entre-temps.
//! - Il est aussi écrit dans `volume-restore.json` (dossier de configuration) et effacé au
//!   rétablissement : après un arrêt brutal de Murmure, [`recover`] le rétablit au démarrage.
//!
//! Plateformes : Linux par `wpctl` (PipeWire) ou à défaut `pactl` (PulseAudio et PipeWire), macOS
//! par CoreAudio (volume principal virtuel du périphérique par défaut), Windows par
//! `IAudioEndpointVolume` (Core Audio). Sans mixeur utilisable, rien ne se passe.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

/// Volume pendant la dictée, en proportion du volume d'origine.
pub const DUCK_FACTOR: f32 = 0.3;
/// Laisse au son de début de dictée le temps d'être entendu.
const START_DELAY: Duration = Duration::from_millis(250);
/// Écart toléré entre le volume baissé et celui relu au rétablissement (arrondis des mixeurs).
const TOLERANCE: f32 = 0.02;
const RESTORE_FILE: &str = "volume-restore.json";

/// Volume d'origine et volume appliqué, de 0 à 1.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Saved {
    pub original: f32,
    pub ducked: f32,
}

/// Volume baissé tant que cet objet existe.
pub struct Duck {
    cancel: Arc<AtomicBool>,
    worker: Option<JoinHandle<Option<Saved>>>,
    restore_file: PathBuf,
}

impl Duck {
    /// Baisse le volume en arrière-plan (sans bloquer l'ouverture du micro).
    pub fn start(config_dir: &Path) -> Duck {
        let cancel = Arc::new(AtomicBool::new(false));
        let restore_file = config_dir.join(RESTORE_FILE);
        let (flag, file) = (cancel.clone(), restore_file.clone());
        let worker = std::thread::Builder::new()
            .name("murmure-volume".into())
            .spawn(move || {
                std::thread::sleep(START_DELAY);
                if flag.load(Ordering::Relaxed) {
                    return None;
                }
                duck(&file)
            })
            .map_err(|e| log::warn!("baisse du volume : {e}"))
            .ok();
        Duck { cancel, worker, restore_file }
    }
}

impl Drop for Duck {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        let Some(worker) = self.worker.take() else { return };
        if let Ok(Some(saved)) = worker.join() {
            restore(saved);
            let _ = std::fs::remove_file(&self.restore_file);
        }
    }
}

fn duck(restore_file: &Path) -> Option<Saved> {
    let original = mixer::get()?;
    let target = ducked_volume(original)?;
    // Écrit avant de baisser : un arrêt brutal entre les deux reste rattrapable.
    let saved = Saved { original, ducked: target };
    if let Ok(json) = serde_json::to_vec(&saved) {
        let _ = std::fs::write(restore_file, json);
    }
    if !mixer::set(target) {
        let _ = std::fs::remove_file(restore_file);
        return None;
    }
    // Pas de relecture immédiate : certains serveurs son appliquent la valeur avec un léger retard.
    log::debug!("volume baissé : {original:.2} → {target:.2}");
    Some(saved)
}

/// Volume à appliquer pendant la dictée ; `None` si le son est déjà coupé ou presque.
pub fn ducked_volume(original: f32) -> Option<f32> {
    (original > 0.01).then(|| (original * DUCK_FACTOR).clamp(0.0, 1.0))
}

/// Faut-il rétablir ? Seulement si le volume n'a pas été modifié pendant la dictée.
pub fn should_restore(saved: Saved, current: Option<f32>) -> bool {
    current.is_none_or(|c| (c - saved.ducked).abs() <= TOLERANCE)
}

fn restore(saved: Saved) {
    if should_restore(saved, mixer::get()) {
        if mixer::set(saved.original) {
            log::debug!("volume rétabli : {:.2}", saved.original);
        } else {
            log::warn!("impossible de rétablir le volume à {:.2}", saved.original);
        }
    } else {
        log::debug!("volume modifié pendant la dictée : laissé tel quel");
    }
}

/// Au démarrage : rétablit un volume resté baissé après un arrêt brutal.
pub fn recover(config_dir: &Path) {
    let path = config_dir.join(RESTORE_FILE);
    let Ok(raw) = std::fs::read(&path) else { return };
    if let Ok(saved) = serde_json::from_slice::<Saved>(&raw) {
        log::info!("volume resté baissé lors de la session précédente : rétablissement");
        restore(saved);
    }
    let _ = std::fs::remove_file(&path);
}

// ───────────────────────────────── Linux ─────────────────────────────────────

/// `wpctl get-volume @DEFAULT_AUDIO_SINK@` : « Volume: 0.40 » ou « Volume: 0.40 [MUTED] ».
#[cfg(any(target_os = "linux", test))]
fn parse_wpctl(out: &str) -> Option<f32> {
    let line = out.lines().find_map(|l| l.trim().strip_prefix("Volume:"))?;
    if line.contains("MUTED") {
        return None;
    }
    line.split_whitespace().next()?.parse().ok()
}

/// `pactl get-sink-volume @DEFAULT_SINK@` : « Volume: front-left: 26214 /  40% / -23.88 dB, … ».
/// On retient le premier canal.
#[cfg(any(target_os = "linux", test))]
fn parse_pactl(out: &str) -> Option<f32> {
    let line = out.lines().find(|l| l.trim_start().starts_with("Volume:"))?;
    let pct = line.split('/').nth(1)?.trim().strip_suffix('%')?.trim();
    pct.parse::<f32>().ok().map(|p| p / 100.0)
}

#[cfg(target_os = "linux")]
mod mixer {
    use std::process::{Command, Stdio};

    fn run(bin: &str, args: &[&str]) -> Option<String> {
        let out = Command::new(bin).args(args).env("LC_ALL", "C").stderr(Stdio::null()).output().ok()?;
        out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
    }

    pub fn get() -> Option<f32> {
        if let Some(v) = run("wpctl", &["get-volume", "@DEFAULT_AUDIO_SINK@"]).and_then(|o| super::parse_wpctl(&o)) {
            return Some(v);
        }
        run("pactl", &["get-sink-volume", "@DEFAULT_SINK@"]).and_then(|o| super::parse_pactl(&o))
    }

    pub fn set(v: f32) -> bool {
        let v = v.clamp(0.0, 1.0);
        if run("wpctl", &["set-volume", "@DEFAULT_AUDIO_SINK@", &format!("{v:.3}")]).is_some() {
            return true;
        }
        let pct = format!("{}%", (v * 100.0).round() as u32);
        run("pactl", &["set-sink-volume", "@DEFAULT_SINK@", &pct]).is_some()
    }
}

// ───────────────────────────────── macOS ─────────────────────────────────────

#[cfg(target_os = "macos")]
mod mixer {
    use std::ffi::c_void;

    #[repr(C)]
    struct Address {
        selector: u32,
        scope: u32,
        element: u32,
    }

    #[link(name = "CoreAudio", kind = "framework")]
    unsafe extern "C" {
        fn AudioObjectHasProperty(id: u32, address: *const Address) -> u8;
        fn AudioObjectGetPropertyData(
            id: u32,
            address: *const Address,
            qualifier_size: u32,
            qualifier: *const c_void,
            size: *mut u32,
            data: *mut c_void,
        ) -> i32;
        fn AudioObjectSetPropertyData(
            id: u32,
            address: *const Address,
            qualifier_size: u32,
            qualifier: *const c_void,
            size: u32,
            data: *const c_void,
        ) -> i32;
    }

    const fn fourcc(s: &[u8; 4]) -> u32 {
        u32::from_be_bytes(*s)
    }
    const SYSTEM_OBJECT: u32 = 1;
    const DEFAULT_OUTPUT_DEVICE: u32 = fourcc(b"dOut");
    const SCOPE_GLOBAL: u32 = fourcc(b"glob");
    const SCOPE_OUTPUT: u32 = fourcc(b"outp");
    /// `kAudioHardwareServiceDeviceProperty_VirtualMainVolume` : le volume de la barre de menus.
    const VIRTUAL_MAIN_VOLUME: u32 = fourcc(b"vmvc");
    /// `kAudioDevicePropertyVolumeScalar`, pour les périphériques sans volume virtuel.
    const VOLUME_SCALAR: u32 = fourcc(b"volm");

    fn default_device() -> Option<u32> {
        let address = Address { selector: DEFAULT_OUTPUT_DEVICE, scope: SCOPE_GLOBAL, element: 0 };
        let mut device: u32 = 0;
        let mut size = std::mem::size_of::<u32>() as u32;
        // SAFETY : adresse et tampon valides, taille exacte d'un AudioObjectID.
        let status = unsafe {
            AudioObjectGetPropertyData(
                SYSTEM_OBJECT,
                &address,
                0,
                std::ptr::null(),
                &mut size,
                (&mut device as *mut u32).cast(),
            )
        };
        (status == 0 && device != 0).then_some(device)
    }

    /// Propriétés de volume disponibles : volume virtuel, sinon élément principal, sinon canaux 1 et 2.
    fn addresses(device: u32) -> Vec<Address> {
        // SAFETY : simple interrogation d'un objet CoreAudio par une adresse valide.
        let has = |a: &Address| unsafe { AudioObjectHasProperty(device, a) } != 0;
        let virtual_main = Address { selector: VIRTUAL_MAIN_VOLUME, scope: SCOPE_OUTPUT, element: 0 };
        if has(&virtual_main) {
            return vec![virtual_main];
        }
        let main = Address { selector: VOLUME_SCALAR, scope: SCOPE_OUTPUT, element: 0 };
        if has(&main) {
            return vec![main];
        }
        (1..=2).map(|element| Address { selector: VOLUME_SCALAR, scope: SCOPE_OUTPUT, element }).filter(has).collect()
    }

    pub fn get() -> Option<f32> {
        let device = default_device()?;
        let address = addresses(device).into_iter().next()?;
        let mut volume: f32 = 0.0;
        let mut size = std::mem::size_of::<f32>() as u32;
        // SAFETY : tampon f32 de la taille annoncée.
        let status = unsafe {
            AudioObjectGetPropertyData(device, &address, 0, std::ptr::null(), &mut size, (&mut volume as *mut f32).cast())
        };
        (status == 0).then_some(volume)
    }

    pub fn set(volume: f32) -> bool {
        let Some(device) = default_device() else { return false };
        let volume = volume.clamp(0.0, 1.0);
        let list = addresses(device);
        !list.is_empty()
            && list.iter().all(|address| {
                // SAFETY : valeur f32 lue par CoreAudio, taille exacte.
                let status = unsafe {
                    AudioObjectSetPropertyData(
                        device,
                        address,
                        0,
                        std::ptr::null(),
                        std::mem::size_of::<f32>() as u32,
                        (&volume as *const f32).cast(),
                    )
                };
                status == 0
            })
    }
}

// ──────────────────────────────── Windows ────────────────────────────────────

#[cfg(windows)]
mod mixer {
    use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
    use windows::Win32::Media::Audio::{IMMDeviceEnumerator, MMDeviceEnumerator, eConsole, eRender};
    use windows::Win32::System::Com::{CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize};

    /// Volume du périphérique de sortie par défaut, dans un appartement COM propre à l'appel.
    fn with_endpoint<T>(f: impl FnOnce(&IAudioEndpointVolume) -> windows::core::Result<T>) -> Option<T> {
        // SAFETY : initialisation COM équilibrée par CoUninitialize si elle a réussi ; les interfaces
        // sont libérées (Drop) avant.
        unsafe {
            let init = CoInitializeEx(None, COINIT_MULTITHREADED);
            let result = (|| {
                let enumerator: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
                let device = enumerator.GetDefaultAudioEndpoint(eRender, eConsole)?;
                let endpoint: IAudioEndpointVolume = device.Activate(CLSCTX_ALL, None)?;
                f(&endpoint)
            })();
            if init.is_ok() {
                CoUninitialize();
            }
            result.map_err(|e| log::debug!("volume Windows : {e}")).ok()
        }
    }

    pub fn get() -> Option<f32> {
        // SAFETY : appels COM sur une interface valide.
        with_endpoint(|v| unsafe {
            if v.GetMute()?.as_bool() {
                return Ok(None);
            }
            v.GetMasterVolumeLevelScalar().map(Some)
        })
        .flatten()
    }

    pub fn set(volume: f32) -> bool {
        // SAFETY : appel COM sur une interface valide ; pas de contexte d'événement.
        with_endpoint(|v| unsafe { v.SetMasterVolumeLevelScalar(volume.clamp(0.0, 1.0), std::ptr::null()) }).is_some()
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
mod mixer {
    pub fn get() -> Option<f32> {
        None
    }
    pub fn set(_: f32) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_baisse() {
        assert_eq!(ducked_volume(1.0), Some(0.3));
        assert!((ducked_volume(0.5).unwrap() - 0.15).abs() < 1e-6);
        assert_eq!(ducked_volume(0.0), None);
        assert_eq!(ducked_volume(0.005), None);
    }

    #[test]
    fn retablissement_seulement_si_inchange() {
        let saved = Saved { original: 0.6, ducked: 0.18 };
        assert!(should_restore(saved, Some(0.18)));
        assert!(should_restore(saved, Some(0.19)));
        // L'utilisateur a remonté le son pendant la dictée : on n'y touche plus.
        assert!(!should_restore(saved, Some(0.5)));
        // Volume illisible (périphérique changé) : on tente quand même.
        assert!(should_restore(saved, None));
    }

    #[test]
    fn lecture_wpctl() {
        assert_eq!(parse_wpctl("Volume: 0.40\n"), Some(0.40));
        assert_eq!(parse_wpctl("Volume: 1.00"), Some(1.0));
        assert_eq!(parse_wpctl("Volume: 0.40 [MUTED]\n"), None);
        assert_eq!(parse_wpctl("Erreur"), None);
    }

    #[test]
    fn lecture_pactl() {
        let out = "Volume: front-left: 26214 /  40% / -23.88 dB,   front-right: 26214 /  40% / -23.88 dB\n        balance 0.00\n";
        assert_eq!(parse_pactl(out), Some(0.40));
        assert_eq!(parse_pactl("Volume: mono: 65536 / 100% / 0.00 dB"), Some(1.0));
        assert_eq!(parse_pactl("Failed to get sink volume"), None);
    }

    #[test]
    fn fichier_de_secours() {
        let dir = std::env::temp_dir().join(format!("murmure-duck-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let saved = Saved { original: 0.7, ducked: 0.21 };
        std::fs::write(dir.join(RESTORE_FILE), serde_json::to_vec(&saved).unwrap()).unwrap();
        let raw = std::fs::read(dir.join(RESTORE_FILE)).unwrap();
        assert_eq!(serde_json::from_slice::<Saved>(&raw).unwrap(), saved);
        // Fichier illisible : effacé sans rien toucher.
        std::fs::write(dir.join(RESTORE_FILE), b"{").unwrap();
        recover(&dir);
        assert!(!dir.join(RESTORE_FILE).exists());
        std::fs::remove_dir_all(dir).ok();
    }
}
