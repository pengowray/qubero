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

See [DESIGN.md](DESIGN.md) for architecture and roadmap.
