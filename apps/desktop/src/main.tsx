import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./app/App";
import { InMemoryTransport } from "./ipc/inMemoryTransport";
import { TauriCoreTransport } from "./ipc/tauriTransport";
import "./styles/reset.css";
import "./styles/tokens.css";

const rootElement = document.getElementById("root");
if (!rootElement) {
  throw new Error("index.html must provide a #root element");
}

const win = typeof window !== "undefined" ? (window as any) : null;
const isTauri =
  win != null && (win.__TAURI_INTERNALS__ != null || win.__TAURI__ != null);

const isDev =
  (typeof process !== "undefined" && process.env?.NODE_ENV === "development") ??
  (typeof import.meta !== "undefined" && (import.meta as any).env?.DEV) ??
  false;

const transport = isTauri
  ? new TauriCoreTransport({ fallbackToMemoryInDev: false })
  : isDev
    ? new InMemoryTransport()
    : new TauriCoreTransport({ fallbackToMemoryInDev: false });

createRoot(rootElement).render(
  <StrictMode>
    <App transport={transport} />
  </StrictMode>,
);
