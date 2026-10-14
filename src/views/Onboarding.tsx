import { type ReactNode, useEffect, useMemo, useRef, useState } from "react";
import { Button } from "../components/Button";
import { HotkeyRecorder } from "../components/HotkeyRecorder";
import {
  IconArrowLeft,
  IconArrowRight,
  IconCheck,
  IconDownload,
  IconLaptop,
  IconLock,
  IconServer,
} from "../components/Icons";
import { HotkeyKeys } from "../components/Kbd";
import { Logo } from "../components/Logo";
import { PermissionsPanel } from "../components/PermissionsPanel";
import { Progress } from "../components/Progress";
import { RemoteForm } from "../components/RemoteForm";
import { on } from "../lib/api";
import { downloadPercent, formatProgressSize, formatSize, hotkeyLabel } from "../lib/format";
import type { DictationState, EngineKind, Platform } from "../lib/types";
import { useApp } from "../store";

type Step = "welcome" | "engine" | "setup" | "permissions" | "try";
const STEPS: Step[] = ["welcome", "engine", "setup", "permissions", "try"];

export function Onboarding() {
  const platform = useApp((s) => s.platform);
  const settings = useApp((s) => s.settings)!;
  const update = useApp((s) => s.updateSettings);
  const flush = useApp((s) => s.flushSettings);
  const models = useApp((s) => s.models);
  const [step, setStep] = useState<Step>("welcome");
  const [dir, setDir] = useState<1 | -1>(1);

  const index = STEPS.indexOf(step);
  const go = (delta: 1 | -1) => {
    setDir(delta);
    setStep(STEPS[Math.min(STEPS.length - 1, Math.max(0, index + delta))]);
  };

  const finish = async () => {
    update({ onboarding_done: true });
    await flush();
  };

  const localReady = models.some((m) => m.id === settings.local_model && m.downloaded);
  const remoteReady = settings.remote_endpoint.trim().length > 0;

  let next: ReactNode = null;
  switch (step) {
    case "welcome":
      next = (
        <Button variant="primary" size="lg" onClick={() => go(1)}>
          Commencer <IconArrowRight size={15} />
        </Button>
      );
      break;
    case "engine":
    case "permissions":
      next = (
        <Button variant="primary" size="lg" onClick={() => go(1)}>
          Continuer <IconArrowRight size={15} />
        </Button>
      );
      break;
    case "setup": {
      const ready = settings.engine === "local" ? localReady : remoteReady;
      next = (
        <div className="flex items-center gap-2">
          {!ready && (
            <Button variant="ghost" size="lg" onClick={() => go(1)}>
              Plus tard
            </Button>
          )}
          <Button variant="primary" size="lg" disabled={!ready} onClick={() => go(1)}>
            Continuer <IconArrowRight size={15} />
          </Button>
        </div>
      );
      break;
    }
    case "try":
      next = (
        <Button variant="primary" size="lg" onClick={finish}>
          Terminer
        </Button>
      );
      break;
  }

  return (
    <div className="flex h-full flex-col bg-bg">
      <div data-tauri-drag-region className="flex h-[52px] shrink-0 items-center justify-center">
        <div className="flex items-center gap-1.5" aria-label={`Étape ${index + 1} sur ${STEPS.length}`} role="img">
          {STEPS.map((s, i) => (
            <span
              key={s}
              className={`h-1.5 rounded-full transition-all duration-300 ${
                i === index ? "w-5 bg-accent" : i < index ? "w-1.5 bg-accent/45" : "w-1.5 bg-line-strong"
              }`}
            />
          ))}
        </div>
      </div>

      <main className="flex flex-1 items-center justify-center overflow-y-auto px-8 py-4">
        <div
          key={step}
          className="w-full max-w-[540px]"
          style={{ animation: `${dir === 1 ? "ob-in-right" : "ob-in-left"} 360ms cubic-bezier(0.2, 0.8, 0.2, 1) both` }}
        >
          {step === "welcome" && <Welcome platform={platform} hotkey={settings.hotkey} />}
          {step === "engine" && <EngineChoice value={settings.engine} onChange={(engine) => update({ engine })} />}
          {step === "setup" && (settings.engine === "local" ? <LocalSetup /> : <RemoteSetup />)}
          {step === "permissions" && (
            <StepFrame
              title="Deux autorisations"
              text={
                platform === "macos"
                  ? "Murmure a besoin d'entendre votre voix et de coller le texte dans l'application active."
                  : "Murmure a besoin d'accéder à votre microphone."
              }
            >
              <div className="overflow-hidden rounded-xl border border-line bg-surface shadow-soft">
                <PermissionsPanel platform={platform} />
              </div>
              <p className="mt-3 text-center text-[12px] text-subtle">Le statut se met à jour automatiquement.</p>
            </StepFrame>
          )}
          {step === "try" && <TryIt />}
        </div>
      </main>

      <footer className="flex h-[76px] shrink-0 items-center justify-between px-8">
        <div>
          {index > 0 && (
            <Button variant="ghost" size="lg" onClick={() => go(-1)} icon={<IconArrowLeft size={15} />}>
              Retour
            </Button>
          )}
        </div>
        {next}
      </footer>

    </div>
  );
}

