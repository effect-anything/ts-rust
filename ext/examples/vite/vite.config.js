import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";
import { extSyntax } from "./plugin.js";

export default defineConfig({
  plugins: [extSyntax(), react()],
  build: { minify: false, sourcemap: true },
  optimizeDeps: {
    include: ["react-dom/client", "tsover-runtime", "@typegpu/react", "typegpu"],
  },
});
