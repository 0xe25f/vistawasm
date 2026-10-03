// Shortens the names wasm-bindgen gives the functions passed between the
// WASM module and its JavaScript glue, in the module, the glue and the
// glue's declarations.
//
//   node scripts/shorten-names.mjs [package-directory]
//
// wasm-bindgen names each import after the JavaScript it calls, with a
// 16-digit hash, such as `__wbg_set_format_7f2bdbfb101b1ae1`, and each
// closure shim after the full type path of its closure, hundreds of
// characters long. There are hundreds of them, and the hashes compress
// poorly. This gives each a short name instead, in the order the module
// lists them, so builds stay reproducible. It changes nothing else: the
// module imports and exports the same functions, by name, to and from
// the same objects. The exports the glue's users call keep their names.
//
// It fails, leaving every file as it was, if a name it renames is not
// where it expects it in the glue.
import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const directory = process.argv[2] ?? "dist/pkg";
const wasmPath = join(directory, "vista_wasm_bg.wasm");
const gluePath = join(directory, "vista_wasm.js");
const typesPath = join(directory, "vista_wasm.d.ts");
const glueModule = "./vista_wasm_bg.js";
const IMPORTS = 2;
const EXPORTS = 7;

/** An unsigned LEB128 number at `at`: its value and the next offset. */
function readLeb(bytes, at) {
  let value = 0;
  let shift = 0;
  let byte;

  do {
    byte = bytes[at++];
    value += (byte & 0x7f) * 2 ** shift;
    shift += 7;
  } while (byte & 0x80);

  return [value, at];
}

function writeLeb(value) {
  const out = [];

  do {
    let byte = value % 128;
    value = Math.floor(value / 128);

    if (value > 0) {
      byte |= 0x80;
    }

    out.push(byte);
  } while (value > 0);

  return out;
}

/** The offset just past the limits at `at` (a flag, a minimum and maybe a maximum). */
function skipLimits(bytes, at) {
  const flags = bytes[at++];
  [, at] = readLeb(bytes, at);

  if (flags & 1) {
    [, at] = readLeb(bytes, at);
  }

  return at;
}

/** The offset just past an import's description at `at`. */
function skipImportDescription(bytes, at, name) {
  const kind = bytes[at++];

  if (kind === 0x00) {
    return readLeb(bytes, at)[1];
  } else if (kind === 0x01) {
    return skipLimits(bytes, at + 1);
  } else if (kind === 0x02) {
    return skipLimits(bytes, at);
  } else if (kind === 0x03) {
    return at + 2;
  } else if (kind === 0x04) {
    return readLeb(bytes, at + 1)[1];
  }

  throw new Error(`Import ${name} has an unknown kind, ${kind}.`);
}

/** Short names that are valid JavaScript identifiers, after `prefix`. */
function* shortNames(prefix) {
  const letters = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";
  const rest = `${letters}0123456789`;

  for (const first of letters) {
    yield prefix + first;
  }

  for (const first of letters) {
    for (const second of rest) {
      yield prefix + first + second;
    }
  }
}

const wasm = readFileSync(wasmPath);
let glue = readFileSync(gluePath, "utf8");
let types = readFileSync(typesPath, "utf8");
const decoder = new TextDecoder();
const encoder = new TextEncoder();

if (wasm.readUInt32LE(0) !== 0x6d736100) {
  throw new Error(`${wasmPath} is not a WebAssembly module.`);
}

/** The module's sections, by id, with where each starts and ends. */
const sections = new Map();

for (let at = 8; at < wasm.length; ) {
  const id = wasm[at];
  const [size, start] = readLeb(wasm, at + 1);
  sections.set(id, { at, start, end: start + size });
  at = start + size;
}

for (const id of [IMPORTS, EXPORTS]) {
  if (!sections.has(id)) {
    throw new Error(`${wasmPath} has no ${id === IMPORTS ? "import" : "export"} section.`);
  }
}

