---
title: 附件内嵌查看器与鉴权下载管线
type: decision
source_files:
  - tauri-app/frontend/src/components/PdfViewerDialog.tsx
  - tauri-app/frontend/src/shared/attachment.ts
  - tauri-app/frontend/src/panels/InfoPanel.tsx
  - tauri-app/src-tauri/src/commands/portal.rs
  - crates/campus-portal/src/article.rs
  - tauri-app/src-tauri/tauri.conf.json
tags:
  - pdf
  - attachment
  - csp
  - react-pdf
  - webview2
---

# 附件内嵌查看器与鉴权下载管线

**背景**：门户资讯/通知正文常附文件（多为 PDF），用户要求应用内直接预览而不是跳系统浏览器。三个硬约束决定了方案形态：

1. **WebView2 没有 Chrome 那样的内置 PDF 查看器**——`<embed>/<iframe>` 加载 PDF 在 WebView2 中不可用，必须 JS 渲染方案（pdf.js 系）。
2. **门户会话 cookie 在 Rust 侧 RecordingJar**，前端 WebView 的 fetch 不会自动携带——「前端直接 fetch 附件 URL」拿不到鉴权文件。
3. **CSP 冻结在 `connect-src 'self' ipc://localhost`**（tauri.conf.json），前端不能直连外域下载。

**选型**：`react-pdf`（wojtekmaj）11.0.0，MIT，peer 明确支持 React 19，锁 pdfjs-dist 6.3.289，活跃维护；底层即 mozilla pdf.js。排除：@react-pdf-viewer/core（2026-03 归档 + 商业授权）、@cyntler/react-doc-viewer（停止维护）、pdfobject（仅封装 `<embed>`，前提在 WebView2 不成立）。备选 pdfjs-dist 裸用（Apache-2.0），但要多写分页/缩放，无必要。

**数据流（单一管线，base64 单路径）**：

```
正文清洗时抽取附件（campus-portal article.rs extract_attachments）
  → InfoDetail.attachments[{name,url}]（相对 URL 已补全、去重保序）
  → 前端附件区（InfoPanel AttachmentItem）
  → download_attachment 命令（commands/portal.rs）：白名单校验（*.cwxu.edu.cn + 10.3.100.110，
    is_allowed_attachment_url，复用 is_allowed_info_url 风格防 SSRF）→ session_client GET
    （RecordingJar 自动带门户 cookie）→ 双重 15MB 限流（content-length 预检 + 流式累计兜底）
    → { fileName, base64 }
  → PDF：atob→Uint8Array → <Document file={{ data }}>；非 PDF：Blob + a[download] 触发系统保存
```

**取舍**：
- 大文件不走 Tauri asset protocol（避免 assetProtocol scope 配置与 `asset: http://asset.localhost` 的 CSP 额外放行），统一 base64 单路径；15MB 以上直接报「附件过大，请从原文页下载」降级。校园附件普遍几 MB 内，值得为极少数大文件引入第二条管线的复杂度时再说。
- 全页顺序渲染不做虚拟化（<50 页口径）；页码用 scroll spy（data-pdf-page offsetTop），缩放 0.6–2.0 步进 0.2。
- PdfViewerDialog 用 `React.lazy` 按需加载——pdfjs 打包后约 704KB，静态导入会拖慢资讯面板首屏；worker 也是 `new URL(..., import.meta.url)` 产物化同源加载，CSP `worker-src 'self'` 覆盖。
- 非 PDF（doc/xls/zip 等）不做应用内预览，走同一条 download_attachment 管线下载到本机。

**日程页课程去重的口径**：bs-schedule 分类含「课程」，与课表页（TimetablePanel）重复。过滤放前端（`isCourseEvent`：classifyName === "课程" 或 classifyCode 含 "course"，双判据容错），月视图角标改由过滤后的 `get_schedule_month` 月明细前端自算、不再调 `get_schedule_day_counts`（月明细本就全量返回，同源保证角标与列表口径一致）；后端命令保留不删。
