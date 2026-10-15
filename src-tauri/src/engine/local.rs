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

/// Accélérateur que whisper.cpp retiendra (même règle que `whisper_backend_init_gpu` : premier
/// périphérique ggml de type GPU), ex. `("Vulkan", "NVIDIA GeForce RTX 5050")`. `None` : CPU seul,
/// soit parce qu'aucun backend GPU n'est compilé, soit parce qu'aucun périphérique n'est présent
/// (Vulkan sans pilote ICD, ou seulement un pilote logiciel type lavapipe, que ggml ignore).
pub fn acceleration() -> Option<(String, String)> {
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

/// Libellé lisible : « Vulkan · NVIDIA GeForce RTX 5050 », ou « Processeur ».
pub fn acceleration_label() -> String {
    match acceleration() {
        Some((backend, device)) if device.is_empty() => backend,
        Some((backend, device)) => format!("{backend} · {device}"),
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
        // Redirige les journaux de whisper.cpp/ggml (sinon imprimés sur stderr).
        LOG_HOOK.call_once(whisper_rs::install_logging_hooks);

        let path_str = model_path
            .to_str()
            .ok_or_else(|| anyhow!("Chemin de modèle non UTF-8 : {}", model_path.display()))?;
        if !model_path.is_file() {
            anyhow::bail!("Modèle introuvable : {}", model_path.display());
        }
        let mut params = WhisperContextParameters::default();
        params.use_gpu(true);
        params.flash_attn(true);
        let ctx = WhisperContext::new_with_params(path_str, params)
            .map_err(|e| anyhow!("Chargement du modèle impossible ({}) : {e}", model_path.display()))?;

        let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
        let threads = cores.clamp(1, 8) as i32;
        let accel = acceleration();
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

    /// L'interrogation du registre ggml ne plante jamais, avec ou sans GPU (Vulkan sans pilote ICD compris).
    #[test]
    fn acceleration_detectee_sans_panique() {
        let label = acceleration_label();
        println!("Accélération : {label}");
        assert!(!label.is_empty());
        assert_eq!(acceleration().is_none(), label == "Processeur");
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
