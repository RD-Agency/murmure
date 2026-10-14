import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { isMock, loadMock } from "./lib/api";
import "./styles.css";

if (isMock) {
  // Mode démo navigateur : le raccourci est simulé dans la page.
  void loadMock().then((m) => m.installMockHotkey());
  const theme = new URLSearchParams(location.search).get("theme");
  if (theme === "light") document.documentElement.dataset.theme = "light";
}

createRoot(document.getElementById("root") as HTMLElement).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
