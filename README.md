# Qubero

Web hex editor for files of any size. Rust core (wasm) + TypeScript UI.

## Build and run

```
rustup target add wasm32-unknown-unknown
cargo install wasm-pack
cd web && npm install && npm run wasm && npm run dev
```

Open http://localhost:5173 and drop a file, or `?synthetic=5G` for a fake 5 GiB file.

Tests: `cargo test -p qubero-core`. Typecheck: `cd web && npx tsc --noEmit`.
`npm run wasm` builds two modules: the editor, and the file(1) rule database
used to identify formats with no template, which the page fetches only when it
meets one.

## Licences

Qubero is MIT ([LICENSE](LICENSE)). The crates and the rule database it ships
carry their own terms, listed in [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md);
regenerate that file with `node tools/notices.mjs` after changing dependencies.

See [DESIGN.md](DESIGN.md) for architecture and roadmap.
