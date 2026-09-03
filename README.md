# Qubero

Web hex editor for files of any size. Rust core (wasm) + TypeScript UI.

## Build and run

Node and the Rust wasm toolchain are the prerequisites:

```
rustup target add wasm32-unknown-unknown
cargo install wasm-pack
```

Then `run.bat` on Windows, `./run.sh` elsewhere. It installs and builds
whatever is missing before starting the dev server, so a fresh clone and an
everyday run are the same command.

Open http://localhost:17272 and drop a file, or `?synthetic=5G` for a fake
5 GiB file.

`build.bat` / `./build.sh` build the site into `web/dist` instead, the same
steps the deploy workflow runs. Serve that with `npm run preview` in `web`.

Tests: `cargo test -p qubero-core`. Typecheck: `cd web && npx tsc --noEmit`.
`npm run wasm` builds two modules: the editor, and the file(1) rule database
used to identify formats with no template, which the page fetches only when it
meets one.

## Licences

Qubero is MIT ([LICENSE](LICENSE)). The crates and the rule database it ships
carry their own terms, listed in [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md);
regenerate that file with `node tools/notices.mjs` after changing dependencies.

See [DESIGN.md](DESIGN.md) for architecture and roadmap.
