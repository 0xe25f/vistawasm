# Biomes

Every terrain VistaWASM generates or loads is divided into biomes. A biome
decides the ground textures, which tree species grow and how densely, and
the colour of grass. Biomes are computed on the CPU once per terrain (and
again when `setBiomes` is called) by `terrain/biomes.rs`.

For the exact field list, see
[`docs/options-reference.md`](options-reference.md#biomeoptions).

## The biomes

| Biome | Where it forms | Ground | Trees |
| --- | --- | --- | --- |
| `grassyMeadows` | Temperate, fairly dry lowland | Lush grass | Scattered oaks and shrubs |
| `outerThicket` | Temperate, moderate moisture | Grass and leaf litter | Shrubs, young oaks or conifers |
| `outerForest` | Temperate, moist | Forest floor | Oak and pine (pine and spruce when cold) |
| `innerForest` | Temperate, wet | Forest floor | Dense oak, pine, spruce (spruce when cold) |
| `mountainFoothills` | 40–62% of the way from sea to peak | Grass, rock | Pine, spruce |
| `mountainProper` | High or steep ground | Rock, scree, snow above the snow line | Sparse spruce |
| `outerVolcanic` | Around volcanic peaks | Basalt and ash | Very sparse pine |
| `calderaVolcanic` | Volcanic summits | Basalt with glowing lava cracks | None |
| `savannahExpanse` | Hot and dry | Straw-coloured grass | Sparse acacia |
| `coastalBeach` | Flat ground just above sea level | Sand | Palms when warm, pines when cool |
| `coastalRocky` | Steep ground near the sea | Rock | Sparse pine, shrubs, palms |
| `outerJungle` | Hot and moist | Forest floor | Jungle trees, palms, shrubs |
| `innerJungle` | Hot and very wet | Forest floor | Dense jungle trees |
| `swampWetlands` | Flat, wet, warm lowland | Mud | Cypress with hanging moss |
| `ocean` | Below sea level | Sand in the shallows, silt deeper | None |

## How classification works

1. **Climate.** Two seeded, domain-warped noise fields give each point a
    temperature and a moisture value. They vary over `climateScaleMetres`
    (7 km by default), so climate zones are large and coherent.
    `temperatureBias` and `moistureBias` shift the whole world.
2. **Altitude and terrain.** Temperature falls with height (a lapse
    rate), lowlands are slightly wetter, and slope matters: steep coasts
    become rocky, and steep high ground becomes `mountainProper`.
3. **Volcanoes.** The highest, well-separated peaks can become volcanoes;
    `volcanism` sets how many (up to three) and how large. Their summits
    become calderas with glowing lava in the basalt cracks.
4. **Decision.** Each sample takes the first matching rule, from ocean and
    volcanic regions through coasts and mountains to the climate biomes.
    A little fine noise keeps borders organic.

Surface materials are computed from the same continuous fields rather
than from the discrete biome, so textures blend smoothly across borders:
grass yellows gradually into savannah, sand fades up the beach, snow
settles on flatter ground above the snow line (lower in cold climates), and
river beds turn to sand and mud.

Classification is deterministic: the same terrain and options always give
the same biome map. Climate noise is evaluated on a coarse grid and
interpolated, so the pass is fast even on 4096 × 4096 terrain.

## Using biomes in your application

```ts
engine.setBiomes({
  temperatureBias: 0.6,
  moistureBias: 0.4,
  volcanism: 0.2
});

const biome = engine.biomeAt(player.x, player.z);

if (biome === "swampWetlands") {
  playAmbience("frogs");
}
```

- `setBiomes` re-bakes ground materials, trees, and grass. It is quicker
  than regenerating terrain but not free, so apply it when a slider is
  released rather than on every movement.
- The `"biomes"` debug view (`engine.setDebugView("biomes")`) paints the
  biome map over the terrain.
- `enabled: false` turns off climate: the world stays temperate, with
  meadows, thickets, forests, coasts, and mountains only.
