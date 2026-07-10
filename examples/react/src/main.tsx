import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { VistaPanel } from "./VistaPanel";
import "./style.css";

const container = document.querySelector<HTMLDivElement>("#root");

if (!container) {
  throw new Error("Missing #root element.");
}

createRoot(container).render(
  <StrictMode>
    <VistaPanel />
  </StrictMode>
);
