// SPDX-License-Identifier: AGPL-3.0-or-later
// Réglages de la ponctuation dictée : ponctuation de Whisper, majuscules, table des signes
// (intégrés + personnels), import/export du fichier d'instructions.
import { useState } from "react";
import { api, errorMessage } from "../lib/api";
import type { CustomMark, PunctuationRules, WhisperPunctuation } from "../lib/types";
import { useApp } from "../store";
import { Button, IconButton } from "./Button";
import { Row } from "./Card";
import { IconArrowRight, IconChevronDown, IconDownload, IconUpload, IconX } from "./Icons";
import { Segmented } from "./Segmented";
import { Switch } from "./Switch";

/** Signes intégrés (miroir de `punctuation.rs`, pour information). */
const BUILT_IN: [string, string][] = [
  ["virgule", ","],
  ["point", "."],
  ["point-virgule", ";"],
  ["deux points", ":"],
  ["point d'interrogation", "?"],
  ["point d'exclamation", "!"],
  ["points de suspension", "…"],
  ["ouvrez / fermez les guillemets", "« »"],
  ["ouvrez / fermez la parenthèse", "( )"],
  ["tiret", "–"],
  ["trait d'union", "-"],
  ["à la ligne", "retour à la ligne"],
  ["point à la ligne", ". + retour à la ligne"],
  ["nouveau paragraphe", "ligne vide"],
  ["point nouveau paragraphe", ". + ligne vide"],
];

const WHISPER_OPTIONS: { value: WhisperPunctuation; label: string }[] = [
  { value: "keep", label: "Garder" },
  { value: "ignore_when_dictated", label: "Si je dicte" },
  { value: "ignore", label: "Toujours" },
];

const WHISPER_HELP: Record<WhisperPunctuation, string> = {
  keep: "Les points et virgules que Whisper ajoute aux pauses restent dans le texte.",
  ignore_when_dictated:
    "Dès que vous prononcez un signe dans une dictée, ceux que Whisper a ajoutés de lui-même sont retirés, avec la majuscule qu'il mettait derrière.",
  ignore: "Seule la ponctuation prononcée est écrite, dans toutes les dictées.",
};

/** Affichage d'un signe : les retours à la ligne deviennent lisibles. */
function showSign(sign: string): string {
  return sign.replace(/\n/g, "↵");
}

