# AGENTS.md

## Project purpose

This repository is for creating a WebAssembly library for modern browsers from 2025 onwards.

The library should be small, fast, predictable, and easy to use from JavaScript and TypeScript. It should run in current evergreen browsers without legacy fallbacks for obsolete engines.

## Audience

This file is for AI coding agents and human contributors working on the project.

Follow these instructions unless a more specific instruction appears in a closer `AGENTS.md` file or in the user request.

## Core principles

- Build a library, not an application.
- Keep the public API small and stable.
- Make browser integration simple.
- Optimise for correctness before speed.
- Avoid hidden global state.
- Avoid unnecessary dependencies.
- Prefer clear code over clever code.
- Document all public behaviour.
- Treat generated WASM, JavaScript glue, and TypeScript declarations as release artefacts.

## Target platform

Target modern browsers from 2025 onwards.

Assume support for:

- ES2022 or newer JavaScript.
- Native ES modules.
- `WebAssembly.instantiateStreaming` where the server serves the correct MIME type.
- `WebAssembly.instantiate` as a local development and test fallback.
- `BigInt` for 64-bit integer interop where needed.
- `TextEncoder` and `TextDecoder`.
- `Uint8Array`, `ArrayBuffer`, `DataView`, and typed arrays.
- Secure contexts for browser APIs that require them.

Do not add support for:

- Internet Explorer.
- Legacy CommonJS browser bundles unless explicitly requested.
- Old transpilation targets that increase bundle size without a clear need.
- Browser polyfills unless the project explicitly depends on them.

## Language and documentation style

Use British English in all documentation, comments, examples, user-facing messages, and release notes.

Use spellings such as:

- `optimise`, not `optimize`.
- `behaviour`, not `behavior`.
- `colour`, not `color`.
- `serialise`, not `serialize`.
- `initialise`, not `initialize`.
- `licence` as a noun, `license` as a verb.

Write documentation in short, direct sentences. Prefer practical examples over abstract descriptions.

## Formatting rules

Use 2-space indentation everywhere.

Do not use tabs.

Do not use 4-space indentation.

This applies to:

- JavaScript.
- TypeScript.
- Rust.
- C.
- C++.
- HTML.
- CSS.
- JSON.
- YAML.
- Markdown code examples.
- Build scripts.
- Test fixtures.

Examples must also use 2-space indentation.

### JavaScript and TypeScript example

```ts
export async function loadLibrary(url: string): Promise<WebAssembly.Instance> {
  const response = await fetch(url);
  const bytes = await response.arrayBuffer();
  const result = await WebAssembly.instantiate(bytes, {});

  return result.instance;
}
```

### Rust example

```rust
pub fn add(left: i32, right: i32) -> i32 {
  left + right
}
```

### JSON example

```json
{
  "type": "module",
  "sideEffects": false
}
```

## Repository expectations

A typical repository should contain:

```text
.
├── AGENTS.md
├── README.md
├── package.json
├── src/
├── test/
├── examples/
├── dist/
└── docs/
```

Use this structure unless the project already has a clear alternative.

Recommended responsibilities:

- `src/` contains source code.
- `test/` contains unit, integration, and browser tests.
- `examples/` contains small browser examples.
- `docs/` contains deeper usage notes.
- `dist/` contains generated release artefacts only when the repository intentionally commits them.

## Public API rules

Design the API for JavaScript and TypeScript users first.

Public API requirements:

- Export a clear async loader.
- Expose typed wrapper functions instead of raw WASM exports where practical.
- Hide memory offsets unless low-level access is the point of the library.
- Validate inputs at the JavaScript boundary.
- Return predictable errors.
- Keep names stable once released.
- Document breaking changes.

Avoid APIs that require users to understand WASM internals unless the library is explicitly low-level.

## WASM boundary rules

Crossing the JavaScript and WASM boundary has a cost. Keep the boundary simple.

Prefer:

- Fewer calls with larger batches of data.
- Typed arrays for binary data.
- Explicit ownership rules for allocated memory.
- Clear functions for allocation and disposal when manual memory management is needed.
- Copying data when it makes safety simpler.

