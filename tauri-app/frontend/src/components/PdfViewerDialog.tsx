import { useCallback, useEffect, useRef, useState } from "react";
import { motion, useReducedMotion } from "framer-motion";
import {
  Download,
  FileText,
  Minus,
  Plus,
  RefreshCw,
  X,
} from "lucide-react";
import { Document, Page, pdfjs } from "react-pdf";
import { Button } from "@/components/ui/button";
import { downloadAttachment } from "@/shared/tauriApi";
import { base64ToBytes, downloadAndSaveAttachment } from "@/shared/attachment";
import { cn } from "@/shared/cn";

// pdfjs worker：Vite 会把 new URL(..., import.meta.url) 产物化为静态资源（同源，CSP worker-src 'self' 覆盖）
pdfjs.GlobalWorkerOptions.workerSrc = new URL(
  "pdfjs-dist/build/pdf.worker.min.mjs",
  import.meta.url,
).toString();

/** 缩放范围与步进。 */
const SCALE_MIN = 0.6;
const SCALE_MAX = 2.0;
const SCALE_STEP = 0.2;

/** 打开弹层时的文档数据三态。 */
type DocPhase =
  | { phase: "loading" }
  | { phase: "ready"; data: Uint8Array }
  | { phase: "error"; message: string };

/**
 * 内嵌 PDF 查看器（资讯详情附件区触发）。数据流：download_attachment → 裸 base64
 * → Uint8Array → `<Document file={{ data }}>`，全部页顺序渲染（校园附件普遍 <50 页，不虚拟化）。
 * CJK 字形：依赖 vite-plugin-static-copy 把 pdfjs-dist/cmaps 拷进产物，
 * options.cMapUrl 取相对路径（Vite base 保证同源）。
 */
