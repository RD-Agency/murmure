// SPDX-License-Identifier: AGPL-3.0-or-later
// Réglages › Historique : audio des dictées, plafond, corrections automatiques, règles de remplacement.
import { useCallback, useEffect, useState } from "react";
import { api, errorMessage } from "../lib/api";
import { formatBytes, formatDuration, formatNumber } from "../lib/format";
import type { AudioUsage } from "../lib/types";
import { useApp } from "../store";
import { Button, IconButton } from "./Button";
import { Group, Row } from "./Card";
import { IconArrowRight, IconX } from "./Icons";
import { Progress } from "./Progress";
import { Select } from "./Select";
import { Switch } from "./Switch";

const CAPS: { mb: number; label: string }[] = [
  { mb: 500, label: "500 Mo (environ 55 h)" },
  { mb: 1024, label: "1 Go (environ 110 h)" },
  { mb: 2048, label: "2 Go (environ 230 h)" },
  { mb: 5120, label: "5 Go (environ 570 h)" },
];

export function HistorySettings() {
  const settings = useApp((s) => s.settings)!;
  const update = useApp((s) => s.updateSettings);
  const [usage, setUsage] = useState<AudioUsage | null>(null);
  const [confirm, setConfirm] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(() => {
    api.getAudioUsage().then(setUsage).catch((e) => setError(errorMessage(e)));
  }, []);
  // Rechargé quand le plafond change (rotation côté Rust) ; petit délai pour la sauvegarde.
  useEffect(() => {
    const t = setTimeout(refresh, 300);
    return () => clearTimeout(t);
  }, [refresh, settings.audio_cap_mb, settings.keep_audio]);

  const cap = CAPS.some((c) => c.mb === settings.audio_cap_mb) ? settings.audio_cap_mb : 5120;

  return (
    <Group>
      <Row label="Historique" htmlFor="history" description="Conserve les dictées sur cet appareil pour les retrouver et les corriger.">
        <Switch id="history" label="Historique" checked={settings.history_enabled} onChange={(history_enabled) => update({ history_enabled })} />
      </Row>
      <Row
        label="Conserver l'audio des dictées"
        htmlFor="keep-audio"
        description="Pour réécouter une dictée avant de la corriger. Compressé (Opus), environ 9 Mo par heure de parole."
      >
        <Switch
          id="keep-audio"
          label="Conserver l'audio des dictées"
          checked={settings.keep_audio}
          disabled={!settings.history_enabled}
          onChange={(keep_audio) => update({ keep_audio })}
        />
      </Row>
      <Row
        label="Espace maximal pour l'audio"
        htmlFor="audio-cap"
        description="Au-delà, l'audio des dictées les plus anciennes est supprimé. Le texte est toujours conservé."
      >
        <Select id="audio-cap" className="w-[210px]" value={cap} onChange={(e) => update({ audio_cap_mb: Number(e.target.value) })}>
          {CAPS.map((c) => (
            <option key={c.mb} value={c.mb}>
              {c.label}
            </option>
          ))}
        </Select>
      </Row>
      <div className="px-4 py-3.5">
        <div className="flex items-center gap-4">
          <div className="min-w-0 flex-1">
            <div className="text-[13px] font-medium text-fg">Espace utilisé</div>
            <div className="mt-0.5 text-[12.5px] text-muted">
              {usage
                ? usage.files
                  ? `${formatBytes(usage.bytes)} sur ${formatBytes(usage.cap_bytes)} · ${formatNumber(usage.files)} dictée${usage.files > 1 ? "s" : ""}, ${formatDuration(usage.duration_ms)} d'audio`
                  : "Aucun audio conservé."
                : "Calcul en cours."}
            </div>
          </div>
          {confirm ? (
            <div className="flex animate-fade-in items-center gap-2">
              <span className="text-[12.5px] text-muted">Les textes restent.</span>
              <Button size="sm" variant="ghost" onClick={() => setConfirm(false)}>
                Annuler
              </Button>
              <Button
                size="sm"
                variant="danger"
                loading={busy}
                onClick={async () => {
                  setBusy(true);
                  try {
                    await api.deleteAllAudio();
                    setError(null);
                  } catch (e) {
                    setError(errorMessage(e));
                  }
                  setBusy(false);
                  setConfirm(false);
                  refresh();
                }}
              >
                Supprimer
              </Button>
            </div>
          ) : (
            <Button size="sm" variant="secondary" disabled={!usage?.files} onClick={() => setConfirm(true)}>
              Supprimer les audios
            </Button>
          )}
        </div>
        {usage && usage.files > 0 && (
          <div className="mt-2.5">
            <Progress value={usage.bytes / usage.cap_bytes} label="Espace utilisé par l'audio" />
          </div>
        )}
        {error && <p className="mt-2 text-[12px] text-danger">{error}</p>}
      </div>
    </Group>
  );
}

