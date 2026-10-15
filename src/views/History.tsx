// SPDX-License-Identifier: AGPL-3.0-or-later
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Button } from "../components/Button";
import { Card } from "../components/Card";
import { HistoryItem } from "../components/HistoryItem";
import { IconBook, IconCheck, IconDownload, IconHistory, IconSearch, IconTrash, IconX } from "../components/Icons";
import { PageHeader } from "../components/PageHeader";
import { api, errorMessage } from "../lib/api";
import { formatNumber, isSameDay } from "../lib/format";
import { usePlayer } from "../lib/player";
import type { CorrectionResult, ExportFormat, HistoryEntry, HistoryQuery, VocabularySuggestion } from "../lib/types";
import { useApp } from "../store";

const PAGE = 100;
const dayFmt = new Intl.DateTimeFormat("fr-FR", { weekday: "long", day: "numeric", month: "long" });
const yearFmt = new Intl.DateTimeFormat("fr-FR", { weekday: "long", day: "numeric", month: "long", year: "numeric" });

function dayLabel(d: Date): string {
  const now = new Date();
  if (isSameDay(d, now)) return "Aujourd'hui";
  const y = new Date(now);
  y.setDate(now.getDate() - 1);
  if (isSameDay(d, y)) return "Hier";
  const s = (d.getFullYear() === now.getFullYear() ? dayFmt : yearFmt).format(d);
  return s.charAt(0).toUpperCase() + s.slice(1);
}

const EXPORTS: { format: ExportFormat; label: string; text: string }[] = [
  { format: "csv", label: "Tableur (CSV)", text: "Une ligne par dictée, lisible dans Excel ou LibreOffice." },
  { format: "json", label: "Données (JSON)", text: "Toutes les informations, textes brut, collé et corrigé." },
  { format: "corpus", label: "Corpus audio et texte", text: "Un dossier : l'audio de chaque dictée et son texte corrigé." },
];

