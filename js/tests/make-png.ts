import zlib from "node:zlib";

export const CHANNELS: Record<number, number> = { 0: 1, 2: 3, 3: 1, 4: 2, 6: 4 };
const ADAM7 = [
  [0, 0, 8, 8],
  [4, 0, 8, 8],
  [0, 4, 4, 8],
  [2, 0, 4, 4],
  [0, 2, 2, 4],
  [1, 0, 2, 2],
  [0, 1, 1, 2]
];

export interface PngSpec {
  width: number;
  height: number;
  colourType: number;
  bitDepth: number;
  /** Samples, row-major, channels interleaved. */
  samples: number[];
  interlace?: boolean;
  /** The filter each row uses, by row index; defaults to cycling 0 to 4. */
  filter?: (row: number) => number;
  palette?: number[];
  trns?: number[];
  text?: Record<string, string>;
}

export function chunk(type: string, data: Uint8Array): Buffer {
  const head = Buffer.alloc(8);
  head.writeUInt32BE(data.length, 0);
  head.write(type, 4, "latin1");
  const body = Buffer.concat([head.subarray(4), data]);
  const crc = Buffer.alloc(4);
  crc.writeUInt32BE(zlib.crc32(body), 0);
  return Buffer.concat([head.subarray(0, 4), body, crc]);
}

function paeth(a: number, b: number, c: number): number {
  const p = a + b - c;
  const [pa, pb, pc] = [Math.abs(p - a), Math.abs(p - b), Math.abs(p - c)];
  return pa <= pb && pa <= pc ? a : pb <= pc ? b : c;
}

/** Pack one row's samples at `bitDepth`, big-endian, padded to a byte. */
function packRow(samples: number[], bitDepth: number): Uint8Array {
  const out = new Uint8Array(Math.ceil((samples.length * bitDepth) / 8));

  samples.forEach((value, index) => {
    if (bitDepth === 16) {
      out[index * 2] = value >> 8;
      out[index * 2 + 1] = value & 255;
    } else if (bitDepth === 8) {
      out[index] = value;
    } else {
      const bit = index * bitDepth;
      out[bit >> 3] |= value << (8 - bitDepth - (bit & 7));
    }
  });

  return out;
}

/**
 * Build a PNG with every row filtered as `spec.filter` says, using
 * node:zlib, so the decoder is checked against an independent encoder.
 */
export function makePng(spec: PngSpec): Uint8Array {
  const channels = CHANNELS[spec.colourType];
  const bytesPerPixel = Math.max(1, (channels * spec.bitDepth) >> 3);
  const filter = spec.filter ?? ((row: number) => row % 5);
  const passes = spec.interlace ? ADAM7 : [[0, 0, 1, 1]];
  const lines: Uint8Array[] = [];
  let rowIndex = 0;

  for (const [x0, y0, dx, dy] of passes) {
    const columns = Math.ceil((spec.width - x0) / dx);
    const rows = columns > 0 ? Math.ceil((spec.height - y0) / dy) : 0;
    let previous: Uint8Array | undefined;

    for (let r = 0; r < rows; r += 1) {
      const y = y0 + r * dy;
      const samples: number[] = [];

      for (let c = 0; c < columns; c += 1) {
        const x = x0 + c * dx;

        for (let k = 0; k < channels; k += 1) {
          samples.push(spec.samples[(y * spec.width + x) * channels + k]);
        }
      }

      const raw = packRow(samples, spec.bitDepth);
      const type = filter(rowIndex);
      rowIndex += 1;
      const line = new Uint8Array(raw.length + 1);
      line[0] = type;

      for (let i = 0; i < raw.length; i += 1) {
        const left = i >= bytesPerPixel ? raw[i - bytesPerPixel] : 0;
        const above = previous ? previous[i] : 0;
        const corner = previous && i >= bytesPerPixel ? previous[i - bytesPerPixel] : 0;
        const predicted = [0, left, above, (left + above) >> 1, paeth(left, above, corner)][type];
        line[i + 1] = (raw[i] - predicted) & 255;
      }

      previous = raw;
      lines.push(line);
    }
  }

  const header = Buffer.alloc(13);
  header.writeUInt32BE(spec.width, 0);
  header.writeUInt32BE(spec.height, 4);
  header[8] = spec.bitDepth;
  header[9] = spec.colourType;
  header[12] = spec.interlace ? 1 : 0;
  const parts = [Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]), chunk("IHDR", header)];

  if (spec.palette) {
    parts.push(chunk("PLTE", Uint8Array.from(spec.palette)));
  }

  if (spec.trns) {
    parts.push(chunk("tRNS", Uint8Array.from(spec.trns)));
  }

  for (const [key, value] of Object.entries(spec.text ?? {})) {
    parts.push(chunk("tEXt", Buffer.from(`${key}\0${value}`, "latin1")));
  }

  const idat = zlib.deflateSync(Buffer.concat(lines));
  // Split the image data over several chunks, as encoders may.
  const half = idat.length >> 1;
  parts.push(chunk("IDAT", idat.subarray(0, half)), chunk("IDAT", idat.subarray(half)), chunk("IEND", new Uint8Array(0)));
  return new Uint8Array(Buffer.concat(parts));
}
