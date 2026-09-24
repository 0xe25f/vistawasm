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
| `mountainProper` | High or steep ground below the snow | Rock and scree | Sparse spruce |
| `alpineTransition` | The band just below the snow line | Scree, thin turf, patchy snow in hollows | Dwarf shrubs, a few stunted spruce |
| `lowerSnowyPeaks` | From the snow line to halfway up to the highest peak | Snowfields broken by rock on steep ground | None |
| `upperSnowyPeaks` | The upper half of the ground above the snow line | Permanent snow and ice; rock only on near-vertical faces | None |
| `outerVolcanic` | Around volcanic peaks | Basalt and ash | Very sparse pine |
| `calderaVolcanic` | Volcanic summits | Basalt with glowing lava cracks | None |
| `savannahExpanse` | Hot and dry | Straw-coloured grass | Sparse acacia |
| `coastalBeach` | Flat ground just above sea level | Sand | Palms when warm, pines when cool |
| `coastalRocky` | Steep ground near the sea | Rock | Sparse pine, shrubs, palms |
| `outerJungle` | Hot and moist | Forest floor | Jungle trees, palms, shrubs |
| `innerJungle` | Hot and very wet | Forest floor | Dense jungle trees |
| `swampWetlands` | Flat, wet, warm lowland | Mud | Cypress with hanging moss |
| `ocean` | Below sea level | Sand in the shallows, silt deeper | None |
| `iceArctic` | Below freezing all year | Glacier ice and snow; tundra moss, lichen, and stones where the ice thins | None on glaciers; sparse dwarf shrubs on tundra |

## How classification works

1. **Climate.** Two seeded, domain-warped noise fields give each point a
    temperature and a moisture value. They vary over `climateScaleMetres`
    (7 km by default), so climate zones are large and coherent.
    `temperatureBias` and `moistureBias` shift the whole world.
2. **Altitude and terrain.** Temperature falls with height (a lapse
    rate), lowlands are slightly wetter, and slope matters: steep coasts
    become rocky, and steep high ground becomes `mountainProper`.
    Above the trees, ground rises through `alpineTransition` (15 % of the
    relief, 60 to 400 m deep, below the snow line) into `lowerSnowyPeaks`
    and then `upperSnowyPeaks`, which starts halfway from the snow line to
    the highest peak. The snow line drops in cold climates, and the bands
    drop with it.
3. **Volcanoes.** The highest, well-separated peaks can become volcanoes;
    `volcanism` sets how many (up to three) and how large. Their summits
    become calderas with glowing lava in the basalt cracks.
4. **Ice.** Where it is cold enough, ground becomes `iceArctic`: glacier
    or tundra (see below).
5. **Decision.** Each sample takes the first matching rule, from ocean and
    volcanic regions through ice, coasts, and mountains to the climate
    biomes. A little fine noise keeps borders organic.

Surface materials are computed from the same continuous fields rather
than from the discrete biome, so textures blend smoothly across borders:
grass yellows gradually into savannah, sand fades up the beach, snow
settles on flatter ground above the snow line (lower in cold climates), and
river beds turn to sand and mud.

Classification is deterministic: the same terrain and options always give
the same biome map. Climate noise is evaluated on a coarse grid and
interpolated, so the pass is fast even on 4096 × 4096 terrain.

## Climate temperature

Every sample has a mean annual temperature in °C. Read it with
`engine.temperatureAt(x, z)`, which returns `null` off the terrain.

Set `meanTemperatureCelsius` (from -30 to 35) to choose the climate. It
is the mean at sea level. Climate noise makes some regions warmer and
others colder, by up to about 4 °C across most of a map (never more
than 6 °C), and every sample then cools by 6.5 °C per 1000 m of
altitude, as real air does. A
3000 m mountain on a 15 °C map is about -4.5 °C at the top.
`temperatureBias` still shifts the whole map, by up to ±26 °C. The same
temperature then drives every biome: warm, moist lowlands become jungle
and cold ones freeze.

Leave `meanTemperatureCelsius` unset to keep the classic climate. Biomes
come from `temperatureBias` as before, and no ice forms: the highest
ground carries the snowy peak biomes instead. `temperatureAt()` then
reports 15 °C at sea level, falling to a little below freezing on the
highest summit (less on maps with only a few hundred metres of relief),
and the weather follows it: rain turns to sleet and snow on high
summits.

```ts
// An arctic island.
engine.setBiomes({ meanTemperatureCelsius: -12 });

const celsius = engine.temperatureAt(player.x, player.z);

if (celsius !== null && celsius < 0) {
  playAmbience("wind");
}
```

## Ice and tundra (`iceArctic`)

- **Glacier.** Ice forms on slopes under 35 degrees where the mean
    temperature is below the glacier threshold, and on slopes up to 50
    degrees where it is 4 °C colder still. Glaciers need snowfall as well
    as cold, so the threshold follows the moisture: -2 °C in the wettest
    climates, about -6 °C in average ones, and down to about -16 °C in
    the driest. Cold, dry ground stays tundra instead: polar desert.
- **Tundra.** Moss, lichen, dwarf shrubs, and frost-heaved stones cover
    ground from the glacier threshold up to 3 °C, and cold slopes too
    steep for ice.
- **Cliffs.** Cold slopes steeper than 50 degrees stay bare rock with
    snow: `mountainProper`, or the snowy peak biomes above the snow line.

Glacier ice is blended with snow: fresh snow lies deeper where it is
colder and flatter, and patches are scoured to bare blue ice. The ice
shows a subsurface blue where the sun does not reach, and crevasses open
across the flow where the ice speeds up over slopes of 8 to 30 degrees.

Ice fills valleys: inside a glacier, the ground is raised towards a
smoothed surface by at most 40 m. This happens before rivers are carved,
and it is undone exactly when the climate warms, biomes are switched
off, or the glacier shrinks, so `exportHeightmap()` always returns the
original terrain plus the current glaciers and rivers.

Snow that never melts lies on glaciers and, in patches, on the colder
tundra. It whitens the tops of trees and buries grass, just as snow from
the weather does. Nothing grows on glaciers. Tundra grows dwarf shrubs,
knee to waist high, at a tenth of forest density, and short ochre-green
tufts of grass.

Cold seas freeze too. See [`docs/water.md`](water.md#sea-ice).

| Climate | What you see |
| --- | --- |
| `meanTemperatureCelsius: -18` | Ice sheets and snow-covered mountains, pack ice at sea, fast ice on the coast. |
| `meanTemperatureCelsius: -3` | A mosaic of glaciers and tundra; loose floes on the sea. |
| Unset (about 15 °C) | No ice; the snowy peak biomes on the highest ground. |
| `meanTemperatureCelsius: 15` | Tundra and small glaciers only near the tops of mountains over about 2500 m. |
| `meanTemperatureCelsius: 30` | No ice anywhere. |

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

- `setBiomes` re-bakes ground materials, trees, and grass, and re-shapes
  glaciers and rivers. It is quicker than regenerating terrain but not
  free, so apply it when a slider is released rather than on every
  movement.
- The `"biomes"` debug view (`engine.setDebugView("biomes")`) paints the
  biome map over the terrain.
- `enabled: false` turns off climate: the world stays temperate, with
  meadows, thickets, forests, coasts, and mountains only, and no ice.
