import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  base: "./",
  plugins: [react()],
  root: "ui",
  build: {
    outDir: "../dist/ui",
    emptyOutDir: true
  },
  server: {
    port: 5173
  }
});
