import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    // Bind IPv4 loopback so Playwright embedded (127.0.0.1) can reach the dev server.
    host: "127.0.0.1",
    port: 1420,
    strictPort: true,
    // Dev SUT (:1420) proxies API to `teshi web` during bootstrap dogfooding.
    proxy: {
      "/api/v1": {
        target: "http://127.0.0.1:20253",
        ws: true,
      },
    },
  },
  envPrefix: ["VITE_"],
  build: {
    target: "es2020",
    minify: "esbuild",
    sourcemap: false,
  },
});
