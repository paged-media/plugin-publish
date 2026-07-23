import { defineConfig } from "tsup";
export default defineConfig({
  entry: ["src/index.ts"],
  format: ["esm"],
  dts: true,
  clean: true,
  // Leave `?url` asset imports (the pdf-import `_bg.wasm` + the pdf.js
  // worker) as runtime imports so the editor's Vite resolves + serves them
  // relative to the served dist — esbuild can't load a `?url` at build time.
  // Mirrors web-bundle.
  external: [/\?url$/],
  // Inline jszip INTO the dist. The bundle ships `?url` assets, so the editor
  // excludes it from Vite's dep pre-bundle — which means jszip would be served
  // as raw CJS (jszip.min.js) with no ESM default-export interop, crashing the
  // load. Bundling it here makes the package self-contained (no reliance on the
  // consumer pre-bundling jszip, which pnpm nesting makes unresolvable anyway).
  noExternal: ["jszip"],
});
