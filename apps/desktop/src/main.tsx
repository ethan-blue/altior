import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./app/App";
import { createDefaultTransport } from "./ipc/tauriTransport";
import "./styles/reset.css";
import "./styles/tokens.css";

const rootElement = document.getElementById("root");
if (!rootElement) {
  throw new Error("index.html must provide a #root element");
}

// One transport decision for the whole app (review A02): Tauri WebView →
// real Core IPC; Vite dev server → explicit fixture entry; plain production
// browser → a transport that fails loudly instead of faking a connection.
const transport = createDefaultTransport();

createRoot(rootElement).render(
  <StrictMode>
    <App transport={transport} />
  </StrictMode>,
);
