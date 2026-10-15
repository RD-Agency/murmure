// SPDX-License-Identifier: AGPL-3.0-or-later
//! Moteur local : whisper.cpp via `whisper-rs`, contexte chargé une seule fois.

use super::{clean_transcript, TranscribeOptions};
use anyhow::{anyhow, Context};
use std::path::{Path, PathBuf};
use std::sync::Once;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

/// whisper.cpp tolère mal les clips de moins d'une seconde : on complète à ~1,1 s.
const MIN_SAMPLES: usize = 16_000;
const PAD_TO_SAMPLES: usize = 17_600;
/// Segment écarté si Whisper l'estime très probablement non vocal.
const NO_SPEECH_DROP: f32 = 0.8;

static LOG_HOOK: Once = Once::new();

/// Stratégie de décodage. Mesuré sur M-series (Metal, large-v3-turbo, 13,5 s d'audio) :
/// greedy ≈ 565 ms, beam 5 ≈ 600 ms (+6 %) → beam 5 quand un GPU est utilisé.
/// Sur CPU seul le décodeur pèse bien plus lourd : greedy.
fn default_strategy(gpu: bool) -> SamplingStrategy {
    if gpu {
        SamplingStrategy::BeamSearch { beam_size: 5, patience: -1.0 }
    } else {
        SamplingStrategy::Greedy { best_of: 1 }
    }
}

/// Threads de calcul selon le nombre de threads logiques. Mesuré sur Ryzen 9 5950X (16 cœurs,
/// 32 threads, large-v3-turbo q5_0, AVX2) : 4 → 27 s, 8 → 18 s, 16 → 14 s, 24 → 11 s, 32 → 75 s.
/// Au-delà des cœurs physiques, le pool de ggml (attente active) s'effondre dès qu'un autre
/// processus prend du temps processeur : jusqu'à 16 threads logiques on garde min(n, 8), au-delà
/// la moitié (≈ cœurs physiques), plafonnée à 16.
fn default_threads(logical: usize) -> i32 {
    logical.clamp(1, 8).max((logical / 2).min(16)) as i32
}

/// Instructions x86 avec lesquelles ggml a été compilé mais que ce processeur n'a pas.
///
/// Les binaires publiés visent x86-64-v3 (AVX2, FMA, F16C : `cmake/ggml-x86-64-v3.cmake`). Sur un
/// processeur plus ancien, le moindre appel ggml finirait en « illegal instruction » : à vérifier
/// avant tout appel à whisper.cpp/ggml. Seules les fonctions `ggml_cpu_has_*` sont appelées ici :
/// elles renvoient une constante fixée à la compilation, sans instruction SIMD.
fn missing_cpu_features() -> Vec<&'static str> {
    #[cfg(target_arch = "x86_64")]
    {
        use whisper_rs::whisper_rs_sys as sys;
        // SAFETY : fonctions C sans argument ni état, qui renvoient un drapeau de compilation.
        let compiled = unsafe {
            [
                ("AVX", sys::ggml_cpu_has_avx(), std::arch::is_x86_feature_detected!("avx")),
                ("AVX2", sys::ggml_cpu_has_avx2(), std::arch::is_x86_feature_detected!("avx2")),
                ("FMA", sys::ggml_cpu_has_fma(), std::arch::is_x86_feature_detected!("fma")),
                ("F16C", sys::ggml_cpu_has_f16c(), std::arch::is_x86_feature_detected!("f16c")),
                ("BMI2", sys::ggml_cpu_has_bmi2(), std::arch::is_x86_feature_detected!("bmi2")),
                ("AVX512", sys::ggml_cpu_has_avx512(), std::arch::is_x86_feature_detected!("avx512f")),
            ]
        };
        compiled.into_iter().filter(|&(_, built, present)| built != 0 && !present).map(|(name, ..)| name).collect()
    }
    #[cfg(not(target_arch = "x86_64"))]
    Vec::new()
}

