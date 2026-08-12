import tailwindcss from "@tailwindcss/vite";
import { defineConfig } from "astro/config";

export default defineConfig({
  site: "https://scorch.fractal-tess.xyz",
  vite: {
    plugins: [tailwindcss()],
  },
});
