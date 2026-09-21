import { fileURLToPath, URL } from "node:url";
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import { viteStaticCopy } from "vite-plugin-static-copy";

export default defineConfig({
  plugins: [
    react(),
    tailwindcss(),
    // pdfjs CMap（中日韩字形）拷进产物：PdfViewerDialog 的 Document options.cMapUrl 取相对 "cmaps/"
    //（rename.stripBase 扁平拷贝——CMap 查找按平铺目录名进行）
    viteStaticCopy({
      targets: [
        {
          src: "node_modules/pdfjs-dist/cmaps/*.bcmap",
          dest: "cmaps",
          rename: { stripBase: true },
        },
      ],
    }),
  ],
  clearScreen: false,
  server: { port: 1420, strictPort: true },
  resolve: {
    alias: {
      "@": fileURLToPath(new URL("./src", import.meta.url)),
    },
  },
});
