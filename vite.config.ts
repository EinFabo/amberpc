import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Two pages: the settings window and the bubble.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: { port: 1430, strictPort: true },
  build: {
    target: "es2022",
    rollupOptions: {
      input: { settings: "settings.html", bubble: "bubble.html" },
    },
  },
});
