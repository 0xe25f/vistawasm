// Browser smoke test for continuous integration: render a few scenes with
// software WebGPU in headless Chromium and fail on any page or console
// error, any console warning (a WGSL error is only a warning and a black
// frame), any error the page logged, or a frame that is all but black.
// It is the only check that runs the engine's WebGPU paths.
//
//   python3 -m http.server 8124        (from the repository root)
//   node scripts/visual-check/smoke.mjs [out-directory]
//
// Each scene runs in its own browser through `capture.mjs`, one at a time.
// It checks for errors, not pixels: software rendering differs from a GPU.
// A last scene forces a GPU validation error and checks it arrives as a
// `gpuError` event.
import { spawnSync } from "node:child_process";
import { mkdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { decodePng } from "../../dist/png-decode.js";

const out = process.argv[2] ?? "smoke";
const capture = fileURLToPath(new URL("./capture.mjs", import.meta.url));
const size = [480, 300];
const scenes = [
  ["default", { size }],
  [
    "raw-heightmap",
    {
      size,
      heightmap: {
        url: "fixed-512.f32.gz",
        options: {
          width: 512,
          height: 512,
          sampleFormat: "float32",
          byteOrder: "little-endian",
          metresPerSample: 12,
          heightScaleMetres: 1,
          seaLevelMetres: 0
        }
      },
      camera: { position: [0, 600, 2600], target: [0, 200, 0] }
    }
  ],
  [
    "river",
    {
      size,
      terrain: { landform: "alpine", shape: { island: 0 } },
      shots: [{ position: [510, 0, 1820], height: 40, target: [510, 0, 1902], targetHeight: 0 }]
    }
  ],
  // The render loop, with nothing listening for stats, so it draws
  // without reading them: it must draw frames without errors.
  [
    "render-loop",
    {
      size,
      terrain: { size: 64 },
      shots: [
        {
          eval: [
            "(async () => {",
            "  const before = window.engine.renderOnce().frameIndex;",
            // Software rendering takes many seconds for a first frame, and
            // the engine skips frames while the GPU is busy.
            "  await window.gpuDevice.queue.onSubmittedWorkDone();",
            "  window.engine.start();",
            "  await new Promise((resolve) => setTimeout(resolve, 20000));",
            "  window.engine.stop();",
            "  await window.gpuDevice.queue.onSubmittedWorkDone();",
            "  const drawn = window.engine.renderOnce().frameIndex - before - 1;",
            "  window.log.push(drawn > 0 ? `render loop drew ${drawn} frames` : 'error the render loop drew no frames');",
            "})()"
          ].join("\n")
        }
      ]
    }
  ],
  // A validation error the engine did not cause, on its device: the
  // uncaptured-error handler must report it as a `gpuError` event. The
  // browser also prints it as a console warning, which this scene allows.
  [
    "gpu-error",
    {
      size,
      terrain: { size: 64 },
      shots: [{ eval: "window.gpuDevice.createBuffer({ size: 4, usage: 0 })" }]
    },
    { expectGpuError: true }
  ]
];

/** Mean brightness, 0 to 255, and the share of pixels above near-black. */
function brightness(image) {
  let sum = 0;
  let lit = 0;
  const pixels = image.width * image.height;

  for (let at = 0; at < pixels * 4; at += 4) {
    const value = (image.data[at] + image.data[at + 1] + image.data[at + 2]) / 3;
    sum += value;
    lit += value > 8 ? 1 : 0;
  }

  return { mean: sum / pixels, lit: lit / pixels };
}

mkdirSync(out, { recursive: true });
const failures = [];

for (const [name, config, { expectGpuError = false } = {}] of scenes) {
  const file = join(out, `${name}.png`);
  const started = Date.now();
  const run = spawnSync(process.execPath, [capture, file, JSON.stringify(config), "3"], {
    encoding: "utf8",
    env: process.env,
    maxBuffer: 16 * 1024 * 1024
  });
  const output = `${run.stdout ?? ""}${run.stderr ?? ""}`;
  console.log(`--- ${name} (${Math.round((Date.now() - started) / 1000)} s)\n${output.trim()}`);
  const problems = [];

  if (run.status !== 0) {
    problems.push(`capture.mjs exited with ${run.status ?? run.signal ?? run.error?.message}`);
  }

  if (/^console warning:/m.test(output) && !expectGpuError) {
    problems.push("the console printed a warning (a WGSL error shows as one)");
  }

  if (/"gpuError /.test(output) !== expectGpuError) {
    problems.push(expectGpuError ? "the forced GPU error did not arrive as a gpuError event" : "the engine reported a GPU error");
  }

  if (/"deviceLost /.test(output)) {
    problems.push("the engine lost its device");
  }

  if (/"error /.test(output)) {
    problems.push("the page logged an engine error");
  }

  if (run.status === 0) {
    const image = await decodePng(new Uint8Array(readFileSync(file.replace(/\.png$/, config.shots?.length > 1 ? "-0.png" : ".png"))));
    const { mean, lit } = brightness(image);
    console.log(`${name}: mean brightness ${mean.toFixed(1)}, ${(lit * 100).toFixed(1)} % of pixels lit`);

    if (mean < 10 || lit < 0.5) {
      problems.push(`the frame is black or nearly so (mean ${mean.toFixed(1)}, ${(lit * 100).toFixed(1)} % lit)`);
    }
  }

  failures.push(...problems.map((problem) => `${name}: ${problem}`));
}

if (failures.length) {
  console.log(`\nThe browser smoke test failed:\n${failures.map((failure) => `- ${failure}`).join("\n")}`);
  process.exit(1);
}

console.log(`\nAll ${scenes.length} scenes rendered without errors.`);
