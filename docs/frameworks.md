# Using VistaWASM With React, Vue, and Svelte

VistaWASM owns a `<canvas>` you give it, so it fits any framework the same
way: create the engine when the component mounts, and stop and dispose it
when the component unmounts. These components are complete; copy one and
adjust the terrain options.

Each framework also has a runnable example under `examples/` (see
[`CONTRIBUTING.md`](../CONTRIBUTING.md#running-the-examples)).

Each component below creates the engine at the canvas's real size, keeps
it in step with the canvas, generates a terrain, points the camera at it,
and cleans up on unmount, including when the component unmounts before
loading finishes. Give the canvas a size in CSS:

```css
.vista-canvas {
  display: block;
  width: 100%;
  height: 100%;
}
```

## React

```tsx
import { useEffect, useRef } from "react";
import { createVistaEngine } from "@vista-wasm/vista-wasm";

export function VistaPanel() {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);

  useEffect(() => {
    const canvas = canvasRef.current;

    if (!canvas) {
      return;
    }

    let unmounted = false;
    let cleanup = () => {};

    async function run(target: HTMLCanvasElement) {
      const engine = await createVistaEngine(target, {
        render: {
          width: target.clientWidth,
          height: target.clientHeight,
          devicePixelRatio: window.devicePixelRatio
        }
      });

      if (unmounted) {
        engine.dispose();
        return;
      }

      const observer = new ResizeObserver(() => {
        engine.resize(target.clientWidth, target.clientHeight, window.devicePixelRatio);
      });
      observer.observe(target);
      cleanup = () => {
        observer.disconnect();
        engine.dispose();
      };

      const terrain = await engine.generateFractal({
        seed: 9876,
        size: 1024,
        horizontalScaleMetres: 12,
        verticalScale: 1.1,
        noise: { kind: "ridged", octaves: 7, gain: 0.5, lacunarity: 2 }
      });

      if (unmounted) {
        return;
      }

      const peak = terrain.metadata.maxHeightMetres;
      engine.setCamera({
        position: [0, peak + 400, 3000],
        target: [0, peak * 0.4, 0],
        fieldOfViewDegrees: 55
      });
      engine.start();
    }

    run(canvas).catch((error: unknown) => {
      if (!unmounted) {
        console.error(error);
      }
    });

    return () => {
      unmounted = true;
      cleanup();
    };
  }, []);

  return <canvas ref={canvasRef} className="vista-canvas" />;
}
```

## Vue

```vue
<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref } from "vue";
import { createVistaEngine } from "@vista-wasm/vista-wasm";

const canvas = ref<HTMLCanvasElement | null>(null);
let unmounted = false;
let cleanup = () => {};

async function run(target: HTMLCanvasElement) {
  const engine = await createVistaEngine(target, {
    render: {
      width: target.clientWidth,
      height: target.clientHeight,
      devicePixelRatio: window.devicePixelRatio
    }
  });

  if (unmounted) {
    engine.dispose();
    return;
  }

  const observer = new ResizeObserver(() => {
    engine.resize(target.clientWidth, target.clientHeight, window.devicePixelRatio);
  });
  observer.observe(target);
  cleanup = () => {
    observer.disconnect();
    engine.dispose();
  };

  const terrain = await engine.generateFractal({
    seed: 2222,
    size: 1024,
    horizontalScaleMetres: 10,
    verticalScale: 1,
    noise: { kind: "ridged", octaves: 7, gain: 0.5, lacunarity: 2 }
  });

  if (unmounted) {
    return;
  }

  const peak = terrain.metadata.maxHeightMetres;
  engine.setCamera({
    position: [0, peak + 400, 2500],
    target: [0, peak * 0.4, 0],
    fieldOfViewDegrees: 55
  });
  engine.start();
}

onMounted(() => {
  if (canvas.value) {
    run(canvas.value).catch((error: unknown) => {
      if (!unmounted) {
        console.error(error);
      }
    });
  }
});

onBeforeUnmount(() => {
  unmounted = true;
  cleanup();
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
  import { createVistaEngine } from "@vista-wasm/vista-wasm";

  let canvas: HTMLCanvasElement;
  let unmounted = false;
  let cleanup = () => {};

  async function run(target: HTMLCanvasElement) {
    const engine = await createVistaEngine(target, {
      render: {
        width: target.clientWidth,
        height: target.clientHeight,
        devicePixelRatio: window.devicePixelRatio
      }
    });

    if (unmounted) {
      engine.dispose();
      return;
    }

    const observer = new ResizeObserver(() => {
      engine.resize(target.clientWidth, target.clientHeight, window.devicePixelRatio);
    });
    observer.observe(target);
    cleanup = () => {
      observer.disconnect();
      engine.dispose();
    };

    const terrain = await engine.generateFractal({
      seed: 3333,
      size: 1024,
      horizontalScaleMetres: 10,
      verticalScale: 1,
      noise: { kind: "simplex", octaves: 7, gain: 0.5, lacunarity: 2 }
    });

    if (unmounted) {
      return;
    }

    const peak = terrain.metadata.maxHeightMetres;
    engine.setCamera({
      position: [0, peak + 400, 2500],
      target: [0, peak * 0.4, 0],
      fieldOfViewDegrees: 55
    });
    engine.start();
  }

  onMount(() => {
    run(canvas).catch((error: unknown) => {
      if (!unmounted) {
        console.error(error);
      }
    });
  });

  onDestroy(() => {
    unmounted = true;
    cleanup();
  });
</script>

<canvas bind:this={canvas} class="vista-canvas"></canvas>
```

## The pattern in any framework

1. Wait for the canvas element to exist (after mount).
2. `await createVistaEngine(canvas, { render: { width, height, devicePixelRatio } })`.
3. Generate or load terrain, set the camera, and call `engine.start()`.
4. Keep the render size in step with the canvas (see
    [`docs/getting-started.md`](getting-started.md#6-handle-resizing-and-disposal)).
5. On unmount, call `engine.dispose()`, which also stops the render loop.
    It is safe to call more than once, and while terrain is still
    generating: the engine finishes the in-flight call, then releases its
    GPU resources.