/// Erreur si ce processeur ne peut pas exécuter le moteur local tel que compilé.
pub fn cpu_check() -> anyhow::Result<()> {
    let missing = missing_cpu_features();
    if missing.is_empty() {
        return Ok(());
    }
    anyhow::bail!(
        "Ce processeur ne dispose pas des instructions {} requises par le moteur local \
         (processeur ancien ou d'entrée de gamme). Utilisez le moteur « Serveur ».",
        missing.join(", ")
    )
}

/// Accélérateur que whisper.cpp retiendra (même règle que `whisper_backend_init_gpu` : premier
/// périphérique ggml de type GPU), ex. `("Vulkan", "NVIDIA GeForce RTX 5050")`. `None` : CPU seul,
/// soit parce qu'aucun backend GPU n'est compilé, soit parce qu'aucun périphérique n'est présent
/// (Vulkan sans pilote ICD, ou seulement un pilote logiciel type lavapipe, que ggml ignore).
pub fn acceleration() -> Option<(String, String)> {
    if cpu_check().is_err() {
        return None;
    }
    use std::ffi::CStr;
    use whisper_rs::whisper_rs_sys as sys;
    let text = |p: *const std::os::raw::c_char| {
        if p.is_null() {
            String::new()
        } else {
            // SAFETY : chaînes C statiques détenues par le registre ggml.
            unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
        }
    };
    // SAFETY : API C du registre ggml, initialisée à la première utilisation et jamais libérée.
    unsafe {
        (0..sys::ggml_backend_dev_count()).find_map(|i| {
            let dev = sys::ggml_backend_dev_get(i);
            if dev.is_null() || sys::ggml_backend_dev_type(dev) != sys::ggml_backend_dev_type_GGML_BACKEND_DEVICE_TYPE_GPU {
                return None;
            }
            let backend = text(sys::ggml_backend_reg_name(sys::ggml_backend_dev_backend_reg(dev)));
            Some((backend, text(sys::ggml_backend_dev_description(dev)).trim().to_string()))
        })
    }
}

/// Flash attention selon le backend. Désactivée sur Vulkan : whisper.cpp ne prévoit pas le
/// remplissage du cache KV qu'elle exige pour ce backend (`whisper_kv_cache_get_padding`, prévu pour
/// Metal et CUDA seulement), d'où des transcriptions dégradées (mots répétés, phrases tronquées),
/// mesurées sur RTX 5060 (Dozen) et lavapipe. Sans elle, Vulkan retrouve la qualité du CPU.
fn flash_attn_ok(backend: Option<&str>) -> bool {
    !backend.is_some_and(|b| b.eq_ignore_ascii_case("vulkan"))
}

/// Libellé lisible : « Vulkan · NVIDIA GeForce RTX 5050 », ou « Processeur ».
pub fn acceleration_label() -> String {
    match acceleration() {
        Some((backend, device)) if device.is_empty() => backend,
        Some((backend, device)) => format!("{backend} · {device}"),
        None if cpu_check().is_err() => "Processeur non pris en charge (AVX2 absent)".into(),
        None => "Processeur".into(),
    }
}

pub struct LocalEngine {
    ctx: WhisperContext,
    path: PathBuf,
    threads: i32,
    /// Un GPU est utilisé (Metal, CUDA, Vulkan) : décodage beam search.
    gpu: bool,
}

