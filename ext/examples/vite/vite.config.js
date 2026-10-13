import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";
import { tscExt } from "./plugin.js";

export default defineConfig({
  plugins: [tscExt(), react()],
  build: { minify: false, sourcemap: true },
  optimizeDeps: {
    include: ["react-dom/client", "tsover-runtime", "@typegpu/react", "typegpu"],
  },
});