Avoid:

- Chatty APIs that call WASM once per small value.
- Returning raw pointers without documentation.
- Sharing mutable memory without a clear contract.
- Silent truncation of numbers.
- Implicit text encoding rules.

## Memory management

Document memory ownership clearly.

For every function that allocates memory, provide one of these:

- A matching free or dispose function.
- Automatic cleanup through a wrapper.
- Clear documentation explaining why no cleanup is needed.

If exposing manual memory management, include examples such as:

```ts
const input = new Uint8Array([1, 2, 3, 4]);
const output = library.processBytes(input);

library.dispose();
```

Do not leak WASM memory in examples.

## Error handling

Errors should be clear, typed where possible, and useful for debugging.

Use JavaScript errors at the public boundary:

```ts
throw new TypeError("Expected input to be a Uint8Array.");
```

Error messages should explain what went wrong and how to fix it when practical.

Avoid:

- Empty catch blocks.
- String-only thrown errors.
- Silent fallback behaviour.
- Generic messages such as `Failed` or `Invalid input` without detail.

## TypeScript rules

If the package exposes JavaScript, provide TypeScript declarations.

Requirements:

- Use strict TypeScript settings where TypeScript is part of the project.
- Export named types for public options and results.
- Avoid `any` in public declarations.
- Prefer `unknown` for untrusted input.
- Keep declaration files in sync with runtime behaviour.

Example:

```ts
export interface WasmLibraryOptions {
  wasmUrl?: string | URL;
}

export interface WasmLibrary {
  processBytes(input: Uint8Array): Uint8Array;
  dispose(): void;
}

export function createLibrary(options?: WasmLibraryOptions): Promise<WasmLibrary>;
```

## JavaScript module rules

Use ES modules by default.

Package exports should be explicit:

```json
{
  "type": "module",
  "exports": {
    ".": {
      "types": "./dist/index.d.ts",
      "import": "./dist/index.js"
    },
    "./wasm": "./dist/library.wasm"
  }
}
```

Do not add CommonJS output unless there is a stated project requirement.

## Loading rules

Provide a reliable loader that works in browser environments.

The loader should support:

- A default WASM URL for bundlers that understand asset URLs.
- An explicit `wasmUrl` option for users who host the file themselves.
- Fallback from streaming compilation when MIME types are wrong during local testing.

Example:

```ts
async function instantiateWasm(url: URL): Promise<WebAssembly.Instance> {
  const imports = {};
  const response = await fetch(url);

  if (WebAssembly.instantiateStreaming) {
    try {
      const result = await WebAssembly.instantiateStreaming(response, imports);
      return result.instance;
    } catch (error) {
      if (response.headers.get("content-type") === "application/wasm") {
        throw error;
      }
    }
  }

  const fallbackResponse = await fetch(url);
  const bytes = await fallbackResponse.arrayBuffer();
  const result = await WebAssembly.instantiate(bytes, imports);

  return result.instance;
}
```

## MIME type

Serve `.wasm` files with this MIME type:

```text
application/wasm
```

Document this in the README and deployment notes.

## Browser examples

Every release should include at least one minimal browser example.

The example should:

- Use native ES modules.
- Load the WASM file from a relative URL.
- Avoid bundler-specific behaviour unless the example is for a named bundler.
- Show real input and output.
- Work from a local static server.

Example command:

```sh
npx serve examples
```

Do not tell users to open examples directly from `file://` unless the loader explicitly supports that workflow.

## Testing rules

Test both the WASM core and the JavaScript wrapper.

Recommended test layers:

- Unit tests for core logic.
- Interop tests for JavaScript to WASM calls.
- Browser tests in at least one Chromium-based browser.
- Type tests for public TypeScript declarations.
- Size checks for release artefacts.

Use deterministic tests. Avoid timing-sensitive tests unless they are unavoidable.

Browser tests should run from a real HTTP server so that fetch, MIME types, and module loading behave like production.

## Performance rules

Measure before optimising.

When improving performance:

