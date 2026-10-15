// SPDX-License-Identifier: AGPL-3.0-or-later
import { useEffect, useState } from "react";
import { Group, Row, Section } from "../components/Card";
import { HotkeyRecorder } from "../components/HotkeyRecorder";
import {
  IconBook,
  IconClipboard,
  IconCpu,
  IconGlobe,
  IconKeyboard,
  IconLaptop,
  IconMic,
  IconServer,
  IconSliders,
} from "../components/Icons";
import { ModelList } from "../components/ModelList";
import { PageHeader } from "../components/PageHeader";
import { RemoteForm } from "../components/RemoteForm";
import { SaveIndicator } from "../components/SaveIndicator";
import { Segmented } from "../components/Segmented";
import { Select } from "../components/Select";
import { Switch } from "../components/Switch";
import { UpdateRow } from "../components/UpdateBanner";
import { useWayland, WaylandPanel } from "../components/WaylandPanel";
import { api } from "../lib/api";
import { LANGUAGES } from "../lib/format";
import type { AudioDevice, HotkeyMode } from "../lib/types";
import { useApp } from "../store";

const MODES: { value: HotkeyMode; title: string; text: string; recommended?: boolean }[] = [
  {
    value: "hybrid",
    title: "Hybride",
    text: "Maintenez pour dicter. Un appui bref passe en mains libres ; un second appui termine.",
    recommended: true,
  },
  { value: "hold", title: "Maintenir", text: "La dictée dure tant que le raccourci est enfoncé." },
  { value: "toggle", title: "Basculer", text: "Un appui pour commencer, un autre pour terminer." },
];