export function PunctuationSettings() {
  const settings = useApp((s) => s.settings)!;
  const update = useApp((s) => s.updateSettings);
  const rules = settings.punctuation;
  const [spoken, setSpoken] = useState("");
  const [sign, setSign] = useState("");
  const [showBuiltIn, setShowBuiltIn] = useState(false);
  const [message, setMessage] = useState<{ ok: boolean; text: string } | null>(null);

  const set = (patch: Partial<PunctuationRules>) => update({ punctuation: { ...rules, ...patch } });

  const add = () => {
    const s = spoken.trim();
    if (!s || !sign.trim()) return;
    const mark: CustomMark = { spoken: s, sign: sign.trim().replace(/\\n/g, "\n") };
    set({ custom: [...rules.custom.filter((m) => m.spoken.toLowerCase() !== s.toLowerCase()), mark] });
    setSpoken("");
    setSign("");
  };

  const importFile = async () => {
    setMessage(null);
    try {
      const imported = await api.importPunctuation();
      if (imported) {
        update({ punctuation: imported });
        setMessage({ ok: true, text: `Fichier importé : ${imported.custom.length} signe(s) personnel(s).` });
      }
    } catch (e) {
      setMessage({ ok: false, text: errorMessage(e) });
    }
  };

  const exportFile = async () => {
    setMessage(null);
    try {
      const path = await api.exportPunctuation();
      if (path) setMessage({ ok: true, text: `Exporté dans ${path}` });
    } catch (e) {
      setMessage({ ok: false, text: errorMessage(e) });
    }
  };

  return (
    <>
      <Row label="Ponctuation ajoutée par Whisper" description={WHISPER_HELP[rules.whisper_punctuation]} align="start">
        <Segmented
          label="Ignorer la ponctuation de Whisper"
          value={rules.whisper_punctuation}
          options={WHISPER_OPTIONS}
          onChange={(whisper_punctuation) => set({ whisper_punctuation })}
          className="w-[300px] whitespace-nowrap"
        />
      </Row>
      <Row
        label="Majuscule après un retour à la ligne"
        htmlFor="cap-newline"
        description="Après « à la ligne » ou « nouveau paragraphe ». Après « point à la ligne », la majuscule reste toujours."
      >
        <Switch
          id="cap-newline"
          label="Majuscule après un retour à la ligne"
          checked={rules.capitalize_after_newline}
          onChange={(capitalize_after_newline) => set({ capitalize_after_newline })}
        />
      </Row>
      <Row
        label="Pas de majuscule après une virgule"
        htmlFor="lower-comma"
        description="Ni après un point-virgule ou deux-points, même après une pause. Les noms propres probables (Durand, RGPD) gardent leur majuscule."
      >
        <Switch
          id="lower-comma"
          label="Pas de majuscule après une virgule"
          checked={rules.lowercase_after_comma}
          onChange={(lowercase_after_comma) => set({ lowercase_after_comma })}
        />
      </Row>
      <div className="px-4 py-3.5">
        <div className="flex items-center justify-between gap-3">
          <div className="text-[13px] font-medium text-fg">Table de ponctuation</div>
          <div className="flex gap-1.5">
            <Button size="sm" variant="secondary" onClick={() => void importFile()}>
              <IconUpload size={13} /> Importer un fichier
            </Button>
            <Button size="sm" variant="secondary" onClick={() => void exportFile()}>
              <IconDownload size={13} /> Exporter
            </Button>
          </div>
        </div>
        <p className="mt-0.5 mb-2.5 text-[12.5px] text-muted">
          Ajoutez vos propres mots : « esperluette » → &amp;, « paragraphe » → §… Ils passent avant les signes intégrés. Le fichier
          exporté (texte) contient aussi les règles ci-dessus : modifiez-le puis réimportez-le, ou partagez-le avec vos confrères.
        </p>
        {message && (
          <p role="status" className={`mb-2.5 text-[12.5px] ${message.ok ? "text-muted" : "text-danger"}`}>
            {message.text}
          </p>
        )}
        {rules.custom.length > 0 && (
          <ul className="mb-2.5 divide-y divide-line rounded-[10px] border border-line">
            {rules.custom.map((m) => (
              <li key={m.spoken} className="flex items-center gap-2 px-3 py-1.5 text-[13px]">
                <span className="min-w-0 flex-1 truncate text-muted">« {m.spoken} »</span>
                <IconArrowRight size={13} className="shrink-0 text-subtle" />
                <span className="min-w-0 flex-1 truncate font-mono font-medium text-fg">{showSign(m.sign)}</span>
                <IconButton
                  label={`Supprimer « ${m.spoken} »`}
                  onClick={() => set({ custom: rules.custom.filter((x) => x.spoken !== m.spoken) })}
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
            aria-label="Mot prononcé"
            placeholder="Je dis : esperluette"
            value={spoken}
            onChange={(e) => setSpoken(e.target.value)}
            className="field h-8 min-w-0 flex-1"
          />
          <IconArrowRight size={13} className="shrink-0 text-subtle" />
          <input
            aria-label="Signe écrit"
            placeholder="Écrire : &"
            value={sign}
            onChange={(e) => setSign(e.target.value)}
            className="field h-8 w-28 font-mono"
          />
          <Button size="sm" type="submit" disabled={!spoken.trim() || !sign.trim()}>
            Ajouter
          </Button>
        </form>
        <button
          type="button"
          className="mt-3 flex items-center gap-1 text-[12.5px] text-muted hover:text-fg"
          aria-expanded={showBuiltIn}
          onClick={() => setShowBuiltIn((v) => !v)}
        >
          <IconChevronDown size={13} className={`transition-transform ${showBuiltIn ? "" : "-rotate-90"}`} />
          Signes intégrés
        </button>
        {showBuiltIn && (
          <ul className="mt-2 grid grid-cols-2 gap-x-4 gap-y-1 text-[12.5px]">
            {BUILT_IN.map(([s, g]) => (
              <li key={s} className="flex justify-between gap-2">
                <span className="text-muted">« {s} »</span>
                <span className="font-mono text-fg">{g}</span>
              </li>
            ))}
          </ul>
        )}
      </div>
    </>
  );
}