impl LocalEngine {
    /// Charge le modèle (GPU activé : Metal sur macOS, CUDA/Vulkan si compilés et un périphérique
    /// est présent, sinon repli CPU par whisper.cpp). Lourd.
    pub fn load(model_path: &Path) -> anyhow::Result<LocalEngine> {
        // Avant tout appel ggml : sinon « illegal instruction » sur un processeur sans AVX2.
        cpu_check()?;
        // Redirige les journaux de whisper.cpp/ggml vers `log` (cibles whisper_rs::*).
        LOG_HOOK.call_once(|| {
            whisper_rs::install_logging_hooks();
            // Instructions compilées dans ggml (« AVX2 = 1 | FMA = 1 … ») : diagnostic des lenteurs CPU.
            log::info!("whisper.cpp : {}", whisper_rs::print_system_info().trim_end_matches([' ', '|']));
        });

        let path_str = model_path
            .to_str()
            .ok_or_else(|| anyhow!("Chemin de modèle non UTF-8 : {}", model_path.display()))?;
        if !model_path.is_file() {
            anyhow::bail!("Modèle introuvable : {}", model_path.display());
        }
        let accel = acceleration();
        let mut params = WhisperContextParameters::default();
        params.use_gpu(true);
        params.flash_attn(flash_attn_ok(accel.as_ref().map(|(backend, _)| backend.as_str())));
        let ctx = WhisperContext::new_with_params(path_str, params)
            .map_err(|e| anyhow!("Chargement du modèle impossible ({}) : {e}", model_path.display()))?;

        let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
        // MURMURE_THREADS : forçage manuel (diagnostic).
        let threads = std::env::var("MURMURE_THREADS")
            .ok()
            .and_then(|v| v.parse::<i32>().ok())
            .filter(|&n| n > 0)
            .unwrap_or_else(|| default_threads(cores));
        match &accel {
            Some((backend, device)) => log::info!("Accélération : {backend} ({device})"),
            None if cfg!(any(feature = "cuda", feature = "vulkan")) => {
                log::warn!("Accélération : aucun GPU utilisable, transcription sur le processeur")
            }
            None => log::info!("Accélération : aucune, transcription sur le processeur"),
        }
        let engine = LocalEngine { ctx, path: model_path.to_path_buf(), threads, gpu: accel.is_some() };

        // Chauffe : le tout premier passage compile les shaders GPU (jusqu'à ~13 s sur
        // Metal après une mise à jour du binaire). On le paie ici plutôt qu'à la 1re dictée.
        let started = std::time::Instant::now();
        let warmup = TranscribeOptions { language: Some("fr".into()), initial_prompt: None };
        if let Err(e) = engine.transcribe(&[0.0; PAD_TO_SAMPLES], &warmup) {
            log::warn!("Chauffe du modèle en échec : {e:#}");
        }
        log::info!(
            "Modèle local chargé : {} ({threads} threads, chauffe {} ms)",
            model_path.display(),
            started.elapsed().as_millis()
        );
        Ok(engine)
    }

    pub fn model_path(&self) -> &Path {
        &self.path
    }

    /// Transcrit un signal mono 16 kHz f32. Un état whisper neuf est créé à chaque appel.
    pub fn transcribe(&self, samples: &[f32], opts: &TranscribeOptions) -> anyhow::Result<String> {
        self.transcribe_with(samples, opts, default_strategy(self.gpu))
    }

