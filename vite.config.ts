import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Vite is configured for Tauri: a fixed dev port and an es2021 target that
// matches the app's minimum supported toolchain.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 5173,
    strictPort: true,
  },
  build: {
    target: "es2021",
    outDir: "dist",
    emptyOutDir: true,
  },
});