/** Corrections automatiques et règles de remplacement (section « Langue et vocabulaire »). */
export function CorrectionSettings() {
  const settings = useApp((s) => s.settings)!;
  const update = useApp((s) => s.updateSettings);
  const addReplacement = useApp((s) => s.addReplacement);
  const [from, setFrom] = useState("");
  const [to, setTo] = useState("");
  const rules = settings.replacements;

  const add = () => {
    if (!from.trim() || !to.trim()) return;
    addReplacement({ from, to });
    setFrom("");
    setTo("");
  };

  return (
    <>
      <Row
        label="Corriger les mots douteux"
        htmlFor="auto-correct"
        description={
          settings.engine === "local"
            ? "Quand Whisper hésite sur un mot proche d'un terme du vocabulaire, Murmure écrit ce terme. Les mots bien reconnus ne sont jamais modifiés."
            : "Disponible avec le moteur sur l'appareil, qui indique la confiance de chaque mot."
        }
      >
        <Switch id="auto-correct" label="Corriger les mots douteux" checked={settings.auto_correct} onChange={(auto_correct) => update({ auto_correct })} />
      </Row>
      <div className="px-4 py-3.5">
        <div className="text-[13px] font-medium text-fg">Règles de remplacement</div>
        <p className="mt-0.5 mb-2.5 text-[12.5px] text-muted">
          Une expression toujours mal transcrite est remplacée après chaque dictée, en mots entiers et sans tenir compte des
          majuscules. Créez-les ici ou depuis une correction dans l'historique.
        </p>
        {rules.length > 0 && (
          <ul className="mb-2.5 divide-y divide-line rounded-[10px] border border-line">
            {rules.map((r) => (
              <li key={r.from} className="flex items-center gap-2 px-3 py-1.5 text-[13px]">
                <span className="min-w-0 flex-1 truncate text-muted">{r.from}</span>
                <IconArrowRight size={13} className="shrink-0 text-subtle" />
                <span className="min-w-0 flex-1 truncate font-medium text-fg">{r.to}</span>
                <IconButton
                  label={`Supprimer la règle « ${r.from} »`}
                  onClick={() => update({ replacements: rules.filter((x) => x.from !== r.from) })}
                >
                  <IconX size={13} />
                </IconButton>
              </li>
            ))}
          </ul>
        )}
        <form
          className="flex items-center gap-2"
          onSubmit={(e) => {
            e.preventDefault();
            add();
          }}
        >
          <input
            aria-label="Expression transcrite"
            placeholder="Transcrit : L 12 32 2"
            value={from}
            onChange={(e) => setFrom(e.target.value)}
            className="field h-8 min-w-0 flex-1"
          />
          <IconArrowRight size={13} className="shrink-0 text-subtle" />
          <input
            aria-label="Remplacement"
            placeholder="Écrire : L. 1232-2"
            value={to}
            onChange={(e) => setTo(e.target.value)}
            className="field h-8 min-w-0 flex-1"
          />
          <Button size="sm" type="submit" disabled={!from.trim() || !to.trim()}>
            Ajouter
          </Button>
        </form>
      </div>
    </>
  );
}
