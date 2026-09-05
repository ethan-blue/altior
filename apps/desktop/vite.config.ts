import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// The fixture shell dev/test server serves the in-memory IPC transport;
// real Tauri wiring arrives with the P0.4 desktop spike (ADR 0005).
export default defineConfig({
  plugins: [react()],
  server: {
    // Fixtures are owned by the Rust protocol crate and imported directly
    // so there is exactly one durable fixture source (ADR 0005).
    fs: {
      allow: ["../.."],
    },
    // Never watch the Tauri shell build tree: target/debug contains locked
    // executables while `tauri dev` compiles, which crashes chokidar (EBUSY).
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
  test: {
    environment: "jsdom",
    setupFiles: ["./src/test/setup.ts"],
  },
});
