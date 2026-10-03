// The smallest useful VistaWASM app: generate a terrain and render it.
import { createVistaEngine } from "@vista-wasm/vista-wasm";

const canvas = document.querySelector("#view");
const engine = await createVistaEngine(canvas, { render: { width: 960, height: 540 } });

await engine.generateFractal({
  seed: 1,
  size: 512,
  horizontalScaleMetres: 12,
  verticalScale: 1,
  noise: { kind: "simplex", octaves: 5, gain: 0.5, lacunarity: 2 }
});
engine.setCamera({ position: [0, 600, 2400], target: [0, 150, 0], fieldOfViewDegrees: 55 });
engine.start();
