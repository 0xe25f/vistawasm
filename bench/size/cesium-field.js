// A 512 x 512 height field 6,132 m across (12 m samples, as in the other
// apps) for CesiumJS, which has no terrain generator: it streams terrain
// tiles, so the field is served through a CustomHeightmapTerrainProvider.
// Heights come from the same kind of sine field the three-terrain app uses.
import { Cartographic, CustomHeightmapTerrainProvider, Math as CesiumMath } from "cesium";

export const CENTRE = Cartographic.fromDegrees(7.5, 46.5);
const HALF_METRES = 3066;
const EARTH_RADIUS = 6378137;
const SAMPLES = 65;

function height(eastMetres, northMetres) {
  if (Math.abs(eastMetres) > HALF_METRES || Math.abs(northMetres) > HALF_METRES) {
    return 0;
  }

  const x = (eastMetres + HALF_METRES) / 12;
  const z = (northMetres + HALF_METRES) / 12;

  return 150 + 120 * Math.sin(x * 0.02) * Math.cos(z * 0.017);
}

export function fieldProvider() {
  const provider = new CustomHeightmapTerrainProvider({
    width: SAMPLES,
    height: SAMPLES,
    callback: (x, y, level) => {
      const rectangle = provider.tilingScheme.tileXYToRectangle(x, y, level);
      const heights = new Float32Array(SAMPLES * SAMPLES);
      const cosLatitude = Math.cos(CENTRE.latitude);

      for (let row = 0; row < SAMPLES; row += 1) {
        const latitude = rectangle.north - (row / (SAMPLES - 1)) * rectangle.height;
        const north = (latitude - CENTRE.latitude) * EARTH_RADIUS;

        for (let column = 0; column < SAMPLES; column += 1) {
          const longitude = rectangle.west + (column / (SAMPLES - 1)) * rectangle.width;
          const east = (longitude - CENTRE.longitude) * EARTH_RADIUS * cosLatitude;
          heights[row * SAMPLES + column] = height(east, north);
        }
      }

      return heights;
    }
  });

  return provider;
}

export const VIEW = {
  destination: Cartographic.toCartesian(
    new Cartographic(CENTRE.longitude, CENTRE.latitude - CesiumMath.toRadians(0.03), 900)
  ),
  orientation: { heading: 0, pitch: CesiumMath.toRadians(-10), roll: 0 }
};