function StepFrame({ title, text, children }: { title: string; text?: ReactNode; children?: ReactNode }) {
  return (
    <div>
      <h1 className="text-center text-[24px] leading-tight font-semibold tracking-[-0.02em] text-fg">{title}</h1>
      {text && <p className="mx-auto mt-2.5 max-w-[440px] text-center text-[14px] leading-relaxed text-muted">{text}</p>}
      {children && <div className="mt-8">{children}</div>}
    </div>
  );
}

// ---------- Étapes ----------

function Welcome({ platform, hotkey }: { platform: Platform; hotkey: string }) {
  return (
    <div className="flex flex-col items-center text-center">
      <div className="mb-7">
        <Logo size={64} />
      </div>
      <h1 className="text-[28px] leading-tight font-semibold tracking-[-0.025em] text-fg">Bienvenue dans Murmure</h1>
      <p className="mt-3 max-w-[400px] text-[14.5px] leading-relaxed text-muted">
        Dictez partout. Maintenez un raccourci, parlez, relâchez : le texte apparaît là où se trouve votre curseur.
      </p>
      <div className="mt-9 flex items-center gap-3 text-[13px] text-subtle">
        <HotkeyKeys accelerator={hotkey} platform={platform} />
        <span>puis parlez</span>
      </div>
    </div>
  );
}

function EngineChoice({ value, onChange }: { value: EngineKind; onChange: (v: EngineKind) => void }) {
  const options: { value: EngineKind; icon: ReactNode; title: string; tagline: string; points: string[] }[] = [
    {
      value: "local",
      icon: <IconLaptop size={20} />,
      title: "Sur cet appareil",
      tagline: "Privé, hors ligne",
      points: ["Whisper large-v3-turbo", "Rien ne quitte votre machine", "Un seul téléchargement de 570 Mo"],
    },
    {
      value: "remote",
      icon: <IconServer size={20} />,
      title: "Serveur",
      tagline: "Endpoint + clé API",
      points: ["Serveur Murmure, OpenAI, Groq…", "Idéal pour les machines modestes", "Clé rangée dans le trousseau"],
    },
  ];
  return (
    <StepFrame title="Où transcrire votre voix ?" text="Vous pourrez changer à tout moment dans les réglages.">
      <div role="radiogroup" aria-label="Moteur de transcription" className="grid grid-cols-2 gap-3">
        {options.map((o) => {
          const active = value === o.value;
          return (
            <button
              key={o.value}
              type="button"
              role="radio"
              aria-checked={active}
              onClick={() => onChange(o.value)}
              className={`relative rounded-[14px] border p-5 text-left transition-[border-color,box-shadow,background-color] duration-200 ${
                active
                  ? "border-accent bg-surface shadow-[0_0_0_3px_var(--accent-soft),var(--shadow-md)]"
                  : "border-line-strong bg-surface/70 hover:border-subtle/50 hover:bg-surface"
              }`}
            >
              <span
                className={`absolute top-4 right-4 flex size-[18px] items-center justify-center rounded-full border transition-colors ${
                  active ? "border-accent bg-accent text-accent-fg" : "border-line-strong"
                }`}
              >
                {active && <IconCheck size={11} strokeWidth={2.6} />}
              </span>
              <span
                className={`flex size-10 items-center justify-center rounded-[11px] transition-colors ${
                  active ? "bg-accent-soft text-accent" : "bg-surface-2 text-muted"
                }`}
              >
                {o.icon}
              </span>
              <div className="mt-4 text-[14.5px] font-semibold text-fg">{o.title}</div>
              <div className="text-[12.5px] text-muted">{o.tagline}</div>
              <ul className="mt-4 grid gap-1.5">
                {o.points.map((p) => (
                  <li key={p} className="flex items-start gap-2 text-[12.5px] leading-snug text-muted">
                    <IconCheck size={13} className="mt-[2px] shrink-0 text-subtle" />
                    {p}
                  </li>
                ))}
              </ul>
            </button>
          );
        })}
      </div>
    </StepFrame>
  );
}

