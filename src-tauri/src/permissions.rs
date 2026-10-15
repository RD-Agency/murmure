// SPDX-License-Identifier: AGPL-3.0-or-later
//! Permissions système : micro et accessibilité (macOS surtout).

use serde::Serialize;

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct Permissions {
    /// "granted" | "denied" | "undetermined" | "unknown"
    pub microphone: &'static str,
    /// "granted" | "denied" | "unknown"
    pub accessibility: &'static str,
}

pub fn check() -> Permissions {
    Permissions { microphone: microphone_status(), accessibility: accessibility_status() }
}

/// Demande l'accès au micro si nécessaire (boîte de dialogue système sur macOS)
/// puis renvoie l'état à jour.
pub async fn request_microphone() -> Permissions {
    #[cfg(target_os = "macos")]
    if microphone_status() == "undetermined" {
        let rx = macos::request_microphone();
        // Le dialogue système attend l'utilisateur : pas de délai trop court.
        let _ = tokio::time::timeout(std::time::Duration::from_secs(120), rx).await;
    }
    check()
}

/// URL / action pour ouvrir le panneau de réglages correspondant.
pub fn settings_url(kind: &str) -> Result<&'static str, String> {
    #[cfg(target_os = "macos")]
    {
        match kind {
            "microphone" => Ok("x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone"),
            "accessibility" => Ok("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility"),
            other => Err(format!("Type de permission inconnu : {other}")),
        }
    }
    #[cfg(target_os = "windows")]
    {
        match kind {
            "microphone" => Ok("ms-settings:privacy-microphone"),
            "accessibility" => Err("Aucun réglage d'accessibilité requis sous Windows.".into()),
            other => Err(format!("Type de permission inconnu : {other}")),
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = kind;
        Err("Aucun panneau de permissions à ouvrir sur cette plateforme.".into())
    }
}

#[cfg(target_os = "macos")]
fn microphone_status() -> &'static str {
    match macos::microphone_authorization() {
        Some(0) => "undetermined",
        Some(1) | Some(2) => "denied", // restreint ou refusé
        Some(3) => "granted",
        _ => "unknown",
    }
}

#[cfg(target_os = "macos")]
fn accessibility_status() -> &'static str {
    if macos::accessibility_trusted() { "granted" } else { "denied" }
}

#[cfg(target_os = "linux")]
fn microphone_status() -> &'static str {
    // Pas de modèle de permission micro hors bac à sable (Flatpak/portails non gérés ici).
    "granted"
}

#[cfg(target_os = "linux")]
fn accessibility_status() -> &'static str {
    // X11 : la simulation clavier est libre. Wayland : dépend des outils (wtype/ydotool).
    if crate::inject::is_wayland() { "unknown" } else { "granted" }
}

#[cfg(target_os = "windows")]
fn microphone_status() -> &'static str {
    // Le réglage de confidentialité Windows n'est pas interrogeable simplement.
    "unknown"
}

#[cfg(target_os = "windows")]
fn accessibility_status() -> &'static str {
    "granted"
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn microphone_status() -> &'static str {
    "unknown"
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn accessibility_status() -> &'static str {
    "unknown"
}

#[cfg(target_os = "macos")]
mod macos {
    use block2::RcBlock;
    use objc2::runtime::{AnyClass, AnyObject, Bool};
    use objc2::msg_send;
    use std::sync::Mutex;

    #[link(name = "AVFoundation", kind = "framework")]
    unsafe extern "C" {
        /// NSString constante « soun ».
        static AVMediaTypeAudio: *const AnyObject;
    }

    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        fn AXIsProcessTrusted() -> u8;
    }

    fn capture_device_class() -> Option<&'static AnyClass> {
        AnyClass::get(c"AVCaptureDevice")
    }

    /// `AVAuthorizationStatus` : 0 indéterminé, 1 restreint, 2 refusé, 3 autorisé.
    pub fn microphone_authorization() -> Option<isize> {
        let cls = capture_device_class()?;
        // SAFETY: méthode de classe documentée, constante liée depuis AVFoundation.
        unsafe {
            let media = AVMediaTypeAudio;
            if media.is_null() {
                return None;
            }
            let status: isize = msg_send![cls, authorizationStatusForMediaType: media];
            Some(status)
        }
    }

    /// Déclenche la demande système ; le récepteur reçoit la réponse de l'utilisateur.
    pub fn request_microphone() -> tokio::sync::oneshot::Receiver<bool> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let Some(cls) = capture_device_class() else { return rx };
        let tx = Mutex::new(Some(tx));
        let block = RcBlock::new(move |granted: Bool| {
            if let Ok(mut guard) = tx.lock()
                && let Some(tx) = guard.take()
            {
                let _ = tx.send(granted.as_bool());
            }
        });
        // SAFETY: le bloc est copié par AVFoundation ; il est appelé sur une file arbitraire
        // et ne capture que des données `Send`.
        unsafe {
            let media = AVMediaTypeAudio;
            if !media.is_null() {
                let _: () = msg_send![cls, requestAccessForMediaType: media, completionHandler: &*block];
            }
        }
        rx
    }

    pub fn accessibility_trusted() -> bool {
        // SAFETY: fonction C sans argument.
        unsafe { AXIsProcessTrusted() != 0 }
    }
}
