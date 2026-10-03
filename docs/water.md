# Water

VistaWASM draws four kinds of water, all shaded by `shaders/water.wgsl`:

- **Ocean** — a camera-following grid at `seaLevelMetres` that reaches the
  horizon, displaced by a simulated Gerstner swell.
- **Rivers** — fed by rain, snowmelt and springs, running in channels
  shaped into the terrain, with a current that runs downstream.
- **Lakes** — filling basins to their spill height and overflowing into
  the rivers below them.
- **Waterfalls** — where a river drops over a step, with mist and a
  churned plunge pool.

For the exact field list, see
[`docs/options-reference.md`](options-reference.md#wateroptions).

## Sea level

`WaterOptions.seaLevelMetres` positions the ocean. Set it relative to the
generated terrain's real height range from `TerrainMetadata`, not a
hardcoded constant:

```ts
const handle = await engine.generateFractal(options);

engine.setWater({
  enabled: true,
  seaLevelMetres: handle.metadata.minHeightMetres + 5,
  waveScale: 0.8,
  reflectivity: 0.35,
  shorelineSoftnessMetres: 6
});
```

[`FractalTerrainOptions.seaLevelMetres`](options-reference.md#fractalterrainoptions)
is a separate field that seeds `TerrainMetadata.seaLevelMetres`, which
biomes, rivers, and vegetation use. It is normal to set both to the same
value.

## Waves (`waves`)

The swell is a sum of eight Gerstner waves spread around
`directionDegrees`. Secondary waves are shorter and keep roughly the same
steepness as the dominant swell, which gives a natural, non-repeating sea.

- `amplitudeMetres` and `wavelengthMetres` set the dominant swell.
- `steepness` sharpens crests; high values produce choppy water and
  whitecaps where crests fold.
- `directionalSpread` goes from a clean, parallel swell (`0`) to a
  confused, storm-like sea (`1`).
- `speed` scales animation; `1` uses deep-water dispersion, so long waves
  travel faster than short ones.
- `enabled: false` keeps the surface flat and leaves only ripples.

Waves shoal as the water gets shallow: they shrink towards the shore and
break into rolling bands of surf foam. Each wave also fades out wherever
the grid is too coarse to represent it, so distant water never aliases.
The whole simulation is analytic and runs in the vertex and fragment
shaders; it costs nothing on the CPU.

```ts
engine.setWater({
  enabled: true,
  seaLevelMetres: 0,
  waveScale: 1.2,
  reflectivity: 0.4,
  shorelineSoftnessMetres: 6,
  waves: { amplitudeMetres: 3, wavelengthMetres: 90, steepness: 0.8, directionalSpread: 0.8 },
  currentSpeed: 1.2,
  foam: 1
});
```

## Sea state

While the weather drives the water (`WeatherOptions.effects.water`, on by
default), the sea follows the wind speed U, in m/s:

- **Wave height.** The significant wave height is `0.0246 x U^2` for a
    fully developed sea, or less where the fetch limits it: the open water
    upwind of the camera, up to 20 km, with the sea beyond the map counted
    as open. A sheltered bay stays calmer than the open coast. It scales
    the configured swell, so `waves.amplitudeMetres` stays a multiplier: it
    is the swell of a 5 m/s breeze over the longest fetch.
- **Whitecaps.** From 4 m/s, crests break over `3.84e-6 x U^3.41` of the
    sea, up to 30 %, as white foam on the crests.
- **Direction.** The waves turn towards the wind over about a minute.
- **Spray.** From 15 m/s, spray blown off the crests streaks the sea
    downwind.
- **Gusts.** Gusts roughen the water in dark patches as they sweep across
    it, with the same gust front that bends the trees and grass.

```ts
// A gale at the coast: steep seas, whitecaps and spray.
engine.setWeather({ enabled: true, state: "storm" });
// A calm evening: a low swell and no whitecaps.
engine.setWeather({ enabled: true, state: "fewClouds", windScale: 0.5 });
```

## Currents

`currentDirectionDegrees` and `currentSpeed` move small ripples and foam
across open water and lakes. Rivers carry their own current (below).

## Rivers and lakes (`rivers`)

Rivers come from the water draining the land. When a terrain is
installed, and whenever `rivers` or the water mask changes, the engine
routes water over the finished heights (after erosion, glaciers and any
painted water) on a grid at full terrain resolution up to 1024 samples
per side. The river build is reported as the `"rivers"` progress phase;
at 512 × 512 it takes about 100 to 350 ms in the browser.

### Sources

- **Rain.** Every sample adds runoff from its precipitation: 300 mm a
    year in the driest climates to 3000 mm in the wettest, from the
    climate's moisture. Runoff is accumulated downhill into a mean
    discharge in m³/s.
- **Snowmelt.** Snow fields and glaciers add `snowmelt` × snow × 0.6 m of
    water a year. Glacier snouts (where meltwater leaves the ice) and the
    lower edge of each snowy-peak field start streams of their own, even
    where too little water has gathered yet. Water runs on under glacier
    ice: nothing is drawn or carved on the ice itself.
- **Springs.** Where steep ground (over 20 degrees) meets gentle ground
    (under 8 degrees) in a hollow draining more than 0.05 km², a small
    spring (0.02 m³/s) starts a stream. Springs are placed by the terrain
    and at most one per 600 m. `springs: false` turns them off.
- **Inflow.** On a map with open edges (`edges: "open"`), a river can
    arrive from beyond the map. `inflow: "auto"` (the default) places one
    at the lowest valley mouth on the edge that lies at least an eighth
    of the map from the sea, with the discharge of a basin ten times the
    map's land area at its mean rain. `inflow: "none"` adds nothing, and
    a list places up to 8 inflows yourself; each snaps to the nearest
    land sample. `getInflows()` returns the inflows in use.
- A sample becomes a channel when its discharge reaches
    `minCatchmentKm2` × 0.03 m³/s per km², about the flow of that
    catchment in an average climate.
- **Rills.** On slopes of 10 % or more, a stream's head stays undrawn
    while it runs alongside a larger stream: within two samples, on a
    course within 20° of it over the next four samples, without meeting
    it. Only a sliver of ground divides the two, and on a real hillside
    the smaller is a rill, dry for much of the year. It is drawn from
    where it turns away or another channel joins it. Its water still
    flows down the same way.

With biomes switched off, rain and snow follow the default climate.

```ts
const water = {
  enabled: true,
  seaLevelMetres: 0,
  waveScale: 0.8,
  reflectivity: 0.35,
  shorelineSoftnessMetres: 6
};

// A 40 m³/s river entering 2 km west of the centre of the map.
engine.setWater({
  ...water,
  rivers: { inflow: [{ position: [-2000, 0], dischargeCubicMetresPerSecond: 40 }] }
});
const [inflow] = engine.getInflows();
console.log(inflow.position, inflow.dischargeCubicMetresPerSecond);

// Back to the automatic inflow on open edges.
engine.setWater({ ...water, rivers: { inflow: "auto" } });
```

### Lakes

Every basin is filled to its spill height. A basin with at least 24
samples, or deeper than 1.5 m, becomes a lake whose surface is its spill
height. Its outlet river starts at the lowest point of its rim, so rivers
never end in a dead end. Where evaporation (warmer and drier climates
evaporate more) takes all the inflow, the lake keeps its water with no
outlet: an endorheic lake, as in a desert basin. Every channel ends at the
sea, at a lake, or at the map edge.

### Channel form

The channels follow the valleys erosion carved; the river build routes
water down them, draws each river along a smooth curve and shapes its
bed and banks:

- **Routing.** Water leaves each sample in the true direction of
    steepest descent over its eight triangular facets, and keeps track
    of how far its path has strayed from that line, picking whichever of
    the two neighbours brings it back (least transversal deviation,
    Orlandini and others, 2003). Rivers on a slope at any angle run
    straight along it, not in 45° zigzags. Flat ground (filled pits and
    lake floors) drains towards its way out by converging paths, never
    in parallel straight lines.
- **Smooth centrelines.** Each river follows the lowest ground across
    its valley, then is smoothed without shrinking (Taubin), bends
    tighter than 1.5 widths (or a sample) are eased, and it is resampled
    along a spline every half width, or half a sample for narrow
    streams. It never climbs out of its valley, and heads, mouths, the
    map edge and both ends of every waterfall stay where they are.
- **Width and depth** grow with discharge: width 2.7 √Q × `widthScale`
    and depth 0.35 Q⁰·⁴ metres, both from 0.6 to 400 m. A small island
    stream carries a few litres a second and is under a metre wide; a
    large lowland river is tens of metres wide. Steep bedrock reaches
    (over 2 %) are up to 30 % narrower.
- **Junctions.** Tributaries meet their main stem at an acute angle
    pointing downstream, 25° to 85°, wider where the tributary is much
    steeper than the main stem, along a smooth curve; streams are ordered
    by Strahler order, and the largest are drawn first.
- **The bed never rises downstream**, and is smoothed along the river
    except across waterfall steps.
- **Cross-section.** In steep valleys (over 6 %) the channel is a narrow
    V; on gentle ground (under 2 %) it is flat-bottomed, with a floodplain
    four widths wide on each side levelled towards the bank; in between
    it blends. On a bend the deepest water moves towards the outer bank,
    a steeper cut bank, and the inner bank is a gentle point bar; steep
    outer banks show bare earth and rock.
- **Valley floors.** A river at least 20 m wide on ground flatter than
    1 % lowers the ground beside it to half a metre above the bank for
    three widths, blending back over six more, so a big river flows
    through a floor, not a canal.
- **Meanders.** Rivers wide enough for their bends to show on the
    heightmap (11 widths at least 3 samples) migrate by bank erosion
    (Howard and Knutson, 1984): bends grow, skew downstream and travel,
    faster where the valley floor is wide, the ground gentle and the
    banks soft, and valley walls deflect them. `meanders` sets how
    strongly they migrate and `meanderMaturity` how long. A sinuous river
    is gentler than its valley, and its tributaries join it where it now
    runs.
- **Oxbows and scroll bars.** Loops whose necks close are cut off and
    leave oxbow lakes; older oxbows are shorter and shallower. The belt
    the river swept is levelled into a floodplain, and faint curved
    swales mark where its bends used to lie.
- **Braids.** Steep, wide rivers with room to spread (slope over
    `0.02 Q^-0.44`, at least a sample wide, a floor at least three
    widths) split into two to four threads that wander, merge and split
    across a gravel belt up to three widths wide. `braiding` sets how
    readily. Confined and rock-walled reaches stay single.
- **Estuaries.** A river meeting the sea on gentle ground widens like a
    funnel over its last stretch, up to three times its width at the
    sea; on a steep coast, at most one and a half times.
- **Deltas.** A river wider than 8 m meeting the sea on ground flatter
    than 0.5 % splits at its delta head into distributaries that divide
    again at mouth bars, unequally, up to eight arms, each following
    the seaward slope with a little wander, over a low fan of silt 0.5 m
    above the sea.
- **Waterfalls** form where the bed drops more than 3 m (or 1.5 widths)
    within two samples far more steeply than the reach around it, or runs
    steeper than 35 degrees. A long steep run becomes a staircase of steps
    of at most two samples each, with pools between them, as steep
    mountain streams are. Steps under 3 m become rapids. Falls close
    together form one cascade. Each fall has a plunge pool 0.3 × its
    height + its width across and 0.15 × its height deep, scaled by its
    discharge (from 0.15 for a trickle to the full size at 4 m³/s), at
    least 0.3 m deep. A trickle under 0.05 m³/s and under 1 m wide falls
    as whitewater down the step, with no sheet, mist or pool. The bowl is cut into the ground, never built up, and
    its water stands only as high as where it spills, over its lowest
    rim point or into its outlet. Around that, and all over on a slope
    where the bowl holds nothing, it is a thin film of churned water over
    the ground. `waterfalls: false` keeps the channel draped over the
    step instead.

Every change is recorded, so turning rivers off (or clearing the water
mask) restores the original heights exactly, and `exportHeightmap()`
returns the shaped terrain while rivers are on.

### Flowing water

- **On the ground.** A drawn stream's water lies at most its depth and
    0.5 m above the carved ground under its line, and never rises
    downstream, so a steep stream drawn between the samples it was routed
    through runs down its slope instead of hanging above it.
- **Ribbons** are 1.3 channel widths wide, and fade out where the water
    is shallower than 25 cm, so banks meet the water with no edge. Each
    edge wanders in and out by up to a fifth of the ribbon every few
    widths, so a stream narrower than a sample is not one even width. On
    the inside of a tight bend the edge stops short of the bend's
    centre, so it never folds over itself.
- **Speed** comes from Manning's equation, `v = R^(2/3) S^(1/2) / n`
    with R the depth, from 0.2 to 6 m/s, times `currentSpeed`. Lowland
    channels have n = 0.035. Steep ones lose their energy over boulders,
    steps and pools, so n grows with slope as `0.39 S^0.38 R^-0.16`
    (Jarrett's relation for mountain streams): a 4 % river 2 m deep runs
    at about 3 m/s, and a cascade at under 2 m/s. The speed moves the
    ripples downstream at the water's speed.
- **The current across the river** is fastest in a core that hugs the
    outer bank of a bend, and slower by the banks; the average stays the
    river's speed.
- **Ripples** are glassy under 0.5 m/s and small and choppy above 2 m/s,
    in two sizes, and stretched along the current into flow lines where
    it runs fast. They are carried along the flow in two copies
    cross-faded in turn;
    each patch of water fades at its own moment, so the river never
    blinks, and the fade keeps the ripples' contrast.
- **Streaks and foam** are laid out along the channel, so they curve
    with every bend.
- **Rapids** on slopes over 2 % (and below small steps) carry standing
    waves fixed in place, square to the channel and bent into chevrons
    pointing downstream. Crests finer than a few pixels fade out rather
    than flicker.
- **Whitewater** covers a share of the surface that grows with slope: a
    few breaking crests on a 2 % riffle, about half of a 12 % cascade,
    and never more than three quarters. It breaks in clumps a few widths
    long on the waves' crests, with dark, fast water between, so even a
    steep torrent reads as water, not as a pale, dry gravel bed. Where
    the clumps are finer than the pixels the water is evenly paler by
    that share.
- **Step-pools.** On slopes of 6 to 30 %, a stream at least 0.75 m wide
    drops from pool to pool instead of sliding down an even ramp. Its
    steps form behind its largest stones, `0.31 S^-1.19` metres apart
    (Judd's relation, kept within 1.5 to 3 widths), wherever a step
    drops at least 10 cm. The water surface itself steps: each pool is
    level at its lip and drops over a face 0.2 m long, white water
    plunges below each lip, and a dark, glassy pool lies beneath it. A
    pool never stands above the smooth surface, nor more than its depth
    and 0.5 m above the ground, nor sinks below 40 % of its depth over it.
    Steps stop two widths short of joins, lakes and falls. Steeper
    streams tumble down as a cascade of white water.
- **Eddies** (`eddies`, on by default): on the inner bank below a sharp
    bend, beside a tributary's join and below a fall, the water by the
    bank slows and turns back, with foam along the line where it meets
    the main flow. The ripples swirl there, and turning vortices with
    glassy, dimpled cores drift downstream; fast water shows small boils
    everywhere, and stones leave eddies in their wake. They fade out
    beyond 250 to 400 m. `eddies: 0` turns them off without rebuilding
    rivers.
- **Bends** run faster on the outer bank, and fast water breaks there.
- **Clarity.** Shallow water shows the bed; fast rivers carry silt that
    clouds and browns them.
- **Colour from the catchment.** A river draining mostly bog, or cold,
    wet ground, runs tea-brown and darker; one below a glacier runs milky
    turquoise and cloudy; others run clear.
- **Seeing into the water** (`refraction`, on by default). Under water
    less than 2 m deep the bed bends with the ripples (refractive index
    1.33), read from the scene behind the water; plants in front of it
    and the screen's edges leave the bed as drawn. Sunlight the ripples
    focus plays over the bed as caustics, drifting with the flow, gone
    in shadow and under cloud and beyond 30 m. Both fade out by 120 m.
    `refraction: 0` turns them off without rebuilding rivers.
- **Mouths.** An estuary's banks open at no more than 12 degrees and
    curve out smoothly. Two sand bars just above the sea flank a wide
    estuary's mouth. The river's water fades into the sea as the sea
    floor falls below its bed, and out at sea the river's colour (silt,
    peat or glacial flour) spreads in a plume to four times the mouth's
    width, fading out over 5 to 20 widths by discharge. The eight mouths
    that matter most from the camera have plumes.
- **Snowmelt fullness.** Where the camera is warmer, rivers run faster
    and foamier and rise up to a fifth of their depth in their channels;
    where it is cold they run slow and low.

### Small streams

Streams too narrow for their meanders to migrate on the heightmap carry
detail the grid cannot:

- **Loops.** On slopes under 1 %, the ribbon meanders about its smooth
    centreline at the stream's
    own wavelength (about 11 widths, drifting by up to 30 %, with the
    amplitude between 60 and 100 %, so no two loops match), inside a
    corridor of ±0.45 samples around the carved path so the water stays
    in its trench. The loops
    pass through the stream's ends and every join with another stream.
    The hydrology, carving and sounds keep the carved path.
- **Wander.** Steeper streams do not meander, but none runs straight:
    each follows the hollows of its slope and is turned by boulders,
    roots and banks. It wanders in long bends about 3.5 samples (at
    least 30 widths) long, by up to 0.6 of the same corridor, and short
    ones about 5 widths long, by up to half a width, pinned at its ends
    and joins like the loops.
- **Banks.** Each bank is a mesh with a profile: a wet margin of mud,
    sand or gravel by the speed (under 0.4 m/s, to 1 m/s, above), a face,
    and a turf lip on the ground behind. On the outside of bends and
    where the valley side rises steeply, banks are cut: a face 0.2 to
    1.5 m high, steep, with roots and a lip overhanging it. Inner banks
    shelve gently into a wide margin. The shape changes smoothly over a
    few widths, a tributary's banks take the main stem's at its join,
    and no face stands higher than the ground behind it. The turf takes
    the terrain's own grass colour and fades into it. Banks fade out
    between 60 and 150 m, and on snow and ice.
- **A budget for strips.** A map has at most 300,000 strip vertices.
    Where a stream runs straight, rows merge: a row is dropped where the
    heading turns by less than 4 degrees, the merged segment is at most
    a sample long, and the ground under it stays within 5 cm of the
    strip. If a map is still over budget, each stream keeps every n-th
    row, with n a power of two in inverse proportion to how visible it
    is, so the least visible streams (the narrowest, slowest and
    furthest from the map centre) lose rows first and most, until it
    fits.
- **No dashes.** Ribbons and strips widen to at least 0.75 pixel each
    side and fade by how much of that the water covers, so a far brook is
    a faint continuous line.
- **No white wires.** A channel hides the low sky: a ray reflected at a
    grazing angle meets the far bank before it clears the bank top, about
    a channel depth above the water. So a stream mirrors the sky only as
    far as `width x tan(elevation) / depth` allows, and its dark banks
    otherwise, and its sun glint needs the sun clear of the banks too.
    Glint and foam also shrink with the stream's width in pixels. A small
    stream seen from afar is a darker line of water, never a bright
    white one.
- **Reeds** grow along the true banks of slow brooks, where the grid is
    too coarse to find them.

### Beds, stones and rapids

- **Bed materials.** Beside every drawn channel, the bed and the banks
    the river shaped turn to gravel where the water runs at 1 m/s and
    over, sand from 0.4 to 1 m/s and mud below, out to a band that grows
    with the river, and to rock where its walls are rock. They follow the
    drawn edge through a distance field at four times the heightmap's
    resolution, so they never show the samples' squares. Point bars of
    gravel or sand reach out to 1.5 widths on the inner side of bends;
    mouths near sea level are sand. Gravel within 2 m of the water is
    wet: darker and glossy.
- **Stones.** Fast streams carry real stones, drawn as boulder meshes
    on a 1 m lattice of their own. Their median size is the largest the
    flow can move (Shields: depth × slope / 0.099, at most a third of the
    width), they lie where the stream power passes 30 W/m² and the water
    runs over 0.5 m/s, and their sizes follow a power law from 0.7 to 3
    times the median. Cobbles a third the size line the edge within
    0.5 m, and a few blocks lie on the bank out to 1.5 m. Stones by the
    water are wet and carry no lichen. The water breaks on the same
    stones: foam on the upstream side of those breaking the surface, a
    bright riffle over those just under it, eddies behind them. By the
    banks the water barely moves, so stones there break no foam. Stone
    foam fades out from 30 to 90 m; stones under a pixel or so across
    are not drawn.
- **Rock-walled rapids.** Where the stream power, 1000 × 9.81 × Q × S /
    w, is over 300 W/m², the slope over 2 % and the discharge over
    1 m³/s, the banks turn to rock (fully at 600 W/m², 3 % and 4 m³/s).
    A rivulet is steep enough for that power but runs over soil and
    boulders, not in a bedrock trench. Where the banks are rock, the
    carved walls steepen towards vertical, the stones grow to 0.8 to 2 m
    with foam streaks behind them, and standing waves and whitewater
    grow.

### Green banks

Ground near water is moister: within `R = 25 + 12 √Q` metres of a river
(25 to 400 m) and 40 m of a lake, classification adds up to 0.45 to the
moisture above the bank, falling off as `(1 - d / R)²`. Dry country by
water turns to meadow and thicket, trees in meadows and savannah grow up
to 2.5 times as dense, and grass grows denser. The moisture makes
bankside turf greener and fresher, not darker: leaf litter and mud
follow the ground's own moisture, and the terrain shader brightens and
saturates grass by the riparian value. `riparian` scales it from 0 (off)
to 2; the default is 1. Glacier ice stays as it is, and trees never grow
on bars or in the channel.

A band of plants lines every drawn channel, from its edge out:

- **Tufts** within 0.6 m lean over the water.
- **Tall herbs and ferns**, 0.5 to 1.5 m, grow from 0.3 to 3 m out,
    denser in shade. Above the trees they are sedges.
- **Scrub** 2 to 6 m tall grows from 1 m to 4 m plus half the width
    out, in clumps with gaps every 10 to 40 m where the water shows
    through, leaning over the stream: willow and alder by temperate
    streams, willow alone in the cold, shrubs and palms in the tropics,
    shrubs and young cypress in swamps, and only a narrow line in dry
    savannah. None grows above the trees or on snow. It thins on banks
    steeper than 35 degrees and where stream power passes 300 W/m²,
    thickens by slow water, and on braided threads is sparse pioneer
    scrub.
- **From afar**, where the scrub thins out with distance, the ground
    along the water darkens to a deeper green in its place.

`riparian` scales the band too: 0 removes it and 2 doubles it. The tree
density mask scales the scrub, and the grass mask the tufts and herbs.

```ts
engine.setWater({
  enabled: true,
  seaLevelMetres: 0,
  waveScale: 0.8,
  reflectivity: 0.35,
  shorelineSoftnessMetres: 6,
  rivers: { riparian: 2 }
});
```

### Waterfalls

Each waterfall has:

- **a sheet** following the path of water leaving the lip, `x = v t` and
    `y = -g t² / 2`, 12 rows down and at least three across, pushed out to
    lie just over the rock where the face is not vertical;
- **streaks** falling at the impact speed, `√(2 g drop)`, breaking up into
    aerated white water towards the foot, thin at the edges, and lit
    through from behind when the sun is beyond it;
- **mist**: 16 to 64 camera-facing sprites rising and drifting downwind
    from the foot, soft where they meet the ground, and not drawn beyond
    1.5 km;
- **a plunge pool** of churned foam in rings spreading from the foot.

`engine.getWaterfalls()` lists every waterfall, and a cascade as one
waterfall from its first lip to its last foot, where its water lands:

```ts
const [fall] = engine.getWaterfalls();

if (fall) {
  const [x, y, z] = fall.position;
  const back = fall.heightMetres * 3;
  engine.setCamera({
    position: [x + back, y + fall.heightMetres * 0.5, z],
    target: [x, y + fall.heightMetres * 0.4, z],
    fieldOfViewDegrees: 55
  });
}
```

### Frozen water

Lakes, rivers and waterfalls follow the climate (see
[`docs/biomes.md`](biomes.md#climate-temperature)):

- **Lakes** freeze where the temperature at their outlet is below 0 °C,
    fully at -2 °C, shallow margins first. Lake ice is drawn as great
    smooth sheets meeting at pressure cracks, under snow that deepens as
    it gets colder and is blown thin in patches of clear, dark ice. No
    ripples, flow or foam show through it.
- **Rivers** freeze below -5 °C, fully at -7 °C: snow-dusted ice with open
    dark leads over the fastest water (over 2 m/s).
- **Waterfalls** below -8 °C become icefalls: still, blue-white ice ribbed
    down the fall line, with no mist and no sound, over a pool frozen
    like a lake's ice.

A map with no water below 0 °C pays nothing for any of this.

```ts
engine.setBiomes({ meanTemperatureCelsius: -18 });
```

### Wet banks and reeds

Only a thin margin, within 2 m of a river, lake or waterfall, darkens
by up to 30 %, turns glossy, and turns towards wet, brown earth where
the bank is gentle. Near the camera it follows the drawn edge through
the channel field.
Distances are measured from the water's true edge: a stream narrower
than a sample is measured from its drawn centreline less half its
width, so the margin and the grass beside it follow the stream, not the
30 m sample it runs through. Within 12 m grass grows
denser and greener (when grass is on), and beside lakes, oxbows and
rivers slower than 0.6 m/s, in temperate and warm climates, reeds 1.4 to
2.2 m tall grow in the same wind: within 3 m of the water, or on the
first samples from the shore where samples are further apart. Wet banks
reach 40 m from the water, so maps with samples more than 80 m apart
have no reeds.

### Painted water (`setWaterMask`)

`engine.setWaterMask(mask)` paints rivers and lakes into the terrain,
which carves and draws them like its own. Each byte of `mask.data` is one
sample, row-major, north row first:

- `0`: no water;
- `1` to `127`: a river brush, `1` painting a river 1 m wide and `127` one
    60 m wide (or wider, where its discharge asks for it);
- `128` to `255`: a lake or pond.

Painted lakes are flattened into basins below their rim, `max(1.5 m,
0.05 × √area)` deep but never within 0.5 m of the sea, and fill to their
lowest rim point, where their outlet joins the drainage. Painted water
whose rim is less than 1.5 m above the sea is sea water: it lies level
with the sea, shelves out from the land at 1 in 50 like a natural coast,
and is never deeper than the sea floor beside it, so painting water on a
coast widens the sea with no edge or darker patch.

Painted river strokes are thinned to centrelines, with the short spurs
thinning leaves at bends and ends dropped, and each stroke becomes one
river however wide it was painted. A river runs downhill from its higher
end (towards the nearer sea or lake when both ends are level); a stroke
that ends on another painted river is its tributary and flows into it.
Rivers are cut with the same channel form as natural ones, and are as
deep, as fast and the same colour as a natural river of their width: a
river painted 60 m wide carries at least the 490 m³/s such a river
would, and is about 4 m deep. Below their end they join the natural
network; natural streams end where they reach painted water. Painted
water always wins over generated water.

A mask of another size is resampled to the terrain (the nearest value
decides between no water, river and lake; river strength is
interpolated), with a `"warning"` event. The mask stays through
`setWater()` and is cleared when new terrain loads. `null` removes it and
restores the terrain exactly.

```ts
const width = 512;
const height = 512;
const data = new Uint8Array(width * height);

// A 20 m wide river across the middle, and a round pond.
for (let x = 100; x < 400; x += 1) {
  data[256 * width + x] = 40;
}

for (let y = 0; y < height; y += 1) {
  for (let x = 0; x < width; x += 1) {
    if (Math.hypot(x - 380, y - 140) < 25) {
      data[y * width + x] = 255;
    }
  }
}

engine.setWaterMask({ width, height, data });
// Later: remove it and restore the terrain.
engine.setWaterMask(null);
```

A wrong type throws a `TypeError`; a wrong size or data length throws a
`VistaWasmError` with the code `OPTIONS_INVALID`.

### Sound hooks

`engine.getWaterSounds(x, y, z)` returns the loudest river, waterfall,
lake shore and surf near a position, for your own audio. Each is
`{ distanceMetres, loudness, position }`, or `null` when there is none
within 400 m (rivers and lake shores), 1500 m (waterfalls) or 600 m
(surf). Loudness runs from 0 to 1, from the source's strength over its
distance squared: river strength is speed × width, waterfall strength
discharge × drop, surf strength the wave height. Frozen water is silent.
A query reads only the grid cells around the position and costs a few
microseconds, so it can run every frame; VistaWASM plays no audio itself.

```ts
const audio = new AudioContext();
const river = new Audio("river-loop.ogg");
river.loop = true;
const gain = audio.createGain();
audio.createMediaElementSource(river).connect(gain).connect(audio.destination);
await river.play();

// `controls` from attachFlyCameraControls(), or your own camera.
engine.on("stats", () => {
  const [x, y, z] = controls.getCamera().position;
  const sounds = engine.getWaterSounds(x, y, z);
  gain.gain.setTargetAtTime(sounds.river?.loudness ?? 0, audio.currentTime, 0.2);
});
```

## Shading

- **Depth colour.** The shader reads the real water depth from the
  terrain heights. Shallow water shows the sea bed through
  `shallowColour`; it fades to `deepColour` by `clarityMetres`.
- **Reflections.** Schlick Fresnel reflects the same analytic sky as the
  sky pass, including cloud reflections, plus a GGX sun glitter.
  `reflectivity` scales reflection strength. With `reflections:
  "screen"` (the default), water also reflects what is on screen: the
  terrain, trees and banks. A reflected ray is followed through a
  half-resolution copy of the scene, and falls back to the sky where it
  leaves the screen or finds nothing, so the edges of the screen show
  the sky. Ripples and waves break the reflection up. `reflections:
  "sky"` reflects the sky and clouds only, and costs less.
- **Subsurface light** glows through thin wave crests facing the sun.
- **Foam** appears on folding crests, along shorelines, in rapids, on the
  outer bank of bends and in plunge pools, scaled by `foam`.
- **Cloud shadows** and fog apply to water like everything else.

## Sea ice

Cold seas freeze. The concentration of ice follows the sea's mean
temperature: open water above -1.5 °C, full pack ice below -7.5 °C, and
loose floes in between. Over the terrain the temperature comes from the
climate (see [`docs/biomes.md`](biomes.md#climate-temperature)); beyond
it, from the sea-level mean of `BiomeOptions.meanTemperatureCelsius` and
`temperatureBias`. Where the climate is colder than -10 °C, fast ice is
frozen solid to the shore for 200 m out.

- **Floes** come in three sizes: big floes about 600 m across, broken by
    a 120 m scale into bays, cracks and loose pieces, with 25 m cakes in
    the gaps. Their edges are rounded and irregular. Beyond 1.5 to 3 km
    the cakes fade out, and beyond 4 to 8 km the middle scale, so distant
    ice does not shimmer. The pack drifts with the weather's wind at 2 %
    of its speed.
- **Leads** are long, narrow cracks of open water, 5 to 40 m wide, that
    meander across the sea. They grow fewer and narrower as the pack
    closes up.
- **Pressure ridges** run along some floe boundaries as thin, bright,
    raised lines.
- **Slush.** In a close pack, grey brash and grease ice fills the gaps
    between floes, matt and without glint.
- **Shading.** Floes are snow-white with a blue shadow side, lit like
    snow on the ground, with bevelled rims. About one in five carries only
    thin, patchy snow over blue-grey ice. The water in the leads is dark.
- **Calm.** Waves, ripples, and foam die down as the concentration rises.
    Where the ice is solid, the water beneath is not shaded at all.

Sea ice forms on the ocean; lakes, rivers and waterfalls freeze by their
own rules (see [Frozen water](#frozen-water)). A map whose sea never
freezes pays nothing for sea ice.

```ts
engine.setBiomes({ meanTemperatureCelsius: -12 });
```

## Interaction with mist

When `MistOptions.riseAboveWater` is enabled, extra mist appears near
`seaLevelMetres`. This only takes effect while `WaterOptions.enabled` is
`true`.

## Interaction with weather

While the weather system drives water (`WeatherOptions.effects.water`,
on by default), the wind sets the sea state (see [Sea state](#sea-state)),
steepens the swell and turns the current downwind. Rain rings the
surface with raindrop ripples, where it rains. Under cloud, sun glitter
fades with the direct sunlight. Switch the effect off to keep your own
wave settings. See [`docs/weather.md`](weather.md).

## What water does not do

- No buoyancy or gameplay interaction. Compare your own height query (see
  [`docs/game-development.md`](game-development.md#querying-terrain-height-for-gameplay))
  against `seaLevelMetres` yourself.
- No reflections of water in water, and nothing off screen is reflected:
  screen reflections fall back to the sky there.
- Rivers do not change sea level or flood terrain: discharge is a yearly
  mean, with no floods or droughts.
- No audio playback; `getWaterSounds()` tells your own audio where the
  water is.
