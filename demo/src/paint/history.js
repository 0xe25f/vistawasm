// Undo and redo by tiles. Before a stroke first writes to a 64 x 64 tile
// of a layer, the tile is copied; when the stroke ends, the tile's new
// contents are copied too. Undo writes the old copies back and redo the
// new ones, so a stroke costs memory in proportion to the area it
// touched, not to the document.

/** Tile side, in samples. */
export const TILE = 64;

function tileBounds(size, tx, ty) {
  return {
    x0: tx * TILE,
    y0: ty * TILE,
    x1: Math.min(size, (tx + 1) * TILE),
    y1: Math.min(size, (ty + 1) * TILE)
  };
}

function copyTile(layer, size, bounds) {
  const width = bounds.x1 - bounds.x0;
  const copy = new layer.constructor(width * (bounds.y1 - bounds.y0));

  for (let y = bounds.y0; y < bounds.y1; y += 1) {
    copy.set(layer.subarray(y * size + bounds.x0, y * size + bounds.x1), (y - bounds.y0) * width);
  }

  return copy;
}

function pasteTile(layer, size, bounds, copy) {
  const width = bounds.x1 - bounds.x0;

  for (let y = bounds.y0; y < bounds.y1; y += 1) {
    layer.set(copy.subarray((y - bounds.y0) * width, (y - bounds.y0 + 1) * width), y * size + bounds.x0);
  }
}

/**
 * The strokes of one document of `size` samples a side. `layers` is the
 * document's `{ name: typed array }` object; history reads it when it is
 * called, so replacing a layer's array is safe between strokes.
 */
export class History {
  constructor(size, limits = {}) {
    this.size = size;
    this.maxStrokes = limits.maxStrokes ?? 100;
    this.maxBytes = limits.maxBytes ?? 256 * 2 ** 20;
    this.undoStack = [];
    this.redoStack = [];
    this.bytes = 0;
    this.current = null;
  }

  get canUndo() {
    return this.undoStack.length > 0;
  }

  get canRedo() {
    return this.redoStack.length > 0;
  }

  begin() {
    this.current = new Map();
  }

  /** Copy the tiles of `layers[name]` that `rect` overlaps and this stroke has not copied yet. */
  touch(layers, name, rect) {
    if (!this.current || !rect) {
      return;
    }

    const layer = layers[name];

    for (let ty = Math.floor(rect.y0 / TILE); ty <= Math.floor((rect.y1 - 1) / TILE); ty += 1) {
      for (let tx = Math.floor(rect.x0 / TILE); tx <= Math.floor((rect.x1 - 1) / TILE); tx += 1) {
        const key = `${name}:${tx}:${ty}`;

        if (!this.current.has(key)) {
          const bounds = tileBounds(this.size, tx, ty);
          this.current.set(key, { name, bounds, before: copyTile(layer, this.size, bounds), after: null });
        }
      }
    }
  }

  /**
   * Finish the stroke: keep each touched tile's new contents, and drop
   * the redo records and, beyond the limits, the oldest strokes. Returns
   * whether anything was recorded.
   */
  end(layers) {
    const tiles = [...(this.current?.values() ?? [])];
    this.current = null;

    if (tiles.length === 0) {
      return false;
    }

    let bytes = 0;

    for (const tile of tiles) {
      tile.after = copyTile(layers[tile.name], this.size, tile.bounds);
      bytes += tile.before.byteLength + tile.after.byteLength;
    }

    for (const record of this.redoStack) {
      this.bytes -= record.bytes;
    }

    this.redoStack = [];
    this.undoStack.push({ tiles, bytes });
    this.bytes += bytes;

    while (this.undoStack.length > this.maxStrokes || (this.bytes > this.maxBytes && this.undoStack.length > 0)) {
      this.bytes -= this.undoStack.shift().bytes;
    }

    return true;
  }

  /** Abandon the stroke in progress, restoring what it changed. Returns the changes, as `undo()` does. */
  cancel(layers) {
    const tiles = [...(this.current?.values() ?? [])];
    this.current = null;
    return this.restore(layers, tiles, "before");
  }

  /** Undo the last stroke. Returns `[{ name, rect }]` for each restored tile, or `null`. */
  undo(layers) {
    const record = this.undoStack.pop();

    if (!record) {
      return null;
    }

    this.redoStack.push(record);
    return this.restore(layers, record.tiles, "before");
  }

  /** Redo the last undone stroke, as `undo()` does. */
  redo(layers) {
    const record = this.redoStack.pop();

    if (!record) {
      return null;
    }

    this.undoStack.push(record);
    return this.restore(layers, record.tiles, "after");
  }

  restore(layers, tiles, which) {
    for (const tile of tiles) {
      pasteTile(layers[tile.name], this.size, tile.bounds, tile[which]);
    }

    return tiles.map((tile) => ({ name: tile.name, rect: tile.bounds }));
  }
}
