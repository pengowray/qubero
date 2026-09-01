import { createReadStream, statSync } from "node:fs";
import { join, normalize, resolve, sep } from "node:path";
import { defineConfig } from "vite";
import type { Plugin } from "vite";

/**
 * Serve files from a directory outside the project, for testing against real
 * files too big or too private to check in.
 *
 * `QUBERO_FILES=D:/koboldcpp npm run dev` puts that directory under
 * `/local/`, so `/local/bge-m3-q8_0.gguf` fetches the model. Ranges are
 * honoured, which is what makes a range-fetching source over one of these
 * worth having: a five-gigabyte file is opened without reading it.
 *
 * Vite's own `server.fs.allow` plus `/@fs/` does not do this — it hands back
 * `index.html` for an arbitrary binary — and this is dev-only either way: the
 * plugin does nothing when the variable is unset, and nothing in a build.
 */
function localFiles(): Plugin {
  const root = process.env["QUBERO_FILES"];
  return {
    name: "qubero-local-files",
    apply: "serve",
    configureServer(server) {
      if (root === undefined || root === "") return;
      const base = resolve(root);
      server.middlewares.use("/local", (req, res, next) => {
        const rel = decodeURIComponent((req.url ?? "/").split("?")[0] ?? "/").replace(/^\/+/, "");
        const path = normalize(join(base, rel));
        // Nothing above the configured directory, whatever the URL says.
        if (path !== base && !path.startsWith(base + sep)) {
          res.statusCode = 403;
          res.end();
          return;
        }
        let size = 0;
        try {
          size = statSync(path).size;
        } catch {
          next();
          return;
        }
        res.setHeader("content-type", "application/octet-stream");
        res.setHeader("accept-ranges", "bytes");
        const range = /^bytes=(\d*)-(\d*)$/.exec(req.headers.range ?? "");
        if (range === null) {
          res.setHeader("content-length", String(size));
          createReadStream(path).pipe(res);
          return;
        }
        const start = range[1] === "" ? Math.max(0, size - Number(range[2])) : Number(range[1]);
        const end = range[1] === "" || range[2] === "" ? size - 1 : Math.min(size - 1, Number(range[2]));
        res.statusCode = 206;
        res.setHeader("content-range", `bytes ${start}-${end}/${size}`);
        res.setHeader("content-length", String(end - start + 1));
        createReadStream(path, { start, end }).pipe(res);
      });
    },
  };
}

export default defineConfig({
  // PORT lets a second dev server (another session, another branch) get its own port.
  server: { port: Number(process.env["PORT"]) || 17272 },
  build: { target: "es2022" },
  plugins: [localFiles()],
});