- Add or update a benchmark.
- Record the input size and browser used.
- Check that memory usage does not grow unexpectedly.
- Compare against the previous implementation.

Prefer simple data transfer patterns before complex shared-memory designs.

## Security rules

Treat all JavaScript inputs as untrusted.

Validate:

- Buffer lengths.
- Numeric ranges.
- String lengths.
- Enum values.
- Offsets and indexes.

Avoid:

- Out-of-bounds memory access.
- Unchecked pointer arithmetic.
- Passing unsanitised strings into generated code.
- Fetching WASM from arbitrary remote URLs by default.
- Using `eval` or `new Function`.

If the project uses threads, shared memory, SIMD, or relaxed SIMD, document the browser requirements and security headers clearly.

## Build rules

Builds should be reproducible.

The build should:

- Clean old artefacts before generating new ones.
- Fail on warnings where practical.
- Produce deterministic file names or a clear manifest.
- Generate TypeScript declarations when JavaScript is published.
- Keep debug and release builds separate.

Do not commit large generated artefacts unless the repository's release process requires it.

## Package rules

A published package should include only necessary files.

Include:

- Runtime JavaScript.
- WASM binary.
- TypeScript declarations.
- README.
- Licence.
- Changelog, if maintained.

Exclude:

- Source maps from private source unless intentionally published.
- Test fixtures that are not needed by users.
- Local build caches.
- Temporary files.
- Editor metadata.

## Versioning rules

Use semantic versioning once the library has a public release.

Breaking changes include:

- Removing or renaming exports.
- Changing parameter meanings.
- Changing return types.
- Changing memory ownership rules.
- Requiring new browser features without a major version bump.

Document migration steps for breaking changes.

## Dependency rules

Keep dependencies minimal.

Before adding a dependency, check:

- Whether the platform already provides the feature.
- Whether the dependency works in browsers.
- Whether it increases bundle size.
- Whether it affects security or supply-chain risk.
- Whether a small local implementation would be clearer.

Prefer development dependencies for build-time tools. Avoid runtime dependencies unless they provide clear value.

## Accessibility and developer experience

Examples and documentation should be accessible and easy to inspect.

Use:

- Clear button labels in examples.
- Visible error messages.
- Plain HTML where possible.
- Console output only as a supplement.

Do not hide essential results only in developer tools.

## Comments

Use comments to explain why code exists, not to repeat what the code does.

Good:

```ts
// Some local servers serve WASM with the wrong MIME type, so fall back to ArrayBuffer loading.
```

Avoid:

```ts
// Fetch the URL.
```

## Commit and change rules

When making changes:

- Keep commits focused.
- Update tests with behaviour changes.
- Update documentation with public API changes.
- Do not reformat unrelated files.
- Do not mix generated artefact updates with unrelated source changes unless required by the build.

## Pull request checklist

Before considering work complete, confirm:

- Code uses 2-space indentation.
- No tabs are present.
- Public documentation uses British English.
- Browser examples still run.
- WASM loads over HTTP with `application/wasm`.
- TypeScript declarations match the public API.
- Tests pass.
- Build output is clean and intentional.
- Package contents are suitable for publishing.

## Agent-specific instructions

AI agents must:

- Inspect existing project structure before creating new files.
- Preserve existing public APIs unless asked to change them.
- Prefer small, reviewable changes.
- Explain trade-offs when changing API, memory, or build behaviour.
- Avoid adding frameworks without a clear requirement.
- Avoid broad rewrites unless the user asks for one.
- Keep examples concise and runnable.
- Use 2-space indentation in every generated file.
- Use British English in every generated document and comment.

AI agents must not:

- Add legacy browser support without instruction.
- Introduce tabs or 4-space indentation.
- Hide WASM loading errors.
- Create APIs that expose raw pointers without documentation.
- Add untested memory allocation paths.
- Publish or commit secrets.
- Assume Node-only APIs are available in browsers.

## Preferred completion response

When an agent finishes a task, it should report:

- What changed.
- How it was tested.
- Any risks or follow-up work.

Keep the response short and specific.