export function Settings() {
  const settings = useApp((s) => s.settings)!;
  const platform = useApp((s) => s.platform);
  const update = useApp((s) => s.updateSettings);
  const [devices, setDevices] = useState<AudioDevice[]>([]);
  const wayland = useWayland();

  useEffect(() => {
    api.listInputDevices().then(setDevices).catch(() => setDevices([]));
  }, []);

  const defaultDevice = devices.find((d) => d.is_default);
  const deviceMissing = settings.input_device !== null && !devices.some((d) => d.name === settings.input_device);

  return (
    <div className="flex h-full flex-col">
      <PageHeader title="Réglages" subtitle="Les modifications sont enregistrées automatiquement." right={<SaveIndicator />} />
      <div className="flex-1 overflow-y-auto px-10 pb-16">
        <div className="mx-auto grid max-w-[760px] gap-9">
          {/* Moteur */}
          <Section id="moteur" title="Moteur" icon={<IconCpu size={15} />}>
            <Group>
              <Row
                label="Transcription"
                description={
                  settings.engine === "local"
                    ? "Whisper tourne sur cet appareil : privé, hors ligne, rien ne quitte votre machine."
                    : "L'audio est envoyé à votre serveur de transcription, puis effacé."
                }
              >
                <Segmented
                  label="Moteur de transcription"
                  value={settings.engine}
                  onChange={(engine) => update({ engine })}
                  options={[
                    { value: "local", label: "Sur l'appareil", icon: <IconLaptop size={14} /> },
                    { value: "remote", label: "Serveur", icon: <IconServer size={14} /> },
                  ]}
                />
              </Row>
              <div key={settings.engine} className="animate-fade-in">
                {settings.engine === "local" ? <ModelList /> : <RemoteForm />}
              </div>
            </Group>
          </Section>

          {/* Langue + vocabulaire */}
          <Section id="langue" title="Langue et vocabulaire" icon={<IconGlobe size={15} />}>
            <Group>
              <Row
                label="Langue parlée"
                htmlFor="language"
                description="Fixer la langue est plus rapide et plus fiable que la détection automatique."
              >
                <Select id="language" value={settings.language} onChange={(e) => update({ language: e.target.value })} className="w-[210px]">
                  {LANGUAGES.map((l) => (
                    <option key={l.code} value={l.code}>
                      {l.label}
                    </option>
                  ))}
                </Select>
              </Row>
              {(settings.language === "fr" || settings.language === "auto") && (
                <Row
                  label="Ponctuation dictée"
                  htmlFor="spoken-punctuation"
                  description="Dites « virgule », « point », « point à la ligne », « deux points », « ouvrez les guillemets »… : Murmure écrit le signe."
                >
                  <Switch
                    id="spoken-punctuation"
                    label="Ponctuation dictée"
                    checked={settings.spoken_punctuation}
                    onChange={(spoken_punctuation) => update({ spoken_punctuation })}
                  />
                </Row>
              )}
              <div className="px-4 py-3.5">
                <label htmlFor="initial-prompt" className="flex items-center gap-1.5 text-[13px] font-medium text-fg">
                  <IconBook size={14} className="text-subtle" /> Vocabulaire
                </label>
                <p className="mt-0.5 mb-2.5 text-[12.5px] text-muted">
                  Noms propres, jargon, sigles : quelques mots ou une phrase dans le style souhaité aident Whisper à les orthographier.
                </p>
                <textarea
                  id="initial-prompt"
                  rows={3}
                  className="field resize-none leading-relaxed"
                  placeholder="Ex. : Murmure, Kubernetes, Mme Lefèvre, RGPD, SIRET."
                  value={settings.initial_prompt}
                  onChange={(e) => update({ initial_prompt: e.target.value }, 600)}
                />
              </div>
            </Group>
          </Section>

          {/* Raccourci */}
          <Section id="raccourci" title="Raccourci" icon={<IconKeyboard size={15} />}>
            <Group>
              {wayland ? (
                <WaylandPanel status={wayland} />
              ) : (
                <Row label="Raccourci global" description="Fonctionne dans toutes les applications." align="start">
                  <HotkeyRecorder value={settings.hotkey} platform={platform} onChange={(hotkey) => update({ hotkey })} />
                </Row>
              )}
              <div className="px-4 py-3.5">
                <div className="mb-2.5 text-[13px] font-medium text-fg" id="mode-label">
                  Comportement
                </div>
                <div role="radiogroup" aria-labelledby="mode-label" className="grid gap-2 sm:grid-cols-3">
                  {MODES.map((m) => {
                    const active = settings.hotkey_mode === m.value;
                    return (
                      <button
                        key={m.value}
                        type="button"
                        role="radio"
                        aria-checked={active}
                        onClick={() => update({ hotkey_mode: m.value })}
                        className={`rounded-[10px] border p-3 text-left transition-[border-color,background-color,box-shadow] ${
                          active
                            ? "border-accent bg-accent-soft/60 shadow-[0_0_0_3px_var(--accent-soft)]"
                            : "border-line-strong hover:border-subtle/50 hover:bg-surface-2/60"
                        }`}
                      >
                        <div className="flex items-center gap-2">
                          <span
                            className={`flex size-3.5 items-center justify-center rounded-full border ${
                              active ? "border-accent bg-accent" : "border-line-strong"
                            }`}
                          >
                            {active && <span className="size-1.5 rounded-full bg-accent-fg" />}
                          </span>
                          <span className="text-[13px] font-medium text-fg">{m.title}</span>
                          {m.recommended && <span className="text-[11px] text-accent">Recommandé</span>}
                        </div>
                        <p className="mt-1.5 text-[12px] leading-relaxed text-muted">{m.text}</p>
                      </button>
                    );
                  })}
                </div>
                <p className="mt-2.5 text-[12px] text-subtle">
                  {wayland
                    ? "Sous Wayland, Échap n'est pas capté hors de Murmure : terminez avec le raccourci, puis annulez avec Ctrl+Z si besoin."
                    : "Échap annule une dictée en cours."}
                </p>
              </div>
            </Group>
          </Section>

          {/* Micro */}
          <Section id="micro" title="Microphone" icon={<IconMic size={15} />}>
            <Group>
              <Row label="Périphérique d'entrée" htmlFor="input-device" description="Le micro utilisé pendant la dictée.">
                <Select
                  id="input-device"
                  className="w-[270px]"
                  value={settings.input_device ?? ""}
                  onChange={(e) => update({ input_device: e.target.value || null })}
                >
                  <option value="">
                    Par défaut{defaultDevice ? ` (${defaultDevice.name})` : ""}
                  </option>
                  {devices.map((d) => (
                    <option key={d.name} value={d.name}>
                      {d.name}
                    </option>
                  ))}
                  {deviceMissing && <option value={settings.input_device!}>{settings.input_device} (déconnecté)</option>}
                </Select>
              </Row>
            </Group>
          </Section>

          {/* Collage */}
          <Section id="collage" title="Insertion du texte" icon={<IconClipboard size={15} />}>
            <Group>
              <Row
                label="Après la dictée"
                description={
                  settings.paste_mode === "paste"
                    ? "Le texte est collé directement là où se trouve le curseur."
                    : "Le texte est copié dans le presse-papiers ; collez-le vous-même."
                }
              >
                <Segmented
                  label="Mode d'insertion"
                  value={settings.paste_mode}
                  onChange={(paste_mode) => update({ paste_mode })}
                  options={[
                    { value: "paste", label: "Coller" },
                    { value: "clipboard", label: "Copier seulement" },
                  ]}
                />
              </Row>
              <Row
                label="Restaurer le presse-papiers"
                htmlFor="restore-clipboard"
                description="Remet le contenu précédent du presse-papiers après le collage."
              >
                <Switch
                  id="restore-clipboard"
                  label="Restaurer le presse-papiers"
                  checked={settings.restore_clipboard}
                  disabled={settings.paste_mode !== "paste"}
                  onChange={(restore_clipboard) => update({ restore_clipboard })}
                />
              </Row>
            </Group>
          </Section>

          {/* Général */}
          <Section id="general" title="Général" icon={<IconSliders size={15} />}>
            <Group>
              <Row label="Sons" htmlFor="sounds" description="Un léger signal au début et à la fin de la dictée.">
                <Switch id="sounds" label="Sons" checked={settings.sounds} onChange={(sounds) => update({ sounds })} />
              </Row>
              <Row label="Lancer au démarrage" htmlFor="autostart" description="Murmure s'ouvre discrètement dans la barre de menus.">
                <Switch
                  id="autostart"
                  label="Lancer au démarrage"
                  checked={settings.launch_at_login}
                  onChange={(launch_at_login) => update({ launch_at_login })}
                />
              </Row>
              <Row label="Historique" htmlFor="history" description="Conserve les dictées sur cet appareil pour les retrouver.">
                <Switch
                  id="history"
                  label="Historique"
                  checked={settings.history_enabled}
                  onChange={(history_enabled) => update({ history_enabled })}
                />
              </Row>
              <UpdateRow />
            </Group>
          </Section>

        </div>
      </div>
    </div>
  );
}
