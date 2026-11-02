import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

export default defineConfig({
  plugins: [react(), tailwindcss()],
  clearScreen: false,
  server: { watch: { ignored: ["**/src-tauri/**"] } },
  test: { environment: "jsdom", restoreMocks: true },
});
