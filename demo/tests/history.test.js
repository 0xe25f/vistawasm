import { describe, expect, it } from "vitest";
import { createDocument } from "../src/paint/document.js";
import { applyDab, createStroke, dabRect, toolLayer } from "../src/paint/brushes.js";
import { History, TILE } from "../src/paint/history.js";

/** Paint one stroke of dabs as the paint tab does: copy tiles, then write. */
function paint(doc, history, tool, points, radius = 20) {
  const stroke = createStroke(doc, tool, points[0][0], points[0][1], 5);
  history.begin();

  for (const [x, y] of points) {
    history.touch(doc.layers, toolLayer(tool), dabRect(doc.size, x, y, radius));
    applyDab(doc, stroke, { x, y, radius, strength: 0.8, falloff: "smooth", biome: 2 });
  }

  return history.end(doc.layers);
}

const snapshot = (doc) => Object.fromEntries(Object.entries(doc.layers).map(([name, layer]) => [name, layer.slice()]));

describe("History", () => {
  it("round-trips a stroke through undo and redo byte for byte", () => {
    const doc = createDocument({ size: 256, heightMetres: 40 });
    const history = new History(doc.size);
    paint(doc, history, "raise", [[60, 60], [70, 64], [80, 70]]);
    const before = snapshot(doc);
    paint(doc, history, "erode", [[64, 64], [90, 70], [130, 70]]);
    paint(doc, history, "biome", [[10, 250], [40, 200]]);
    const after = snapshot(doc);

    expect(history.undo(doc.layers)).not.toBeNull();
    expect(history.undo(doc.layers)).not.toBeNull();
    expect(snapshot(doc)).toEqual(before);
    expect(history.canRedo).toBe(true);

    history.redo(doc.layers);
    history.redo(doc.layers);
    expect(snapshot(doc)).toEqual(after);
    expect(history.redo(doc.layers)).toBeNull();
  });

  it("copies only the tiles a stroke touches", () => {
    const doc = createDocument({ size: 512 });
    const history = new History(doc.size);
    paint(doc, history, "treesMore", [[100, 100]], 10);
    const [record] = history.undoStack;

    expect(record.tiles.map((tile) => tile.bounds)).toEqual([{ x0: TILE, y0: TILE, x1: 2 * TILE, y1: 2 * TILE }]);
    expect(record.bytes).toBe(2 * TILE * TILE);
  });

  it("cancels a stroke in progress", () => {
    const doc = createDocument({ size: 256 });
    const history = new History(doc.size);
    const before = snapshot(doc);
    const stroke = createStroke(doc, "raise", 50, 50);
    history.begin();
    history.touch(doc.layers, "height", dabRect(256, 50, 50, 30));
    applyDab(doc, stroke, { x: 50, y: 50, radius: 30, strength: 1, falloff: "smooth" });
    history.cancel(doc.layers);

    expect(snapshot(doc)).toEqual(before);
    expect(history.canUndo).toBe(false);
  });

  it("clears redo when a new stroke is painted", () => {
    const doc = createDocument({ size: 256 });
    const history = new History(doc.size);
    paint(doc, history, "lake", [[100, 100]]);
    history.undo(doc.layers);
    paint(doc, history, "river", [[20, 20]]);

    expect(history.canRedo).toBe(false);
    expect(history.undoStack).toHaveLength(1);
  });

  it("drops the oldest strokes beyond the stroke limit", () => {
    const doc = createDocument({ size: 256 });
    const history = new History(doc.size, { maxStrokes: 3 });

    for (let stroke = 0; stroke < 5; stroke += 1) {
      paint(doc, history, "raise", [[40 * stroke + 20, 40]], 8);
    }

    expect(history.undoStack).toHaveLength(3);
    // The first two strokes (x 20 and 60) are gone, so undoing all leaves them raised.
    while (history.undo(doc.layers)) {
      // Undo everything that is left.
    }

    expect(doc.layers.height[40 * 256 + 20]).toBeGreaterThan(0);
    expect(doc.layers.height[40 * 256 + 60]).toBeGreaterThan(0);
    expect(doc.layers.height[40 * 256 + 100]).toBe(0);
  });

  it("drops the oldest strokes beyond the memory cap", () => {
    const doc = createDocument({ size: 256 });
    // One float tile, before and after, is 32 KB; allow three.
    const history = new History(doc.size, { maxBytes: 3 * 2 * TILE * TILE * 4 });

    for (let stroke = 0; stroke < 4; stroke += 1) {
      paint(doc, history, "raise", [[64 * stroke + 32, 32]], 8);
    }

    expect(history.undoStack).toHaveLength(3);
    expect(history.bytes).toBe(3 * 2 * TILE * TILE * 4);
    expect(history.undoStack[0].tiles[0].bounds.x0).toBe(TILE);
  });
});
