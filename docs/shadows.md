# Shadows

Hills, trees, and clouds all cast sun shadows. Each can be switched off or
tuned on its own.

```ts
engine.setShadows({
  terrain: { enabled: true, strength: 0.9, softness: 0.35 },
  trees: { enabled: true, distanceMetres: 260, resolution: 2048, strength: 0.8, softness: 0.5 },
  clouds: { enabled: true, strength: 0.78 }
});
```

Every field is optional. The same object can be passed as
`VistaEngineOptions.shadows`.

## Terrain shadows

Mountains shadow valleys, and ridges shadow the slopes behind them. A
compute pass marches across the heightmap towards the sun and records
the horizon for each point, with a soft penumbra (`softness`, 0 hard to 1
very soft).

The result is a texture of at most 1024 × 1024. It is baked only when the
sun moves by more than about a third of a degree, `softness` changes, or
the terrain changes, so it is free on almost every frame. Animating the
sun re-bakes a few times a second at most.

## Tree shadows

Trees shadow the ground, grass, water, and each other. Within
`distanceMetres` of the camera (10 to 4000), each tree is drawn into a sun
shadow map as a single sun-facing quad cut out with its own impostor, so
the shadow has the real crown silhouette for two triangles per tree. The
GPU cull pass picks the casters, so there is no CPU cost per tree.

| Setting | Effect |
| --- | --- |
| `resolution` | Shadow map size: `512`, `1024`, `2048` (default), or `4096`. |
| `distanceMetres` | Larger covers more ground but makes each shadow coarser. |
| `softness` | Filter width, 0 (sharp) to 1 (soft). |
| `strength` | Darkness, 0 to 1. |

The shadow area sits slightly ahead of the camera, where shadows are seen,
and snaps to whole shadow-map texels, so shadow edges do not shimmer as the
camera moves.

For a low-end GPU, try `resolution: 1024` and `distanceMetres: 160`, or
switch tree shadows off.

## Cloud shadows

Clouds cast soft moving shadows that line up with the clouds above. They
need both `shadows.clouds.enabled` and `CloudsOptions.castShadows`. Cloud
shadows cost one texture lookup per pixel.

## Weather

Overcast skies diffuse most direct sunlight, so shadows fade out as the
weather greys over. See [`docs/weather.md`](weather.md).
