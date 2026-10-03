// Bundles the terminal app into one file, dist/frisy.mjs, that runs on Node 18+
// with nothing else installed.
import { build } from "esbuild";

await build({
  entryPoints: ["src/main.jsx"],
  outfile: "dist/frisy.mjs",
  bundle: true,
  platform: "node",
  format: "esm",
  target: "node18",
  jsx: "automatic",
  minify: true,
  legalComments: "none",
  define: { "process.env.NODE_ENV": '"production"', "process.env.DEV": '"false"' },
  // Ink can load React DevTools in development; never in this build.
  alias: { "react-devtools-core": "./src/devtools-stub.js" },
  banner: {
    js: "import { createRequire as __cr } from 'module'; const require = __cr(import.meta.url);",
  },
});
console.log("built dist/frisy.mjs");
