// SPDX-License-Identifier: AGPL-3.0-or-later
import { useMemo } from "react";
import { Card } from "../components/Card";
import { EngineStatus, useEngineReady } from "../components/EngineStatus";
import { HistoryItem } from "../components/HistoryItem";
import { IconArrowRight, IconMic } from "../components/Icons";
import { InstallCard, useActiveInstall } from "../components/InstallCard";
import { HotkeyKeys } from "../components/Kbd";
import { PageHeader } from "../components/PageHeader";
import { UpdateBanner } from "../components/UpdateBanner";
import { countWords, formatLatency, formatNumber, isSameDay } from "../lib/format";
import { useApp } from "../store";

const MODE_HINT = {
  hybrid: "Maintenez pour dicter, ou appuyez brièvement pour passer en mains libres.",
  hold: "Maintenez le raccourci pendant que vous parlez, relâchez pour insérer le texte.",
  toggle: "Appuyez une fois pour commencer, une seconde fois pour insérer le texte.",
} as const;

export function Home() {
  const settings = useApp((s) => s.settings)!;
  const platform = useApp((s) => s.platform);
  const history = useApp((s) => s.history);
  const setView = useApp((s) => s.setView);
  const ready = useEngineReady();
  const installing = !!useActiveInstall() && !ready;

  const stats = useMemo(() => {
    const today = new Date();
    let wordsToday = 0;
    let wordsTotal = 0;
    let latency = 0;
    for (const h of history) {
      const w = countWords(h.text);
      wordsTotal += w;
      latency += h.latency_ms;
      if (isSameDay(new Date(h.created_at), today)) wordsToday += w;
    }
    return {
      wordsToday,
      wordsTotal,
      avgLatency: history.length ? latency / history.length : null,
      count: history.length,
    };
  }, [history]);

  const recent = history.slice(0, 4);

  return (
    <div className="flex h-full flex-col">
      <PageHeader title="Accueil" subtitle="Dictez dans n'importe quelle application." />
      <div className="flex-1 overflow-y-auto px-10 pb-10">
        <div className="mx-auto grid max-w-[760px] gap-5">
          {/* Mise à jour disponible / en cours */}
          <UpdateBanner />

          {/* Raccourci + moteur */}
          <Card className="overflow-hidden">
            <div className="p-6">
              <div
                className={`mb-4 flex items-center gap-2 text-[12px] font-medium ${ready || installing ? "text-accent" : "text-warning"}`}
              >
                <IconMic size={14} />
                {ready ? "Prêt à dicter" : installing ? "Prêt dans un instant" : "Configuration requise"}
              </div>
              <div className="flex flex-wrap items-center gap-x-8 gap-y-4">
                <HotkeyKeys accelerator={settings.hotkey} platform={platform} size="lg" />
                <p className="max-w-[340px] flex-1 text-[13px] leading-relaxed text-muted">{MODE_HINT[settings.hotkey_mode]}</p>
              </div>
            </div>
            <div className="border-t border-line bg-surface-2/40 px-6 py-3.5">
              <EngineStatus />
            </div>
          </Card>

          {/* Installation automatique du modèle local */}
          <InstallCard />

          {/* Statistiques */}
          <div className="grid grid-cols-3 gap-3">
            <Stat label="Mots aujourd'hui" value={formatNumber(stats.wordsToday)} />
            <Stat label="Mots au total" value={formatNumber(stats.wordsTotal)} hint={`${formatNumber(stats.count)} dictées`} />
            <Stat
              label="Transcription moyenne"
              value={stats.avgLatency === null ? "—" : formatLatency(stats.avgLatency)}
            />
          </div>

          {/* Dernières dictées */}
          <section aria-labelledby="recent-title">
            <div className="mb-2 flex items-center justify-between px-1">
              <h2 id="recent-title" className="text-[13px] font-semibold text-fg">
                Dernières dictées
              </h2>
              {history.length > 0 && (
                <button
                  type="button"
                  onClick={() => setView("history")}
                  className="inline-flex items-center gap-1 rounded-md text-[12.5px] text-muted transition-colors hover:text-fg"
                >
                  Tout voir <IconArrowRight size={13} />
                </button>
              )}
            </div>
            <Card className="p-1.5">
              {recent.length ? (
                recent.map((h) => <HistoryItem key={h.id} entry={h} compact />)
              ) : (
                <div className="px-4 py-10 text-center">
                  <p className="text-[13px] text-muted">Vos dictées apparaîtront ici.</p>
                  <p className="mt-1 text-[12.5px] text-subtle">
                    {settings.history_enabled ? "Essayez maintenant, dans n'importe quel champ de texte." : "L'historique est désactivé dans les réglages."}
                  </p>
                </div>
              )}
            </Card>
          </section>
        </div>
      </div>
    </div>
  );
}

function Stat({ label, value, hint }: { label: string; value: string; hint?: string }) {
  return (
    <Card className="px-4 py-3.5">
      <div className="text-[12px] text-muted">{label}</div>
      <div className="mt-1 flex items-baseline gap-2">
        <span className="text-[22px] font-semibold tracking-[-0.02em] text-fg tabular-nums">{value}</span>
        {hint && <span className="text-[11.5px] text-subtle">{hint}</span>}
      </div>
    </Card>
  );
}
