import { useEffect, useState } from "react";
import { api, errorMessage } from "../lib/api";
import { useApp } from "../store";
import { Button } from "./Button";
import { IconAlert, IconCheck, IconEye, IconEyeOff, IconLock } from "./Icons";

type TestState = { kind: "idle" } | { kind: "testing" } | { kind: "ok"; message: string } | { kind: "error"; message: string };

/** Endpoint + modèle + clé API (trousseau) + test de connexion. */
export function RemoteForm({ compact = false }: { compact?: boolean }) {
  const settings = useApp((s) => s.settings)!;
  const updateSettings = useApp((s) => s.updateSettings);
  const hasApiKey = useApp((s) => s.hasApiKey);
  const refreshApiKey = useApp((s) => s.refreshApiKey);

  const [key, setKey] = useState("");
  const [showKey, setShowKey] = useState(false);
  const [keyStatus, setKeyStatus] = useState<"idle" | "saving" | "saved" | "error">("idle");
  const [test, setTest] = useState<TestState>({ kind: "idle" });

  // Enregistre la clé dans le trousseau après une courte pause de frappe.
  useEffect(() => {
    if (!key) return;
    const t = setTimeout(async () => {
      setKeyStatus("saving");
      try {
        await api.setApiKey(key.trim());
        await refreshApiKey();
        setKeyStatus("saved");
      } catch {
        setKeyStatus("error");
      }
    }, 700);
    return () => clearTimeout(t);
  }, [key, refreshApiKey]);

  const runTest = async () => {
    setTest({ kind: "testing" });
    try {
      const msg = await api.testRemote(settings.remote_endpoint.trim(), key.trim() || null, settings.remote_model.trim());
      setTest({ kind: "ok", message: msg });
    } catch (e) {
      setTest({ kind: "error", message: errorMessage(e) });
    }
  };

  const removeKey = async () => {
    await api.setApiKey("");
    setKey("");
    setKeyStatus("idle");
    await refreshApiKey();
  };

  const label = "mb-1.5 block text-[12.5px] font-medium text-fg";

  return (
    <div className={`grid gap-4 ${compact ? "" : "p-4"}`}>
      <div>
        <label htmlFor="remote-endpoint" className={label}>
          Adresse du serveur
        </label>
        <input
          id="remote-endpoint"
          className="field font-mono text-[12.5px]"
          placeholder="https://stt.exemple.fr/v1/audio/transcriptions"
          spellCheck={false}
          autoComplete="off"
          value={settings.remote_endpoint}
          onChange={(e) => {
            setTest({ kind: "idle" });
            updateSettings({ remote_endpoint: e.target.value }, 600);
          }}
        />
        <p className="mt-1.5 text-[12px] text-subtle">
          Compatible OpenAI : serveur Murmure, OpenAI, Groq ou tout service équivalent.
        </p>
      </div>

      <div className={`grid gap-4 ${compact ? "" : "sm:grid-cols-2"}`}>
        <div>
          <label htmlFor="remote-model" className={label}>
            Modèle
          </label>
          <input
            id="remote-model"
            className="field font-mono text-[12.5px]"
            placeholder="whisper-large-v3-turbo"
            spellCheck={false}
            autoComplete="off"
            value={settings.remote_model}
            onChange={(e) => {
              setTest({ kind: "idle" });
              updateSettings({ remote_model: e.target.value }, 600);
            }}
          />
        </div>
        <div>
          <label htmlFor="remote-key" className={label}>
            Clé API
          </label>
          <div className="relative">
            <input
              id="remote-key"
              className="field pr-9 font-mono text-[12.5px]"
              type={showKey ? "text" : "password"}
              placeholder={hasApiKey ? "••••••••••••••••" : "sk-…"}
              spellCheck={false}
              autoComplete="off"
              value={key}
              onChange={(e) => {
                setTest({ kind: "idle" });
                setKeyStatus("idle");
                setKey(e.target.value);
              }}
            />
            <button
              type="button"
              className="absolute top-1/2 right-1.5 inline-flex size-6 -translate-y-1/2 items-center justify-center rounded-md text-subtle hover:text-fg"
              aria-label={showKey ? "Masquer la clé" : "Afficher la clé"}
              onClick={() => setShowKey((v) => !v)}
            >
              {showKey ? <IconEyeOff size={14} /> : <IconEye size={14} />}
            </button>
          </div>
          <div className="mt-1.5 flex items-center gap-1.5 text-[12px] text-subtle">
            <IconLock size={12} />
            {keyStatus === "saving"
              ? "Enregistrement…"
              : keyStatus === "error"
                ? "Impossible d'accéder au trousseau"
                : hasApiKey
                  ? "Enregistrée dans le trousseau"
                  : "Stockée dans le trousseau, jamais en clair"}
            {hasApiKey && !key && (
              <button type="button" onClick={removeKey} className="ml-auto text-subtle underline-offset-2 hover:text-danger hover:underline">
                Retirer
              </button>
            )}
          </div>
        </div>
      </div>

      <div className="flex min-h-8 flex-wrap items-center gap-3">
        <Button
          onClick={runTest}
          loading={test.kind === "testing"}
          disabled={!settings.remote_endpoint.trim()}
        >
          Tester la connexion
        </Button>
        {test.kind === "ok" && (
          <span className="inline-flex animate-fade-in items-center gap-1.5 text-[12.5px] text-success">
            <IconCheck size={14} /> {test.message}
          </span>
        )}
        {test.kind === "error" && (
          <span className="inline-flex min-w-0 animate-fade-in items-center gap-1.5 text-[12.5px] text-danger">
            <IconAlert size={14} className="shrink-0" /> <span className="truncate">{test.message}</span>
          </span>
        )}
      </div>
    </div>
  );
}