export function History() {
  const recent = useApp((s) => s.history);
  const historyEnabled = useApp((s) => s.settings?.history_enabled ?? true);
  const vocabulary = useApp((s) => s.settings?.initial_prompt ?? "");
  const replacements = useApp((s) => s.settings?.replacements);
  const deleteEntry = useApp((s) => s.deleteHistoryEntry);
  const clearHistory = useApp((s) => s.clearHistory);
  const upsert = useApp((s) => s.upsertHistoryEntry);
  const addVocabularyTerm = useApp((s) => s.addVocabularyTerm);
  const addReplacement = useApp((s) => s.addReplacement);
  const setView = useApp((s) => s.setView);
  const playerError = usePlayer((s) => s.error);

  const [text, setText] = useState("");
  const [from, setFrom] = useState("");
  const [to, setTo] = useState("");
  const [entries, setEntries] = useState<HistoryEntry[]>([]);
  const [total, setTotal] = useState(0);
  const [loaded, setLoaded] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [confirmClear, setConfirmClear] = useState(false);
  const [suggestions, setSuggestions] = useState<VocabularySuggestion[]>([]);
  const [exportOpen, setExportOpen] = useState(false);
  const [exporting, setExporting] = useState<ExportFormat | null>(null);
  const [exported, setExported] = useState<{ path: string } | { error: string } | null>(null);
  const seq = useRef(0);

  const query = useMemo<HistoryQuery>(() => ({ text, from: from || null, to: to || null }), [text, from, to]);
  const filtering = !!(text.trim() || from || to);

  const load = useCallback(
    async (offset = 0) => {
      const n = ++seq.current;
      try {
        const page = await api.searchHistory({ ...query, limit: PAGE, offset });
        if (n !== seq.current) return;
        setEntries((cur) => (offset ? [...cur, ...page.entries] : page.entries));
        setTotal(page.total);
        setError(null);
      } catch (e) {
        if (n === seq.current) setError(errorMessage(e));
      } finally {
        if (n === seq.current) setLoaded(true);
      }
    },
    [query],
  );

  // Recherche différée pendant la frappe ; nouvelle dictée (liste récente modifiée) : rechargement.
  useEffect(() => {
    const t = setTimeout(() => void load(0), text ? 180 : 0);
    return () => clearTimeout(t);
  }, [load, text, recent.length, recent[0]?.id]);

  const refreshSuggestions = useCallback(() => {
    api.getVocabularySuggestions().then(setSuggestions).catch(() => setSuggestions([]));
  }, []);
  useEffect(refreshSuggestions, [refreshSuggestions, vocabulary, replacements]);

  const groups = useMemo(() => {
    const out: { label: string; items: HistoryEntry[] }[] = [];
    for (const h of entries) {
      const label = dayLabel(new Date(h.created_at));
      const last = out[out.length - 1];
      if (last && last.label === label) last.items.push(h);
      else out.push({ label, items: [h] });
    }
    return out;
  }, [entries]);

  const onDelete = async (id: string) => {
    setEntries((cur) => cur.filter((h) => h.id !== id));
    setTotal((t) => Math.max(0, t - 1));
    await deleteEntry(id);
    refreshSuggestions();
  };

  const onCorrected = (r: CorrectionResult) => {
    setEntries((cur) => cur.map((h) => (h.id === r.entry.id ? r.entry : h)));
    upsert(r.entry);
    refreshSuggestions();
  };

  const runExport = async (format: ExportFormat) => {
    setExportOpen(false);
    setExporting(format);
    setExported(null);
    try {
      setExported({ path: await api.exportHistory(query, format) });
    } catch (e) {
      setExported({ error: errorMessage(e) });
    } finally {
      setExporting(null);
    }
  };

  const resetFilters = () => {
    setText("");
    setFrom("");
    setTo("");
  };

  const empty = loaded && total === 0 && !filtering;
  const count = filtering ? `${formatNumber(total)} résultat${total > 1 ? "s" : ""}` : `${formatNumber(total)} dictée${total > 1 ? "s" : ""}`;

  return (
    <div className="flex h-full flex-col">
      <PageHeader
        title="Historique"
        subtitle={total || filtering ? `${count} · cliquez sur une dictée pour la copier` : "Conservé uniquement sur cet appareil."}
        right={
          !empty &&
          (confirmClear ? (
            <div className="flex animate-fade-in items-center gap-2">
              <span className="text-[12.5px] text-muted">Tout effacer, audio compris ?</span>
              <Button size="sm" variant="ghost" onClick={() => setConfirmClear(false)}>
                Annuler
              </Button>
              <Button
                size="sm"
                variant="danger"
                onClick={async () => {
                  await clearHistory();
                  setConfirmClear(false);
                  resetFilters();
                  setEntries([]);
                  setTotal(0);
                  setSuggestions([]);
                }}
              >
                Effacer
              </Button>
            </div>
          ) : (
            <div className="relative flex items-center gap-1">
              <Button
                size="sm"
                variant="ghost"
                icon={<IconDownload size={14} />}
                loading={exporting !== null}
                aria-expanded={exportOpen}
                aria-haspopup="menu"
                onClick={() => setExportOpen((o) => !o)}
              >
                Exporter
              </Button>
              <Button size="sm" variant="ghost" icon={<IconTrash size={14} />} onClick={() => setConfirmClear(true)}>
                Tout effacer
              </Button>
              {exportOpen && (
                <ExportMenu filtering={filtering} onPick={(f) => void runExport(f)} onClose={() => setExportOpen(false)} />
              )}
            </div>
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

          {exported && (
            <div
              role="status"
              className={`mb-4 flex animate-fade-in items-center gap-3 rounded-[10px] border border-line px-4 py-2.5 text-[12.5px] ${
                "error" in exported ? "bg-danger-soft text-danger" : "bg-surface text-muted"
              }`}
            >
              {"error" in exported ? (
                <span className="flex-1">{exported.error}</span>
              ) : (
                <>
                  <IconCheck size={14} className="shrink-0 text-success" />
                  <span className="min-w-0 flex-1">
                    Export enregistré dans <span className="font-mono text-[12px] break-all text-fg">{exported.path}</span>
                  </span>
                  <Button size="sm" variant="secondary" onClick={() => void api.revealPath(exported.path).catch(() => undefined)}>
                    Afficher
                  </Button>
                </>
              )}
              <button type="button" aria-label="Fermer" className="text-subtle hover:text-fg" onClick={() => setExported(null)}>
                <IconX size={13} />
              </button>
            </div>
          )}

          {suggestions.length > 0 && (
            <Suggestions
              items={suggestions}
              onAdd={(s) => {
                addVocabularyTerm(s.term);
                setSuggestions((cur) => cur.filter((x) => x.term !== s.term));
              }}
              onRule={(s) => {
                for (const v of s.variants) addReplacement({ from: v, to: s.term });
                setSuggestions((cur) => cur.filter((x) => x.term !== s.term));
              }}
              onDismiss={(s) => {
                setSuggestions((cur) => cur.filter((x) => x.term !== s.term));
                void api.dismissVocabularySuggestion(s.term).catch(() => undefined);
              }}
            />
          )}

          {!empty && (
            <div className="sticky top-0 z-10 -mx-1 mb-4 bg-bg/85 px-1 pb-1 backdrop-blur-md">
              <div className="flex flex-wrap items-center gap-2">
                <div className="relative min-w-[220px] flex-1">
                  <IconSearch size={15} className="pointer-events-none absolute top-1/2 left-3 -translate-y-1/2 text-subtle" />
                  <input
                    type="search"
                    aria-label="Rechercher dans l'historique"
                    placeholder="Rechercher"
                    value={text}
                    onChange={(e) => setText(e.target.value)}
                    onKeyDown={(e) => e.key === "Escape" && setText("")}
                    className="field h-9 pr-9 pl-9 [&::-webkit-search-cancel-button]:hidden"
                  />
                  {text && (
                    <button
                      type="button"
                      aria-label="Effacer la recherche"
                      onClick={() => setText("")}
                      className="absolute top-1/2 right-2 inline-flex size-6 -translate-y-1/2 items-center justify-center rounded-md text-subtle hover:text-fg"
                    >
                      <IconX size={13} />
                    </button>
                  )}
                </div>
                <label className="flex items-center gap-1.5 text-[12.5px] text-muted">
                  Du
                  <input
                    type="date"
                    aria-label="À partir du"
                    value={from}
                    max={to || undefined}
                    onChange={(e) => setFrom(e.target.value)}
                    className="field h-9 w-[138px] px-2"
                  />
                </label>
                <label className="flex items-center gap-1.5 text-[12.5px] text-muted">
                  au
                  <input
                    type="date"
                    aria-label="Jusqu'au"
                    value={to}
                    min={from || undefined}
                    onChange={(e) => setTo(e.target.value)}
                    className="field h-9 w-[138px] px-2"
                  />
                </label>
                {filtering && (
                  <Button size="sm" variant="ghost" onClick={resetFilters}>
                    Réinitialiser
                  </Button>
                )}
              </div>
            </div>
          )}

          {(error || playerError) && (
            <div role="alert" className="mb-4 rounded-[10px] bg-danger-soft px-4 py-2.5 text-[12.5px] text-danger">
              {error ?? playerError}
            </div>
          )}

          {empty ? (
            <EmptyState />
          ) : loaded && entries.length === 0 ? (
            <div className="py-16 text-center text-[13px] text-muted">
              Aucun résultat{text.trim() ? ` pour « ${text.trim()} »` : ""}
              {from || to ? " sur cette période" : ""}.
            </div>
          ) : (
            <div className="grid gap-5">
              {groups.map((g) => (
                <section key={g.label} aria-label={g.label}>
                  <h2 className="mb-1.5 px-1 text-[12px] font-medium text-subtle">{g.label}</h2>
                  <Card className="p-1.5">
                    {g.items.map((h) => (
                      <HistoryItem
                        key={h.id}
                        entry={h}
                        onDelete={(id) => void onDelete(id)}
                        onCorrected={onCorrected}
                        onCreateRule={(c) => addReplacement({ from: c.wrong, to: c.right })}
                        highlight={text}
                      />
                    ))}
                  </Card>
                </section>
              ))}
              {entries.length < total && (
                <div className="flex justify-center">
                  <Button variant="secondary" onClick={() => void load(entries.length)}>
                    Afficher plus ({formatNumber(total - entries.length)} restantes)
                  </Button>
                </div>
              )}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

function ExportMenu({
  filtering,
  onPick,
  onClose,
}: {
  filtering: boolean;
  onPick: (f: ExportFormat) => void;
  onClose: () => void;
}) {
  const ref = useRef<HTMLDivElement | null>(null);
  useEffect(() => {
    const onDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) onClose();
    };
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [onClose]);
  return (
    <div
      ref={ref}
      role="menu"
      className="absolute top-9 right-0 z-30 w-[300px] animate-fade-in rounded-xl border border-line bg-surface p-1.5 shadow-card"
    >
      <p className="px-2.5 pt-1.5 pb-1 text-[11.5px] text-subtle">
        {filtering ? "Exporte les dictées affichées par la recherche" : "Exporte tout l'historique"}, dans Documents › Murmure.
      </p>
      {EXPORTS.map((e) => (
        <button
          key={e.format}
          type="button"
          role="menuitem"
          onClick={() => onPick(e.format)}
          className="block w-full rounded-[8px] px-2.5 py-2 text-left transition-colors hover:bg-surface-2"
        >
          <span className="block text-[13px] font-medium text-fg">{e.label}</span>
          <span className="block text-[12px] text-muted">{e.text}</span>
        </button>
      ))}
    </div>
  );
}

function Suggestions({
  items,
  onAdd,
  onRule,
  onDismiss,
}: {
  items: VocabularySuggestion[];
  onAdd: (s: VocabularySuggestion) => void;
  onRule: (s: VocabularySuggestion) => void;
  onDismiss: (s: VocabularySuggestion) => void;
}) {
  return (
    <Card className="mb-5 animate-fade-in overflow-hidden">
      <div className="flex items-start gap-2.5 border-b border-line px-4 py-3">
        <IconBook size={15} className="mt-0.5 shrink-0 text-accent" />
        <div>
          <h2 className="text-[13px] font-semibold text-fg">Termes souvent corrigés</h2>
          <p className="mt-0.5 text-[12.5px] text-muted">
            Ajoutés au vocabulaire, ils aident Whisper à les écrire correctement. Rien n'est ajouté sans votre accord.
          </p>
        </div>
      </div>
      <ul className="divide-y divide-line">
        {items.slice(0, 6).map((s) => (
          <li key={s.term} className="flex flex-wrap items-center gap-x-3 gap-y-2 px-4 py-2.5">
            <div className="min-w-0 flex-1">
              <span className="text-[13.5px] font-medium text-fg">{s.term}</span>
              <span className="ml-2 text-[12px] text-subtle">
                corrigé {s.count} fois
                {s.variants.length > 0 && <> au lieu de {s.variants.slice(0, 3).map((v) => `« ${v} »`).join(", ")}</>}
              </span>
            </div>
            <Button size="sm" variant="primary" onClick={() => onAdd(s)}>
              Ajouter au vocabulaire
            </Button>
            {s.variants.length > 0 && (
              <Button
                size="sm"
                variant="ghost"
                title={s.variants.map((v) => `${v} sera remplacé par ${s.term}`).join("\n")}
                onClick={() => onRule(s)}
              >
                Créer une règle
              </Button>
            )}
            <Button size="sm" variant="ghost" onClick={() => onDismiss(s)}>
              Ignorer
            </Button>
          </li>
        ))}
      </ul>
    </Card>
  );
}

function EmptyState() {
  return (
    <div className="flex flex-col items-center py-20 text-center">
      <div className="mb-4 flex size-12 items-center justify-center rounded-2xl border border-line bg-surface text-subtle shadow-soft">
        <IconHistory size={22} />
      </div>
      <p className="text-[14px] font-medium text-fg">Aucune dictée pour l'instant</p>
      <p className="mt-1 max-w-[320px] text-[12.5px] text-muted">
        Chaque texte dicté est conservé ici, sur votre appareil, pour le retrouver, le copier ou le corriger.
      </p>
    </div>
  );
}