function LocalSetup() {
  const models = useApp((s) => s.models);
  const settings = useApp((s) => s.settings)!;
  const update = useApp((s) => s.updateSettings);
  const downloads = useApp((s) => s.downloads);
  const errors = useApp((s) => s.downloadErrors);
  const download = useApp((s) => s.downloadModel);
  const cancel = useApp((s) => s.cancelDownload);

  // Un modèle déjà présent est réutilisé ; sinon on suit le téléchargement déjà lancé par Rust
  // au démarrage, à défaut le modèle choisi, puis le recommandé.
  const present = models.find((m) => m.downloaded && m.id === settings.local_model) ?? models.find((m) => m.downloaded);
  const [initiallyPresent] = useState(() => !!present);
  const running = models.find((m) => downloads[m.id] && !downloads[m.id].done);
  const target =
    present ??
    running ??
    models.find((m) => m.id === settings.local_model) ??
    models.find((m) => m.recommended) ??
    models[0];

  useEffect(() => {
    if (present && present.id !== settings.local_model) update({ local_model: present.id });
  }, [present, settings.local_model, update]);

  const progress = target ? downloads[target.id] : undefined;
  const error = target ? errors[target.id] : undefined;

  // Arrivée sur l'étape : on démarre l'installation sans attendre de clic (une seule fois ;
  // si Rust l'a déjà lancée, l'appel la rejoint). Après une erreur ou une annulation : bouton.
  const autoStarted = useRef(false);
  useEffect(() => {
    if (!target || target.downloaded || progress || error || autoStarted.current) return;
    autoStarted.current = true;
    void download(target.id);
  }, [target, progress, error, download]);

  if (!target) {
    return <StepFrame title="Modèle de transcription" text="Chargement du catalogue…" />;
  }

  const percent = progress ? downloadPercent(progress.downloaded, progress.total) : 0;

  return (
    <StepFrame
      title={present ? "Modèle prêt" : "Installation du modèle"}
      text={
        present
          ? initiallyPresent
            ? "Un modèle est déjà présent sur cet appareil. Rien à télécharger."
            : "Installation terminée. Tout fonctionne désormais hors ligne."
          : "Murmure installe son modèle de transcription. Un seul téléchargement, ensuite tout fonctionne hors ligne."
      }
    >
      <div className="rounded-[14px] border border-line bg-surface p-5 shadow-soft">
        <div className="flex items-center gap-4">
          <div
            className={`flex size-11 shrink-0 items-center justify-center rounded-[12px] transition-colors ${
              target.downloaded ? "bg-success-soft text-success" : "bg-accent-soft text-accent"
            }`}
          >
            {target.downloaded ? <IconCheck size={20} /> : <IconDownload size={20} />}
          </div>
          <div className="min-w-0 flex-1">
            <div className="flex items-center gap-2">
              <span className="text-[14px] font-semibold text-fg">{target.name}</span>
            </div>
            <div className="text-[12.5px] text-muted">
              {target.downloaded ? "Sur l'appareil" : formatSize(target.size_mb)} · {target.recommended ? "recommandé" : "modèle local"}
            </div>
          </div>
          {!target.downloaded &&
            (progress ? (
              <Button variant="ghost" onClick={() => cancel(target.id)}>
                Annuler
              </Button>
            ) : (
              <Button variant="primary" onClick={() => download(target.id)} icon={<IconDownload size={14} />}>
                {error ? "Réessayer" : "Télécharger"}
              </Button>
            ))}
        </div>
        {progress && !target.downloaded && (
          <div className="mt-5 animate-fade-in">
            <Progress value={percent / 100} label="Progression de l'installation" />
            <div className="mt-2 flex justify-between text-[12px] text-muted tabular-nums">
              <span>{percent} %</span>
              <span>{formatProgressSize(progress.downloaded, progress.total)}</span>
            </div>
          </div>
        )}
        {error && <p className="mt-3 text-[12.5px] text-danger">{error}</p>}
      </div>
      <p className="mt-4 flex items-center justify-center gap-1.5 text-[12px] text-subtle">
        <IconLock size={12} /> L'audio reste sur votre machine.
      </p>
    </StepFrame>
  );
}

