import { defineConfig } from "vite";

export default defineConfig({
  // PORT lets a second dev server (another session, another branch) get its own port.
  server: { port: Number(process.env["PORT"]) || 5173 },
  build: { target: "es2022" },
});
