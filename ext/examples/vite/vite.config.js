import react from "@vitejs/plugin-react";
import tsc from "@effect-x/tsc/unplugin/vite";
import { defineConfig } from "vite";

export default defineConfig({
  // Options come from tsconfig.json (contentMappers "@effect-x/tsc" options).
  plugins: [tsc(), react()],
  build: { minify: false, sourcemap: true },
  optimizeDeps: {
    include: ["react-dom/client", "tsover-runtime", "@typegpu/react", "typegpu"],
  },
});