export function PdfViewerDialog({
  name,
  url,
  onClose,
}: {
  name: string;
  url: string;
  onClose: () => void;
}) {
  const [doc, setDoc] = useState<DocPhase>({ phase: "loading" });
  const [numPages, setNumPages] = useState(0);
  const [currentPage, setCurrentPage] = useState(1);
  const [scale, setScale] = useState(1);
  const [viewportWidth, setViewportWidth] = useState(0);
  const [downloading, setDownloading] = useState(false);
  const [downloadMsg, setDownloadMsg] = useState<{ kind: "ok" | "err"; text: string } | null>(null);

  const scrollRef = useRef<HTMLDivElement>(null);
  const closeRef = useRef<HTMLButtonElement>(null);
  const restoreFocusRef = useRef<Element | null>(null);
  const onCloseRef = useRef(onClose);
  onCloseRef.current = onClose;
  const reduceMotion = useReducedMotion();

  const load = useCallback(() => {
    setDoc({ phase: "loading" });
    setNumPages(0);
    setCurrentPage(1);
    downloadAttachment(url).then((r) => {
      if (!r.success || !r.data) {
        setDoc({ phase: "error", message: r.message ?? "附件下载失败" });
        return;
      }
      setDoc({ phase: "ready", data: base64ToBytes(r.data.base64) });
    });
  }, [url]);

  useEffect(() => {
    load();
  }, [load]);

  // 打开：焦点入弹层（关闭按钮）；Esc 关闭；锁背景滚动；卸载还焦点给触发元素
  useEffect(() => {
    restoreFocusRef.current = document.activeElement;
    const t = window.setTimeout(() => closeRef.current?.focus(), 0);
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onCloseRef.current();
    };
    window.addEventListener("keydown", onKey);
    const prevOverflow = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    return () => {
      window.clearTimeout(t);
      window.removeEventListener("keydown", onKey);
      document.body.style.overflow = prevOverflow;
      const el = restoreFocusRef.current;
      if (el instanceof HTMLElement) el.focus();
    };
  }, []);

  // 视口宽测量（缩放页宽 = 视口宽 × scale）
  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    const update = () => setViewportWidth(el.clientWidth);
    update();
    const ro = new ResizeObserver(update);
    ro.observe(el);
    return () => ro.disconnect();
  }, [doc.phase]);

  // 滚动定位当前页（页容器相对定位，offsetTop 即页顶偏移）
  const handleScroll = () => {
    const el = scrollRef.current;
    if (!el) return;
    const pages = el.querySelectorAll<HTMLElement>("[data-pdf-page]");
    const top = el.scrollTop;
    let cur = 1;
    pages.forEach((p, i) => {
      if (p.offsetTop <= top + 48) cur = i + 1;
    });
    setCurrentPage(cur);
  };

  const stepScale = (dir: -1 | 1) =>
    setScale((s) => Math.min(SCALE_MAX, Math.max(SCALE_MIN, +(s + dir * SCALE_STEP).toFixed(2))));

  const handleDownload = () => {
    if (downloading) return;
    setDownloading(true);
    setDownloadMsg(null);
    downloadAndSaveAttachment(url)
      .then((fileName) => setDownloadMsg({ kind: "ok", text: `已开始下载 ${fileName}` }))
      .catch((e: unknown) =>
        setDownloadMsg({ kind: "err", text: e instanceof Error ? e.message : "附件下载失败" }),
      )
      .finally(() => setDownloading(false));
  };

  const pageWidth = Math.max(240, Math.floor(viewportWidth * scale) - 2);

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 px-[3vw]"
      onMouseDown={(e) => {
        // 只在点遮罩空白处关闭（点内容区不关）
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <motion.div
        role="dialog"
        aria-modal="true"
        aria-label={`PDF 预览：${name}`}
        initial={reduceMotion ? { opacity: 0 } : { opacity: 0, y: 8, scale: 0.99 }}
        animate={{ opacity: 1, y: 0, scale: 1 }}
        transition={
          reduceMotion ? { duration: 0.12 } : { duration: 0.18, ease: [0.22, 1, 0.36, 1] }
        }
        className="flex h-[90vh] w-[min(1100px,94vw)] flex-col overflow-hidden rounded-card border border-line bg-surface shadow-pop"
      >
        {/* 工具条：文件名 + 页码 + 缩放 + 下载 + 关闭 */}
        <div className="flex items-center gap-2 border-b border-line px-3 py-2">
          <FileText aria-hidden className="size-4 shrink-0 text-info" />
          <span className="min-w-0 flex-1 truncate text-body font-medium text-text" title={name}>
            {name}
          </span>
          <span className="tabular-num shrink-0 text-caption text-text-2" aria-live="polite">
            {numPages > 0 ? `${currentPage} / ${numPages}` : "—"}
          </span>
          <div className="flex shrink-0 items-center" role="group" aria-label="缩放">
            <Button
              variant="ghost"
              size="icon-sm"
              aria-label="缩小"
              disabled={scale <= SCALE_MIN}
              onClick={() => stepScale(-1)}
            >
              <Minus aria-hidden className="size-4" />
            </Button>
            <span className="tabular-num w-10 text-center text-caption text-text-2">
              {Math.round(scale * 100)}%
            </span>
            <Button
              variant="ghost"
              size="icon-sm"
              aria-label="放大"
              disabled={scale >= SCALE_MAX}
              onClick={() => stepScale(1)}
            >
              <Plus aria-hidden className="size-4" />
            </Button>
          </div>
          <Button
            variant="outline"
            size="icon-sm"
            aria-label={`下载 ${name}`}
            aria-busy={downloading}
            disabled={downloading}
            onClick={handleDownload}
          >
            <Download aria-hidden className="size-4" />
          </Button>
          <button
            ref={closeRef}
            type="button"
            aria-label="关闭 PDF 预览"
            onClick={onClose}
            className="flex size-8 shrink-0 items-center justify-center rounded-md text-text-2 transition-colors duration-[var(--dur-fast)] hover:bg-accent hover:text-text"
          >
            <X className="size-4" aria-hidden="true" />
          </button>
        </div>
        {downloadMsg && (
          <p
            role={downloadMsg.kind === "err" ? "alert" : "status"}
            className={cn(
              "border-b border-line px-3 py-1.5 text-caption",
              downloadMsg.kind === "ok" ? "text-text-2" : "text-alert",
            )}
          >
            {downloadMsg.text}
          </p>
        )}

        {/* 内容区：全页列表纵向滚动 */}
        <div
          ref={scrollRef}
          onScroll={handleScroll}
          className="min-h-0 flex-1 overflow-y-auto bg-surface-2 px-4 py-4"
        >
          {doc.phase === "loading" ? (
            <div aria-hidden className="mx-auto max-w-2xl space-y-3">
              <div className="h-6 w-40 animate-pulse rounded bg-line" />
              {[0, 1, 2].map((i) => (
                <div key={i} className="h-72 animate-pulse rounded-inner bg-line" />
              ))}
            </div>
          ) : doc.phase === "error" ? (
            <div className="flex h-full flex-col items-center justify-center gap-3 px-6 text-center">
              <span
                aria-hidden
                className="flex size-14 shrink-0 items-center justify-center rounded-full"
                style={{ backgroundColor: "color-mix(in srgb, var(--color-alert) 12%, transparent)" }}
              >
                <FileText className="size-6" style={{ color: "var(--color-alert)" }} />
              </span>
              <p className="text-title font-semibold text-text">PDF 加载失败</p>
              <p className="max-w-[48ch] text-caption text-text-2">{doc.message}</p>
              <div className="mt-1 flex items-center gap-2">
                <Button variant="outline" size="sm" onClick={load}>
                  <RefreshCw aria-hidden className="size-3.5" />
                  重试
                </Button>
                <Button variant="outline" size="sm" onClick={onClose}>
                  关闭
                </Button>
              </div>
            </div>
          ) : (
            <div className="relative mx-auto w-fit">
              <Document
                file={{ data: doc.data }}
                options={{
                  cMapUrl: `${import.meta.env.BASE_URL}cmaps/`,
                  cMapPacked: true,
                }}
                onLoadSuccess={(pdf) => setNumPages(pdf.numPages)}
                onLoadError={(err) =>
                  setDoc({ phase: "error", message: String(err?.message ?? err) })
                }
                loading={null}
              >
                {Array.from({ length: numPages }, (_, i) => (
                  <div
                    key={i}
                    data-pdf-page
                    className="my-3 first:mt-0 last:mb-0"
                  >
                    <Page
                      pageNumber={i + 1}
                      width={pageWidth}
                      loading={
                        <div
                          aria-hidden
                          className="h-72 animate-pulse rounded-inner bg-line"
                          style={{ width: pageWidth }}
                        />
                      }
                    />
                  </div>
                ))}
              </Document>
            </div>
          )}
        </div>
      </motion.div>
    </div>
  );
}
