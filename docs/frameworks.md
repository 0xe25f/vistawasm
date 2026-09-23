# Using VistaWASM With React, Vue, and Svelte

VistaWASM owns a `<canvas>` you give it, so it fits any framework the same
way: create the engine when the component mounts, and stop and dispose it
when the component unmounts. These components are complete; copy one and
adjust the terrain options.

Each framework also has a runnable example under `examples/` (see
[`CONTRIBUTING.md`](../CONTRIBUTING.md#running-the-examples)).

## React

```tsx
import { useEffect, useRef } from "react";
import { createVistaEngine, type VistaEngine } from "@vista-wasm/vista-wasm";

export function VistaPanel() {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const engineRef = useRef<VistaEngine | null>(null);

  useEffect(() => {
    let disposed = false;

    async function run() {
      const canvas = canvasRef.current;

      if (!canvas) {
        return;
      }

      const engine = await createVistaEngine(canvas);

      if (disposed) {
        engine.dispose();
        return;
      }

      engineRef.current = engine;

      await engine.generateFractal({
        seed: 9876,
        size: 2048,
        horizontalScaleMetres: 12,
        verticalScale: 1.1,
        noise: {
          kind: "simplex",
          octaves: 8,
          gain: 0.5,
          lacunarity: 2
        }
      });

      engine.start();
    }

    run().catch((error) => {
      console.error(error);
    });

    return () => {
      disposed = true;
      engineRef.current?.dispose();
      engineRef.current = null;
    };
  }, []);

  return <canvas ref={canvasRef} className="vista-canvas" />;
}
```

## Vue

```vue
<script setup lang="ts">
import { onMounted, onUnmounted, ref } from "vue";
import { createVistaEngine, type VistaEngine } from "@vista-wasm/vista-wasm";

const canvas = ref<HTMLCanvasElement | null>(null);
let engine: VistaEngine | null = null;

onMounted(async () => {
  if (!canvas.value) {
    return;
  }

  engine = await createVistaEngine(canvas.value);

  await engine.generateFractal({
    seed: 2222,
    size: 2048,
    horizontalScaleMetres: 10,
    verticalScale: 1,
    noise: {
      kind: "ridged",
      octaves: 7,
      gain: 0.5,
      lacunarity: 2
    }
  });

  engine.start();
});

onUnmounted(() => {
  engine?.dispose();
  engine = null;
});
</script>

<template>
  <canvas ref="canvas" class="vista-canvas" />
</template>
```

## Svelte

```svelte
<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import { createVistaEngine, type VistaEngine } from "@vista-wasm/vista-wasm";

  let canvas: HTMLCanvasElement;
  let engine: VistaEngine | null = null;

  onMount(async () => {
    engine = await createVistaEngine(canvas);

    await engine.generateFractal({
      seed: 3333,
      size: 2048,
      horizontalScaleMetres: 10,
      verticalScale: 1,
      noise: {
        kind: "simplex",
        octaves: 7,
        gain: 0.5,
        lacunarity: 2
      }
    });

    engine.start();
  });

  onDestroy(() => {
    engine?.dispose();
    engine = null;
  });
</script>

<canvas bind:this={canvas} class="vista-canvas" />
```

## The pattern in any framework

1. Wait for the canvas element to exist (after mount).
2. `await createVistaEngine(canvas, { render: { width, height, devicePixelRatio } })`.
3. Generate or load terrain, set the camera, and call `engine.start()`.
4. Keep the render size in step with the canvas (see
    [`docs/getting-started.md`](getting-started.md#6-handle-resizing-and-disposal)).
5. On unmount, call `engine.stop()` and `engine.dispose()`. `dispose()` is
    safe to call more than once, and is safe while terrain is still
    generating.
