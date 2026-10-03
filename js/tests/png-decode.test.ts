import zlib from "node:zlib";
import { describe, expect, it } from "vitest";
import { encodePng } from "../src/map-export";
import { decodePng } from "../src/png-decode";
import type { ExportedMap } from "../src/types";
import { CHANNELS, chunk, makePng } from "./make-png";

/** Deterministic samples for a spec. */
function samplesFor(width: number, height: number, colourType: number, bitDepth: number, entries = 0): number[] {
  const channels = CHANNELS[colourType];
  const top = colourType === 3 ? entries - 1 : 2 ** bitDepth - 1;
  return Array.from({ length: width * height * channels }, (_, index) => (index * 7919 + (index >> 3) * 31) % (top + 1));
}

describe("decodePng", () => {
  it("decodes every colour type at every bit depth, with all five filters, plain and Adam7", async () => {
    const formats: [number, number[]][] = [
      [0, [1, 2, 4, 8, 16]],
      [2, [8, 16]],
      [3, [1, 2, 4, 8]],
      [4, [8, 16]],
      [6, [8, 16]]
    ];
    let checked = 0;

    for (const [colourType, depths] of formats) {
      for (const bitDepth of depths) {
        for (const interlace of [false, true]) {
          for (const [width, height] of [[1, 1], [3, 2], [13, 11], [33, 17]]) {
            const entries = colourType === 3 ? Math.min(2 ** bitDepth, 200) : 0;
            const palette = colourType === 3 ? Array.from({ length: entries * 3 }, (_, i) => (i * 53) % 256) : undefined;
            const samples = samplesFor(width, height, colourType, bitDepth, entries);
            const png = await decodePng(makePng({ width, height, colourType, bitDepth, samples, interlace, palette }));

            expect(png).toMatchObject({ width, height, channels: CHANNELS[colourType], bitDepth });
            expect(Array.from(png.data)).toEqual(samples);
            expect(png.data).toBeInstanceOf(bitDepth === 16 ? Uint16Array : Uint8Array);

            if (palette) {
              expect(png.palette!.length).toBe(entries * 4);
              expect(Array.from(png.palette!.subarray(4, 8))).toEqual([...palette.slice(3, 6), 255]);
            }

            checked += 1;
          }
        }
      }
    }

    expect(checked).toBe(15 * 2 * 4);
  });

  it("applies each filter on its own", async () => {
    for (let filter = 0; filter <= 4; filter += 1) {
      const samples = samplesFor(19, 7, 6, 16);
      const png = await decodePng(makePng({ width: 19, height: 7, colourType: 6, bitDepth: 16, samples, filter: () => filter }));
      expect(Array.from(png.data)).toEqual(samples);
    }
  });

  it("reads text chunks and transparency", async () => {
    const palette = [255, 0, 0, 0, 255, 0, 0, 0, 255];
    const png = await decodePng(makePng({
      width: 2,
      height: 2,
      colourType: 3,
      bitDepth: 8,
      samples: [0, 1, 2, 0],
      palette,
      trns: [0, 128],
      text: { "vistawasm:range": "[-10,250.5]", Author: "someone" }
    }));
    expect(png.text).toEqual({ "vistawasm:range": "[-10,250.5]", Author: "someone" });
    expect(Array.from(png.palette!)).toEqual([255, 0, 0, 0, 0, 255, 0, 128, 0, 0, 255, 255]);

    const grey = await decodePng(makePng({ width: 2, height: 1, colourType: 0, bitDepth: 16, samples: [5, 6], trns: [0, 6] }));
    expect(grey.transparent).toEqual([6]);
    const rgb = await decodePng(makePng({ width: 1, height: 1, colourType: 2, bitDepth: 8, samples: [1, 2, 3], trns: [0, 1, 0, 2, 0, 3] }));
    expect(rgb.transparent).toEqual([1, 2, 3]);
  });

  it("round-trips encodePng output exactly", async () => {
    const width = 23;
    const height = 9;
    const heights = Float32Array.from({ length: width * height }, (_, index) => Math.sin(index) * 500 + 100);
    const map: ExportedMap = {
      kind: "height",
      width,
      height,
      channels: 1,
      type: "float32",
      data: heights,
      encoding: { metresPerPixel: [10, 10], seaLevelMetres: 0, generator: "test" }
    };

    for (const smaller of [false, true]) {
      const blob = (await encodePng(map, { bitDepth: 16, smaller })) as Blob & { range: [number, number] };
      const png = await decodePng(new Uint8Array(await blob.arrayBuffer()));
      const [low, high] = JSON.parse(png.text["vistawasm:range"]);
      expect([low, high]).toEqual(blob.range);
      heights.forEach((value, index) => {
        const expected = Math.round(((value - low) / (high - low)) * 65535);
        expect(png.data[index]).toBe(expected);
      });
    }

    const bytes: ExportedMap = { ...map, kind: "occlusion", type: "uint8", data: Uint8Array.from(heights, (h) => Math.abs(h) % 256) };
    const grey = await decodePng(new Uint8Array(await ((await encodePng(bytes, { smaller: true })) as Blob).arrayBuffer()));
    expect(Array.from(grey.data)).toEqual(Array.from(bytes.data));

    const materials: ExportedMap = { ...map, kind: "materials", channels: 12, type: "uint8", data: Uint8Array.from({ length: width * height * 12 }, (_, i) => (i * 13) % 256) };
    const splats = (await encodePng(materials)) as Blob[];

    for (const [file, blob] of splats.entries()) {
      const png = await decodePng(new Uint8Array(await blob.arrayBuffer()));
      expect(png.channels).toBe(4);

      for (let pixel = 0; pixel < width * height; pixel += 1) {
        expect(Array.from(png.data.subarray(pixel * 4, pixel * 4 + 4))).toEqual(
          Array.from(materials.data.subarray(pixel * 12 + file * 4, pixel * 12 + file * 4 + 4))
        );
      }
    }
  });

  it("rejects a CRC mismatch, naming the chunk", async () => {
    const png = makePng({ width: 4, height: 4, colourType: 0, bitDepth: 8, samples: samplesFor(4, 4, 0, 8) });
    // The last byte of the first IDAT's data.
    const idat = Buffer.from(png).indexOf("IDAT");
    png[idat + 6] ^= 1;
    await expect(decodePng(png)).rejects.toMatchObject({
      code: "INVALID_DEM",
      message: 'PNG chunk CRC mismatch in "IDAT".'
    });
  });

  it("rejects malformed files with specific messages", async () => {
    const good = makePng({ width: 4, height: 4, colourType: 2, bitDepth: 8, samples: samplesFor(4, 4, 2, 8) });
    const withHeader = (edit: (header: Buffer) => void) => {
      const bytes = Buffer.from(good);
      edit(bytes.subarray(16, 29));
      bytes.writeUInt32BE(zlib.crc32(bytes.subarray(12, 29)), 29);
      return new Uint8Array(bytes);
    };
    const cases: [Uint8Array, RegExp][] = [
      [good.subarray(0, 7), /not a PNG/],
      [good.subarray(0, good.length - 12), /ends before its IEND/],
      [withHeader((header) => header.writeUInt32BE(8193, 0)), /from 1 to 8192/],
      [withHeader((header) => header.writeUInt32BE(0, 4)), /from 1 to 8192/],
      [withHeader((header) => (header[8] = 4)), /is not one PNG defines/],
      [withHeader((header) => (header[9] = 5)), /is not one PNG defines/],
      [withHeader((header) => (header[10] = 1)), /compression, filter or interlace/],
      [withHeader((header) => (header[9] = 3)), /no PLTE chunk/],
      [withHeader((header) => header.writeUInt32BE(5, 0)), /inflates to \d+ bytes, but its header needs/]
    ];

    for (const [bytes, message] of cases) {
      await expect(decodePng(bytes)).rejects.toMatchObject({ code: "INVALID_DEM", message: expect.stringMatching(message) });
    }

    await expect(decodePng([1, 2] as unknown as Uint8Array)).rejects.toThrow(TypeError);
  });

  it("stops inflating once the data passes what the header allows", async () => {
    // 8 x 8 grey needs 72 bytes; this inflates to 64 MiB.
    const bomb = Buffer.from(makePng({ width: 8, height: 8, colourType: 0, bitDepth: 8, samples: samplesFor(8, 8, 0, 8) }));
    const start = bomb.indexOf("IDAT") - 4;
    const tail = Buffer.concat([
      bomb.subarray(0, start),
      chunk("IDAT", zlib.deflateSync(Buffer.alloc(64 * 2 ** 20))),
      chunk("IEND", new Uint8Array(0))
    ]);
    const started = performance.now();
    await expect(decodePng(new Uint8Array(tail))).rejects.toMatchObject({
      code: "INVALID_DEM",
      message: expect.stringContaining("more than the 72 bytes")
    });
    expect(performance.now() - started).toBeLessThan(2000);
  });

  it("rejects unknown critical chunks and bad filter types", async () => {
    const good = Buffer.from(makePng({ width: 2, height: 2, colourType: 0, bitDepth: 8, samples: [1, 2, 3, 4] }));
    const at = good.indexOf("IDAT") - 4;
    const critical = Buffer.concat([good.subarray(0, at), chunk("ABCD", new Uint8Array(1)), good.subarray(at)]);
    await expect(decodePng(new Uint8Array(critical))).rejects.toThrow(/critical chunk "ABCD"/);
    const ancillary = Buffer.concat([good.subarray(0, at), chunk("abCD", new Uint8Array(1)), good.subarray(at)]);
    await expect(decodePng(new Uint8Array(ancillary))).resolves.toMatchObject({ width: 2 });

    const filtered = makePng({ width: 2, height: 2, colourType: 0, bitDepth: 8, samples: [1, 2, 3, 4], filter: () => 7 });
    await expect(decodePng(filtered)).rejects.toThrow(/filter type 7/);
  });
});
