import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import { resolve } from "node:path";
import process from "node:process";

const host = process.env.TAURI_DEV_HOST;
const root = import.meta.dirname;

// https://vite.dev/config/
export default defineConfig(() => ({
  plugins: [react(), tailwindcss()],

  build: {
    target: "es2022",
    rollupOptions: {
      input: {
        main: resolve(root, "index.html"),
        overlay: resolve(root, "overlay.html"),
      },
    },
  },

  // Options Tauri : ne pas masquer les erreurs Rust, port fixe.
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: "ws", host, port: 1421 } : undefined,
    watch: { ignored: ["**/src-tauri/**", "**/server/**"] },
  },
}));
