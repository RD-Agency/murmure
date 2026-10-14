import { useEffect } from "react";
import { Sidebar } from "./components/Sidebar";
import { useApp } from "./store";
import { History } from "./views/History";
import { Home } from "./views/Home";
import { Onboarding } from "./views/Onboarding";
import { Settings } from "./views/Settings";

export function App() {
  const ready = useApp((s) => s.ready);
  const settings = useApp((s) => s.settings);
  const view = useApp((s) => s.view);
  const init = useApp((s) => s.init);
  const flush = useApp((s) => s.flushSettings);

  useEffect(() => {
    void init();
    const onHide = () => void flush();
    window.addEventListener("beforeunload", onHide);
    window.addEventListener("blur", onHide);
    return () => {
      window.removeEventListener("beforeunload", onHide);
      window.removeEventListener("blur", onHide);
    };
  }, [init, flush]);

  if (!ready || !settings) {
    return <div data-tauri-drag-region className="h-full" />;
  }

  if (!settings.onboarding_done) {
    return <Onboarding />;
  }

  return (
    <div className="flex h-full animate-fade-in">
      <Sidebar />
      <main className="min-w-0 flex-1 bg-bg">
        <div key={view} className="h-full animate-fade-in">
          {view === "home" && <Home />}
          {view === "history" && <History />}
          {view === "settings" && <Settings />}
        </div>
      </main>
    </div>
  );
}
