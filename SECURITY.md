# Security policy

## Supported versions

| Version | Supported |
| --- | --- |
| 2.x | Yes: security fixes are released as 2.x patch or minor releases. |
| 1.x | No. Upgrade to the latest 2.x release; the [changelog](CHANGELOG.md#upgrading-from-100) lists what changed. |
| Before 1.0 | No. |

Fixes go into the latest 2.x release. Upgrade to it to receive them.

## Reporting a vulnerability

Please report vulnerabilities privately. Do not open a public issue,
pull request or discussion for them.

1. Open the repository's
    [Security tab](https://github.com/0xe25f/vistawasm/security) and choose
    **Report a vulnerability**. This uses GitHub's private vulnerability
    reporting, so only you and the maintainers see the report.
2. Describe the problem, the VistaWASM version, the browser, and the
    steps or input that trigger it. A small file or code sample that
    reproduces it helps most.
3. Say whether you want to be credited in the advisory, and under which
    name.

## What happens next

- **Within 3 working days:** we acknowledge the report.
- **Within 10 working days:** we confirm whether it is a vulnerability,
    tell you how severe we think it is, and share a plan for the fix.
- **Within 90 days of the report:** we release a fix, usually much
    sooner for severe problems. If a fix needs longer, we tell you why and
    agree a new date with you.
- **When the fix is released:** we publish a GitHub security advisory
    (with a CVE where one applies), credit you if you wish, and list the
    fix under `### Security` in [`CHANGELOG.md`](CHANGELOG.md).

Please keep the details private until the advisory is published.

## Scope

In scope: the published `@vista-wasm/vista-wasm` package (the JavaScript
wrapper, the WebAssembly engine and its decoders), and the published demo.
The examples and `bench/` are in scope where they show unsafe use of the
library.

Out of scope: vulnerabilities in browsers, GPU drivers or third-party
packages, unless VistaWASM uses them in an unsafe way. Report those to
their own projects.

[`docs/security.md`](docs/security.md) describes the threat model, what
the library validates, the limits it enforces, and the Content Security
Policy a host page needs.

## How the code is checked

Every push and pull request runs Clippy with no warnings allowed (library
code may not `unwrap`, `expect` or `panic!`), the Rust and JavaScript
tests, seeded fuzzers over the decoders and over the terrain and river
pipeline, `cargo audit` and `npm audit`, and a browser smoke test, and
the demo deploys only when they pass. The build refuses a downloaded
`wasm-bindgen` whose SHA-256 does not match the one recorded for its
version.
