---
title: WebView2 内嵌 PDF 的 CSP 与 CMap 三坑
type: learning
source_files:
  - tauri-app/src-tauri/tauri.conf.json
  - tauri-app/frontend/src/components/PdfViewerDialog.tsx
  - tauri-app/frontend/vite.config.ts
tags:
  - pdf
  - csp
  - webview2
  - vite
---

# WebView2 内嵌 PDF 的 CSP 与 CMap 三坑

集成 pdf.js（react-pdf 11）进 Tauri 2 WebView2 时实测的三个必要项，缺一就是白屏或乱码：

1. **CSP 三件套**（tauri.conf.json）：`script-src` 必须加 `'wasm-unsafe-eval'`——pdf.js v4 起扫描件 JPEG2000（JPXDecode）走 WASM 解码，不放行整页空白（mozilla/pdf.js#18457）；新增 `worker-src 'self' blob:`——worker 主路径是同源 .mjs 静态资源，`blob:` 兜底 fake-worker 分支；`img-src` 加 `blob:`——pdf.js 渲染位图走 blob URL。`connect-src` **不需要**为 PDF 放行任何外域（下载走 Rust 命令，worker 用 `new URL('pdfjs-dist/build/pdf.worker.min.mjs', import.meta.url)` 产物化，同源加载）。⚠️ tauri.conf.json 是严格 JSON 且 security 段 `deny_unknown_fields`，JSON 注释与未知键都会让 tauri-build 编译失败——放行理由只能记在代码注释里。
2. **workerSrc 必须在使用 react-pdf 组件的同一模块设置**——README 明确警告，放别处会被默认值覆盖（默认值指向 CDN，Tauri 下必挂，tauri-apps/tauri#12610）。
3. **中文 PDF 必须 CMap**：GBK/UniGB 编码的校园红头文件 PDF 无 CMap 会渲染空白/乱码。Vite 下用 `vite-plugin-static-copy` 把 `pdfjs-dist/cmaps/*.bcmap` 拷进产物，`<Document options={{ cMapUrl: BASE_URL + 'cmaps/', cMapPacked: true }}>`。**坑**：vite-plugin-static-copy v4 已废弃旧选项 `structured: false`，等价写法是 `rename: { stripBase: true }`（168 个 bcmap 扁平拷入 dist/cmaps）；CMap 查找按平铺目录名进行，嵌套目录会 404。
