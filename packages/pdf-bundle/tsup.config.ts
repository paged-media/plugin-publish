import { defineConfig } from "tsup";
export default defineConfig({
  entry: ["src/index.ts"],
  format: ["esm"],
  dts: true,
  clean: true,
  // Leave `?url` asset imports (the pdf-import `_bg.wasm`) as runtime imports
  // so the editor's Vite resolves + serves them relative to the served dist —
  // esbuild can't load a `.wasm?url` at build time. Mirrors web-bundle.
  external: [/\?url$/],
});
