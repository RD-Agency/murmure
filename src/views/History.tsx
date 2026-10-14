import { useMemo, useState } from "react";
import { Button } from "../components/Button";
import { Card } from "../components/Card";
import { HistoryItem } from "../components/HistoryItem";
import { IconHistory, IconSearch, IconTrash, IconX } from "../components/Icons";
import { PageHeader } from "../components/PageHeader";
import { formatNumber, isSameDay, normalizeSearch } from "../lib/format";
import type { HistoryEntry } from "../lib/types";
import { useApp } from "../store";

const dayFmt = new Intl.DateTimeFormat("fr-FR", { weekday: "long", day: "numeric", month: "long" });

function dayLabel(d: Date): string {
  const now = new Date();
  if (isSameDay(d, now)) return "Aujourd'hui";
  const y = new Date(now);
  y.setDate(now.getDate() - 1);
  if (isSameDay(d, y)) return "Hier";
  const s = dayFmt.format(d);
  return s.charAt(0).toUpperCase() + s.slice(1);
}

export function History() {
  const history = useApp((s) => s.history);
  const historyEnabled = useApp((s) => s.settings?.history_enabled ?? true);
  const deleteEntry = useApp((s) => s.deleteHistoryEntry);
  const clearHistory = useApp((s) => s.clearHistory);
  const setView = useApp((s) => s.setView);
  const [query, setQuery] = useState("");
  const [confirmClear, setConfirmClear] = useState(false);

  const filtered = useMemo(() => {
    const q = normalizeSearch(query.trim());
    if (!q) return history;
    return history.filter((h) => normalizeSearch(h.text).includes(q));
  }, [history, query]);

  const groups = useMemo(() => {
    const out: { label: string; items: HistoryEntry[] }[] = [];
    for (const h of filtered) {
      const label = dayLabel(new Date(h.created_at));
      const last = out[out.length - 1];
      if (last && last.label === label) last.items.push(h);
      else out.push({ label, items: [h] });
    }
    return out;
  }, [filtered]);

  return (
    <div className="flex h-full flex-col">
      <PageHeader
        title="Historique"
        subtitle={
          history.length
            ? `${formatNumber(history.length)} dictée${history.length > 1 ? "s" : ""} · cliquez sur une dictée pour la copier`
            : "Conservé uniquement sur cet appareil."
        }
        right={
          history.length > 0 &&
          (confirmClear ? (
            <div className="flex animate-fade-in items-center gap-2">
              <span className="text-[12.5px] text-muted">Tout effacer définitivement ?</span>
              <Button size="sm" variant="ghost" onClick={() => setConfirmClear(false)}>
                Annuler
              </Button>
              <Button
                size="sm"
                variant="danger"
                onClick={async () => {
                  await clearHistory();
                  setConfirmClear(false);
                  setQuery("");
                }}
              >
                Effacer
              </Button>
            </div>
          ) : (
            <Button size="sm" variant="ghost" icon={<IconTrash size={14} />} onClick={() => setConfirmClear(true)}>
              Tout effacer
            </Button>
          ))
        }
      />

      <div className="flex-1 overflow-y-auto px-10 pb-10">
        <div className="mx-auto max-w-[760px]">
          {!historyEnabled && (
            <div className="mb-4 flex items-center justify-between gap-3 rounded-[10px] border border-line bg-warning-soft px-4 py-2.5 text-[12.5px] text-warning">
              L'historique est désactivé : les nouvelles dictées ne sont pas conservées.
              <button type="button" className="font-medium underline-offset-2 hover:underline" onClick={() => setView("settings")}>
                Réglages
              </button>
            </div>
          )}

          {history.length > 0 && (
            <div className="sticky top-0 z-10 -mx-1 mb-4 bg-bg/85 px-1 pb-1 backdrop-blur-md">
              <div className="relative">
                <IconSearch size={15} className="pointer-events-none absolute top-1/2 left-3 -translate-y-1/2 text-subtle" />
                <input
                  type="search"
                  aria-label="Rechercher dans l'historique"
                  placeholder="Rechercher"
                  value={query}
                  onChange={(e) => setQuery(e.target.value)}
                  onKeyDown={(e) => e.key === "Escape" && setQuery("")}
                  className="field h-9 pr-9 pl-9 [&::-webkit-search-cancel-button]:hidden"
                />
                {query && (
                  <button
                    type="button"
                    aria-label="Effacer la recherche"
                    onClick={() => setQuery("")}
                    className="absolute top-1/2 right-2 inline-flex size-6 -translate-y-1/2 items-center justify-center rounded-md text-subtle hover:text-fg"
                  >
                    <IconX size={13} />
                  </button>
                )}
              </div>
            </div>
          )}

          {history.length === 0 ? (
            <EmptyState />
          ) : filtered.length === 0 ? (
            <div className="py-16 text-center text-[13px] text-muted">Aucun résultat pour « {query.trim()} ».</div>
          ) : (
            <div className="grid gap-5">
              {groups.map((g) => (
                <section key={g.label} aria-label={g.label}>
                  <h2 className="mb-1.5 px-1 text-[12px] font-medium text-subtle">{g.label}</h2>
                  <Card className="p-1.5">
                    {g.items.map((h) => (
                      <HistoryItem key={h.id} entry={h} onDelete={deleteEntry} highlight={query} />
                    ))}
                  </Card>
                </section>
              ))}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

function EmptyState() {
  return (
    <div className="flex flex-col items-center py-20 text-center">
      <div className="mb-4 flex size-12 items-center justify-center rounded-2xl border border-line bg-surface text-subtle shadow-soft">
        <IconHistory size={22} />
      </div>
      <p className="text-[14px] font-medium text-fg">Aucune dictée pour l'instant</p>
      <p className="mt-1 max-w-[300px] text-[12.5px] text-muted">
        Chaque texte dicté est conservé ici, sur votre appareil, pour pouvoir le retrouver et le copier.
      </p>
    </div>
  );
}
