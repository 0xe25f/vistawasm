import type { VistaErrorCode } from "./types.js";

/**
 * Error class used by the public VistaWASM TypeScript API.
 */
export class VistaWasmError extends Error {
  public readonly code: VistaErrorCode;

  public readonly details?: unknown;

  public constructor(code: VistaErrorCode, message: string, details?: unknown) {
    super(message);
    this.name = "VistaWasmError";
    this.code = code;
    this.details = details;
  }
}

/**
 * Convert an unknown thrown value into a VistaWASM error.
 */
export function toVistaWasmError(error: unknown): VistaWasmError {
  if (error instanceof VistaWasmError) {
    return error;
  }

  if (isObject(error) && typeof error.code === "string" && typeof error.message === "string") {
    return new VistaWasmError(error.code as VistaErrorCode, error.message, error.details);
  }

  if (error instanceof Error) {
    return new VistaWasmError("INTERNAL_ERROR", error.message, error);
  }

  return new VistaWasmError("INTERNAL_ERROR", "VistaWASM encountered an internal fault.", error);
}

function isObject(value: unknown): value is { code?: unknown; message?: unknown; details?: unknown } {
  return typeof value === "object" && value !== null;
}

/** An `OPTIONS_INVALID` error. */
export function invalid(message: string): VistaWasmError {
  return new VistaWasmError("OPTIONS_INVALID", message);
}

/** An `INVALID_DEM` error for a malformed file. */
export function malformed(message: string): VistaWasmError {
  return new VistaWasmError("INVALID_DEM", message);
}

/** Keys that reach an object's prototype when the object is used as a map. */
export const PROTOTYPE_KEYS: readonly string[] = ["__proto__", "constructor", "prototype"];

/**
 * How deeply options objects may nest. The deepest real option, a weather
 * preset's `next` weights, sits four levels down.
 */
export const MAX_OPTIONS_DEPTH = 8;

/**
 * Check that `value` is an options object that is safe to copy, merge
 * and hand to the engine: no key at any depth reaches a prototype, and
 * it nests at most `MAX_OPTIONS_DEPTH` deep, which also catches an object
 * that contains itself (the engine would recurse into it without end).
 * Typed arrays are data, so they are not looked into.
 */
export function checkOptions<Options = Record<string, unknown>>(call: string, value: unknown, path = "", depth = 0): Options {
  if (typeof value !== "object" || value === null || (!depth && Array.isArray(value))) {
    if (!depth) {
      throw new TypeError(`${call} expects an options object, but it was given ${value === null ? "null" : Array.isArray(value) ? "an array" : typeof value}.`);
    }

    return value as Options;
  }

  if (depth > MAX_OPTIONS_DEPTH) {
    throw invalid(`${call} options nest more than ${MAX_OPTIONS_DEPTH} levels deep at ${path}, or contain themselves.`);
  }

  if (!ArrayBuffer.isView(value)) {
    for (const [key, item] of Object.entries(value)) {
      if (PROTOTYPE_KEYS.includes(key)) {
        throw invalid(`${call} options may not hold the key "${key}"${path && ` (in ${path.slice(1)})`}, as it could change object prototypes.`);
      }

      checkOptions(call, item, `${path}.${key}`, depth + 1);
    }
  }

  return value as Options;
}

/**
 * The keys an options object may hold: `0` for any value, the keys of a
 * nested object, or `[keys]` for a list of objects. Generated from the
 * engine's option types into `option-keys.ts`.
 */
export type OptionKeys = { readonly [key: string]: 0 | OptionKeys | readonly [OptionKeys] };

/**
 * `checkOptions()` on a structured clone of `value`, which is returned.
 * The clone reads every property once, so a getter cannot hand the check
 * one value and the engine another, and later changes to the caller's
 * object do not reach the engine's copy. Functions, symbols and other
 * values that are not plain data are rejected. With `keys`, keys the
 * engine does not read are taken out of the copy, with a warning each in
 * `warnings` (see `dropUnknownKeys()`).
 */
export function cloneOptions<Options = Record<string, unknown>>(call: string, value: unknown, keys?: OptionKeys, warnings?: string[]): Options {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    return checkOptions<Options>(call, value);
  }

  let copy: unknown;

  try {
    copy = structuredClone(value);
  } catch (error) {
    throw new TypeError(`${call} options must be plain data (numbers, strings, booleans, arrays, typed arrays and objects): ${(error as Error)?.message ?? String(error)}`);
  }

  checkOptions(call, copy);

  if (keys) {
    dropUnknownKeys(copy as Record<string, unknown>, keys, warnings ?? [], call);
  }

  return copy as Options;
}

/**
 * Take the keys `keys` does not list out of `options`, at every depth,
 * and add a warning for each, naming the closest valid key. The engine
 * would ignore them silently, so a mistyped key would otherwise vanish.
 * They become errors in the next major version.
 */
export function dropUnknownKeys(options: Record<string, unknown>, keys: OptionKeys, warnings: string[], call: string, path = ""): void {
  for (const key of Object.keys(options)) {
    const known = keys[key];
    const value = options[key];
    const at = `${path}${key}`;

    if (known === undefined) {
      delete options[key];
      const closest = closestKey(key, Object.keys(keys));
      warnings.push(`${call} option "${at}" is not one the engine reads, so it was ignored.${closest ? ` Did you mean "${closest}"?` : ""} Unknown options become errors in the next major version.`);
    } else if (Array.isArray(known) && Array.isArray(value)) {
      value.forEach((item, index) => typeof item === "object" && item !== null && dropUnknownKeys(item, known[0], warnings, call, `${at}[${index}].`));
    } else if (typeof known === "object" && !Array.isArray(known) && typeof value === "object" && value !== null && !Array.isArray(value) && !ArrayBuffer.isView(value)) {
      dropUnknownKeys(value as Record<string, unknown>, known as OptionKeys, warnings, call, `${at}.`);
    }
  }
}

/** The key in `keys` within a few edits of `key`, ignoring case, if any. */
function closestKey(key: string, keys: string[]): string | undefined {
  let best: string | undefined;
  let bestDistance = Infinity;

  for (const candidate of keys) {
    const [a, b] = [key.toLowerCase(), candidate.toLowerCase()];
    // Levenshtein distance, one row at a time.
    let row = Array.from({ length: b.length + 1 }, (_, index) => index);

    for (let i = 0; i < a.length; i += 1) {
      const next = [i + 1];

      for (let j = 0; j < b.length; j += 1) {
        next.push(Math.min(row[j + 1] + 1, next[j] + 1, row[j] + (a[i] === b[j] ? 0 : 1)));
      }

      row = next;
    }

    if (row[b.length] < bestDistance && row[b.length] <= Math.max(1, Math.floor(b.length / 4))) {
      [best, bestDistance] = [candidate, row[b.length]];
    }
  }

  return best;
}