    pub(crate) fn transcribe_with(
        &self,
        samples: &[f32],
        opts: &TranscribeOptions,
        strategy: SamplingStrategy,
    ) -> anyhow::Result<String> {
        if samples.is_empty() {
            return Ok(String::new());
        }
        let padded;
        let audio: &[f32] = if samples.len() < MIN_SAMPLES {
            let mut v = samples.to_vec();
            v.resize(PAD_TO_SAMPLES, 0.0);
            padded = v;
            &padded
        } else {
            samples
        };

        // Les CString internes de whisper-rs paniquent sur un octet nul : on les retire.
        let language = opts.language_code().map(|l| l.replace('\0', ""));
        let prompt = opts.prompt().map(|p| p.replace('\0', ""));

        let mut params = FullParams::new(strategy);
        params.set_n_threads(self.threads);
        params.set_language(Some(language.as_deref().unwrap_or("auto")));
        params.set_translate(false);
        params.set_no_context(true);
        params.set_no_timestamps(true);
        params.set_single_segment(false);
        params.set_suppress_blank(true);
        params.set_suppress_nst(true);
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        params.set_temperature(0.0);
        if let Some(p) = prompt.as_deref() {
            params.set_initial_prompt(p);
        }

        let mut state = self.ctx.create_state().context("Création de l'état whisper")?;
        state
            .full(params, audio)
            .map_err(|e| anyhow!("Transcription locale en échec : {e}"))?;

        let mut text = String::new();
        for segment in state.as_iter() {
            if segment.no_speech_probability() > NO_SPEECH_DROP {
                continue;
            }
            match segment.to_str_lossy() {
                Ok(s) => {
                    text.push_str(&s);
                    text.push(' ');
                }
                Err(e) => log::warn!("Segment illisible : {e}"),
            }
        }
        Ok(clean_transcript(&text))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    /// Lit un WAV mono 16 kHz 16 bits.
    fn read_wav(path: &Path) -> Vec<f32> {
        let r = hound::WavReader::open(path).unwrap();
        let spec = r.spec();
        assert_eq!((spec.channels, spec.sample_rate), (1, 16_000), "WAV mono 16 kHz attendu");
        r.into_samples::<i16>().map(|s| f32::from(s.unwrap()) / 32768.0).collect()
    }

    /// Modèle des tests d'intégration : `MURMURE_TEST_MODEL`, sinon large-v3-turbo de `~/.whisper/models`.
    fn test_model() -> PathBuf {
        std::env::var_os("MURMURE_TEST_MODEL")
            .map(PathBuf::from)
            .unwrap_or_else(|| dirs::home_dir().unwrap().join(".whisper/models/ggml-large-v3-turbo.bin"))
    }

    /// Le processeur qui exécute les tests a les instructions compilées dans ggml (sinon la garde
    /// renverrait une erreur au lieu d'un plantage « illegal instruction »).
    #[test]
    fn processeur_compatible_avec_ggml() {
        cpu_check().unwrap();
        println!("whisper.cpp : {}", whisper_rs::print_system_info());
    }

    /// Build portable (SOURCE_DATE_EPOCH défini, comme en CI de release) : ggml doit être compilé en
    /// x86-64-v3, ni en SSE2 scalaire (régression de la v0.1.4 : ~160 s par dictée) ni en AVX-512.
    #[test]
    #[cfg(target_arch = "x86_64")]
    fn build_portable_en_x86_64_v3() {
        if std::env::var_os("SOURCE_DATE_EPOCH").is_none() {
            return; // build de développement : -march=native
        }
        use whisper_rs::whisper_rs_sys as sys;
        // SAFETY : drapeaux de compilation, sans état.
        let flags = unsafe {
            [sys::ggml_cpu_has_avx(), sys::ggml_cpu_has_avx2(), sys::ggml_cpu_has_fma(), sys::ggml_cpu_has_f16c()]
        };
        assert_eq!(flags, [1, 1, 1, 1], "ggml sans AVX/AVX2/FMA/F16C : cmake/ggml-x86-64-v3.cmake non appliqué");
        assert_eq!(unsafe { sys::ggml_cpu_has_avx512() }, 0, "AVX-512 compilé : binaire non portable");
    }

    #[test]
    fn threads_par_defaut() {
        let got: Vec<i32> = [1, 2, 4, 8, 12, 16, 24, 32, 64].map(default_threads).to_vec();
        assert_eq!(got, [1, 2, 4, 8, 8, 8, 12, 16, 16]);
    }

    #[test]
    fn flash_attention_coupee_sur_vulkan() {
        assert!(flash_attn_ok(None));
        assert!(flash_attn_ok(Some("Metal")));
        assert!(flash_attn_ok(Some("CUDA")));
        assert!(!flash_attn_ok(Some("Vulkan")));
    }

    /// L'interrogation du registre ggml ne plante jamais, avec ou sans GPU (Vulkan sans pilote ICD compris).
    #[test]
    fn acceleration_detectee_sans_panique() {
        let label = acceleration_label();
        println!("Accélération : {label}");
        assert!(!label.is_empty());
        assert_eq!(acceleration().is_none(), label.starts_with("Processeur"));
    }

    /// `MURMURE_TEST_WAV=/chemin.wav cargo test --lib engine::local -- --ignored --nocapture`
    #[test]
    #[ignore = "nécessite ~/.whisper/models/ggml-large-v3-turbo.bin et un WAV de test"]
    fn integration_transcription_locale() {
        let model = test_model();
        let wav = std::env::var("MURMURE_TEST_WAV").unwrap_or_else(|_| "/tmp/murmure-test.wav".into());
        let samples = read_wav(Path::new(&wav));

        let t = Instant::now();
        let engine = LocalEngine::load(&model).unwrap();
        println!("chargement : {} ms", t.elapsed().as_millis());

        let opts = TranscribeOptions { language: Some("fr".into()), initial_prompt: None };
        // Premier passage : chauffe (allocation des buffers GPU).
        let t = Instant::now();
        let warm = engine.transcribe(&samples, &opts).unwrap();
        println!("défaut (1er appel) : {} ms → {warm}", t.elapsed().as_millis());

        for _ in 0..3 {
            let t = Instant::now();
            let text = engine
                .transcribe_with(&samples, &opts, SamplingStrategy::Greedy { best_of: 1 })
                .unwrap();
            println!("greedy : {} ms → {text}", t.elapsed().as_millis());
        }
        for _ in 0..2 {
            let t = Instant::now();
            let text = engine
                .transcribe_with(&samples, &opts, SamplingStrategy::BeamSearch { beam_size: 5, patience: -1.0 })
                .unwrap();
            println!("beam 5 : {} ms → {text}", t.elapsed().as_millis());
        }
        let t = Instant::now();
        let auto = engine.transcribe(&samples, &TranscribeOptions::default()).unwrap();
        println!("défaut langue auto : {} ms → {auto}", t.elapsed().as_millis());

        // Clip très court (padding) et silence : ne doivent ni planter ni halluciner.
        let short = engine.transcribe(&samples[..8_000.min(samples.len())], &opts).unwrap();
        println!("clip 0,5 s : « {short} »");
        let silence = engine.transcribe(&vec![0.0; 32_000], &opts).unwrap();
        println!("silence 2 s : « {silence} »");
        assert!(silence.is_empty(), "hallucination sur silence : {silence}");

        assert!(warm.to_lowercase().contains("bonjour"), "transcription inattendue : {warm}");
    }

    /// Corpus de dictées (ponctuation prononcée ou naturelle) :
    /// `MURMURE_PUNCT_DIR=/dossier/des/wav cargo test --release --lib engine::local::tests::corpus_ponctuation -- --ignored --nocapture`
    #[test]
    #[ignore = "nécessite ~/.whisper/models/ggml-large-v3-turbo.bin et un dossier de WAV"]
    fn corpus_ponctuation() {
        let model = test_model();
        let dir = std::env::var("MURMURE_PUNCT_DIR").expect("MURMURE_PUNCT_DIR");
        let mut wavs: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().is_some_and(|e| e == "wav"))
            .collect();
        wavs.sort();
        // RUST_LOG=info : journaux whisper.cpp/ggml (périphérique retenu) et ligne « Accélération ».
        let _ = env_logger::builder().is_test(true).try_init();
        let t = Instant::now();
        let engine = LocalEngine::load(&model).unwrap();
        println!("chargement + chauffe : {} ms", t.elapsed().as_millis());
        let prompt = std::env::var("MURMURE_PUNCT_PROMPT").ok();
        let opts = TranscribeOptions { language: Some("fr".into()), initial_prompt: prompt };
        for wav in wavs {
            let t = Instant::now();
            let raw = engine.transcribe(&read_wav(&wav), &opts).unwrap();
            println!("{} : {} ms", wav.file_name().unwrap().to_string_lossy(), t.elapsed().as_millis());
            println!("{} brut  : {raw}", wav.file_name().unwrap().to_string_lossy());
            let name = wav.file_name().unwrap().to_string_lossy().into_owned();
            println!("{name} final : {}", crate::punctuation::apply_fr(&raw).replace('\n', "⏎"));
        }
    }
}