function RemoteSetup() {
  return (
    <StepFrame title="Connecter un serveur" text="Tout service compatible avec l'API de transcription d'OpenAI.">
      <div className="rounded-[14px] border border-line bg-surface p-5 shadow-soft">
        <RemoteForm compact />
      </div>
    </StepFrame>
  );
}

const LIVE_LABEL: Record<DictationState, string> = {
  idle: "",
  recording: "À l'écoute…",
  transcribing: "Transcription…",
  error: "",
};

function TryIt() {
  const platform = useApp((s) => s.platform);
  const settings = useApp((s) => s.settings)!;
  const update = useApp((s) => s.updateSettings);
  const ref = useRef<HTMLTextAreaElement>(null);
  const [text, setText] = useState("");
  const [state, setState] = useState<{ state: DictationState; message?: string; hands_free?: boolean }>({ state: "idle" });
  const [success, setSuccess] = useState(false);
  const [editing, setEditing] = useState(false);

  useEffect(() => {
    ref.current?.focus();
    const offState = on("dictation-state", setState);
    const offResult = on("dictation-result", (entry) => {
      setSuccess(true);
      // Filet de sécurité : si le collage n'a pas atteint le champ (accessibilité refusée…), on insère nous-mêmes.
      setTimeout(() => {
        const el = ref.current;
        if (el && !el.value.includes(entry.text)) {
          setText((t) => (t ? `${t.trimEnd()} ${entry.text}` : entry.text));
        }
      }, 450);
    });
    return () => {
      offState();
      offResult();
    };
  }, []);

  const label = useMemo(() => hotkeyLabel(settings.hotkey, platform), [settings.hotkey, platform]);
  const live = state.state === "error" ? state.message ?? "Erreur" : LIVE_LABEL[state.state];

  return (
    <StepFrame title="Essayez" text="Cliquez dans le champ ci-dessous, maintenez le raccourci et parlez. Relâchez pour insérer le texte.">
      <div className="mb-6 flex flex-col items-center gap-3">
        {editing ? (
          <HotkeyRecorder
            value={settings.hotkey}
            platform={platform}
            onChange={(hotkey) => {
              update({ hotkey });
              setEditing(false);
              setTimeout(() => ref.current?.focus(), 0);
            }}
          />
        ) : (
          <>
            <HotkeyKeys accelerator={settings.hotkey} platform={platform} size="lg" />
            <button
              type="button"
              className="rounded-md text-[12px] text-subtle underline-offset-2 hover:text-fg hover:underline"
              onClick={() => setEditing(true)}
            >
              Changer de raccourci
            </button>
          </>
        )}
      </div>
      <div
        className={`relative rounded-[14px] border bg-surface shadow-soft transition-[border-color,box-shadow] duration-200 ${
          state.state === "recording"
            ? "border-accent shadow-[0_0_0_4px_var(--accent-soft)]"
            : "border-line focus-within:border-line-strong"
        }`}
      >
        <textarea
          ref={ref}
          rows={4}
          value={text}
          onChange={(e) => setText(e.target.value)}
          placeholder={`Maintenez ${label} et dites par exemple : « Bonjour, ceci est mon premier essai avec Murmure. »`}
          aria-label="Zone d'essai"
          className="block w-full resize-none rounded-[14px] bg-transparent px-4 pt-3.5 pb-9 text-[14px] leading-relaxed text-fg placeholder:text-subtle"
        />
        <div className="pointer-events-none absolute right-4 bottom-3 left-4 flex items-center justify-between text-[12px]">
          <span className={state.state === "error" ? "text-danger" : "text-accent"} aria-live="polite">
            {live}
            {state.state === "recording" && state.hands_free ? " · mains libres" : ""}
          </span>
          {success && state.state === "idle" && (
            <span className="inline-flex animate-fade-in items-center gap-1 text-success">
              <IconCheck size={13} /> Ça marche
            </span>
          )}
        </div>
      </div>
    </StepFrame>
  );
}
