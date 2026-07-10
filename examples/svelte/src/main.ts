import { mount } from "svelte";
import VistaPanel from "./VistaPanel.svelte";
import "./style.css";

const target = document.querySelector<HTMLDivElement>("#app");

if (!target) {
  throw new Error("Missing #app element.");
}

const app = mount(VistaPanel, { target });

export default app;