/**
 * Read the strings and the rest of each entry of a section: an import's
 * module, name and description, or an export's name, kind and index.
 */
function readEntries(id) {
  const { start, end } = sections.get(id);
  let [count, cursor] = readLeb(wasm, start);
  const entries = [];

  for (let index = 0; index < count; index++) {
    const strings = [];

    for (let field = 0; field < (id === IMPORTS ? 2 : 1); field++) {
      const [length, from] = readLeb(wasm, cursor);
      strings.push(decoder.decode(wasm.subarray(from, from + length)));
      cursor = from + length;
    }

    const restStart = cursor;
    cursor =
      id === IMPORTS
        ? skipImportDescription(wasm, cursor, strings[1])
        : readLeb(wasm, cursor + 1)[1];
    entries.push({ strings, rest: wasm.subarray(restStart, cursor) });
  }

  if (cursor !== end) {
    throw new Error(`${wasmPath}'s section ${id} did not parse to its end.`);
  }

  return entries;
}

function writeSection(id, entries) {
  const body = [...writeLeb(entries.length)];

  for (const { strings, rest } of entries) {
    for (const text of strings) {
      const bytes = encoder.encode(text);
      body.push(...writeLeb(bytes.length), ...bytes);
    }

    body.push(...rest);
  }

  return Buffer.from([id, ...writeLeb(body.length), ...body]);
}

/** Whole-word occurrences of `name` in `text`. */
const word = (name) => new RegExp(`(?<![\\w$])${name}(?![\\w$])`, "g");

// Imports from the glue: each is a key of its import object, once.
const imports = readEntries(IMPORTS);
const importNames = shortNames("");
let importsRenamed = 0;

for (const entry of imports) {
  const [module, name] = entry.strings;

  if (module !== glueModule) {
    continue;
  }

  const short = importNames.next().value;
  const key = new RegExp(`^(\\s+)${name}(: function\\b)`, "gm");
  const keys = glue.match(key)?.length ?? 0;
  const anywhere = glue.match(word(name))?.length ?? 0;

  if (keys !== 1 || anywhere !== 1) {
    throw new Error(`${gluePath} holds ${name} ${anywhere} times, ${keys} as an import key; expected once.`);
  }

  glue = glue.replace(key, `$1${short}$2`);
  entry.strings[1] = short;
  importsRenamed++;
}

// Closure shims: only the glue calls them, as `wasm.name`, and wraps each
// in a function of the same name. They are prefixed, so they cannot meet
// the glue's own short names.
const exports = readEntries(EXPORTS);
const closureNames = shortNames("__c");
let closuresRenamed = 0;

for (const entry of exports) {
  const [name] = entry.strings;

  if (!name.includes("___convert__closures_")) {
    continue;
  }

  const short = closureNames.next().value;

  if (!glue.includes(`wasm.${name}(`)) {
    throw new Error(`${gluePath} never calls wasm.${name}.`);
  }

  glue = glue.replace(word(name), short);
  types = types.replace(word(name), short);
  entry.strings[0] = short;
  closuresRenamed++;
}

// The sections, rewritten in place, in the module's order.
const replaced = [
  [sections.get(IMPORTS), writeSection(IMPORTS, imports)],
  [sections.get(EXPORTS), writeSection(EXPORTS, exports)]
].sort((a, b) => a[0].at - b[0].at);
const parts = [];
let last = 0;

for (const [{ at, end }, section] of replaced) {
  parts.push(wasm.subarray(last, at), section);
  last = end;
}

parts.push(wasm.subarray(last));
const out = Buffer.concat(parts);

writeFileSync(wasmPath, out);
writeFileSync(gluePath, glue);
writeFileSync(typesPath, types);
console.log(
  `Shortened ${importsRenamed} import and ${closuresRenamed} closure names: the module is ${wasm.length - out.length} bytes smaller.`
);
