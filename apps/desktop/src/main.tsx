import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./app/App";
import "./styles/reset.css";
import "./styles/tokens.css";

const rootElement = document.getElementById("root");
if (!rootElement) {
  throw new Error("index.html must provide a #root element");
}

createRoot(rootElement).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
