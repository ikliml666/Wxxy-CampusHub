import { useEffect, useState, lazy, Suspense } from "react";
import { ArrowLeft, ChevronLeft, ChevronRight, ExternalLink, Newspaper, Rss } from "lucide-react";
import { EmptyState } from "@/components/EmptyState";
import { PanelHeader } from "@/components/PanelHeader";
import { Surface } from "@/components/Surface";
import { Button } from "@/components/ui/button";
import { useAuthStore } from "@/stores/authStore";
import { useBrowserStore } from "@/stores/browserStore";
import { useUiStore } from "@/stores/uiStore";
import type { InfoAttachment, InfoColumn, InfoDetail, InfoItem, InfoPage } from "@/shared/types";
import { invokeCommand } from "@/shared/tauriApi";
import {
  attachmentIcon,
  downloadAndSaveAttachment,
  isPdfAttachment,
} from "@/shared/attachment";
import { cn } from "@/shared/cn";

// PDF 查看器按需加载：pdfjs 体积大，静态导入会拖慢面板首屏
const PdfViewerDialog = lazy(() =>
  import("@/components/PdfViewerDialog").then((m) => ({ default: m.PdfViewerDialog })),
);

const PAGE_SIZE = 10;

/** 栏目 rail 四态（栏目由后端订阅接口 + 实测全量兜底，失败可重试）。 */
type ColumnsState =
  | { phase: "loading" }
  | { phase: "ready"; data: InfoColumn[] }
  | { phase: "error"; message: string };

/** 列表四态。 */
type ListState =
  | { phase: "loading" }
  | { phase: "ready"; data: InfoPage }
  | { phase: "empty" }
  | { phase: "error"; message: string };

/** 正文视图：打开即带条目标题/链接渲染骨架，取回后端清洗 HTML 后渲染。 */
type DetailState =
  | { phase: "loading"; title: string; url: string }
  | { phase: "ready"; title: string; data: InfoDetail; url: string }
  | { phase: "error"; title: string; url: string; message: string };

/** "2026-09-18 03:31:43" → "2026-09-18"（截取日期，不做时区换算）。 */
function dateOf(publishTime: string): string {
  return publishTime.slice(0, 10);
}

/** 正文内链接一律拦截导航（WebView 不随正文跳转外站）。 */
function stopLinkNav(e: React.MouseEvent) {
  if ((e.target as HTMLElement).closest("a")) e.preventDefault();
}

/** 后端清洗 HTML 的最小排版（正文容器样式，域色沿用 token）。 */
const ARTICLE_CLASS = cn(
  "px-4 py-4 text-body leading-relaxed text-text [&_a]:text-info [&_a]:underline [&_a]:decoration-info/40",
  "[&_blockquote]:border-l-2 [&_blockquote]:border-line [&_blockquote]:pl-3 [&_blockquote]:text-text-2",
  "[&_h1]:mt-4 [&_h1]:text-title [&_h2]:mt-4 [&_h2]:text-title [&_h3]:mt-3 [&_h3]:text-body",
  "[&_h4]:mt-3 [&_h4]:text-body [&_h5]:mt-2 [&_h5]:text-body [&_h6]:mt-2 [&_h6]:text-body",
  "[&_img]:mx-auto [&_img]:my-3 [&_img]:h-auto [&_img]:max-w-full [&_img]:rounded",
  "[&_li]:my-1 [&_ol]:my-3 [&_ol]:list-decimal [&_ol]:pl-6 [&_p]:my-3 [&_strong]:font-semibold",
  "[&_table]:my-3 [&_table]:w-full [&_table]:border-collapse [&_table]:text-caption",
  "[&_td]:border [&_td]:border-line [&_td]:px-2 [&_td]:py-1 [&_th]:border [&_th]:border-line",
  "[&_th]:px-2 [&_th]:py-1 [&_ul]:my-3 [&_ul]:list-disc [&_ul]:pl-6",
);

/**
 * 附件行：PDF 打开内嵌查看器弹层，其余下载到本机。
 * 三态完整：loading（按钮 spinner + 禁用）/ done（「已开始下载 fileName」）/
 * error（命令返回的中文文案，再次点击重试）。
 */
function AttachmentItem({
  att,
  onPreview,
}: {
  att: InfoAttachment;
  onPreview: (att: InfoAttachment) => void;
}) {
  const [status, setStatus] = useState<"idle" | "loading" | "done" | "error">("idle");
  const [doneName, setDoneName] = useState("");
  const [errorMsg, setErrorMsg] = useState("");
  const pdf = isPdfAttachment(att.name);
  const Icon = attachmentIcon(att.name);

  const handleClick = () => {
    if (pdf) {
      onPreview(att);
      return;
    }
    if (status === "loading") return;
    setStatus("loading");
    setErrorMsg("");
    downloadAndSaveAttachment(att.url)
      .then((fileName) => {
        setDoneName(fileName);
        setStatus("done");
      })
      .catch((e: unknown) => {
        setErrorMsg(e instanceof Error ? e.message : "附件下载失败");
        setStatus("error");
      });
  };

  return (
    <div>
      <Button
        variant="outline"
        className="w-full justify-start"
        aria-busy={status === "loading"}
        disabled={status === "loading"}
        onClick={handleClick}
      >
        <Icon aria-hidden className="shrink-0 text-text-2" />
        <span className="min-w-0 flex-1 truncate text-left">{att.name}</span>
        <span className="shrink-0 text-caption font-normal text-text-2">
          {pdf ? "预览" : status === "loading" ? "下载中…" : "下载"}
        </span>
      </Button>
      {status === "done" && (
        <p role="status" className="mt-1 px-1 text-caption text-text-2">
          已开始下载 {doneName}
        </p>
      )}
      {status === "error" && (
        <p role="alert" className="mt-1 px-1 text-caption text-alert">
          {errorMsg}
        </p>
      )}
    </div>
  );
}

export function InfoPanel() {
  const status = useAuthStore((s) => s.status);
  const openLoginDialog = useUiStore((s) => s.openLoginDialog);
  const browserOpen = useBrowserStore((s) => s.browserOpen);
  const authed = status === "authed";

  const [columns, setColumns] = useState<ColumnsState>({ phase: "loading" });
  const [activeColumn, setActiveColumn] = useState<string | null>(null);
  const [page, setPage] = useState(1);
  const [list, setList] = useState<ListState>({ phase: "loading" });
  const [detail, setDetail] = useState<DetailState | null>(null);
  const [pdfView, setPdfView] = useState<InfoAttachment | null>(null);
  // 「打开原文」失败的即时反馈（常规失败由 browserStore.errorMsg 走弹层状态条，此处兜 invoke 层异常）
  const [openErr, setOpenErr] = useState<string | null>(null);
  const [reloadTick, setReloadTick] = useState(0);

  // 栏目 rail：登录后取一次（失败可重试）；首个栏目自动选中
  useEffect(() => {
    if (!authed) return;
    let alive = true;
    setColumns({ phase: "loading" });
    setActiveColumn(null);
    invokeCommand<InfoColumn[]>("get_info_columns").then((r) => {
      if (!alive) return;
      if (r.success && r.data && r.data.length > 0) {
        setColumns({ phase: "ready", data: r.data });
        setActiveColumn(r.data[0].id);
      } else {
        setColumns({ phase: "error", message: r.message ?? "资讯栏目获取失败" });
      }
    });
    return () => {
      alive = false;
    };
  }, [authed, reloadTick]);

  // 当前栏目列表：切栏目回第 1 页；「下一页」按 items.length == pageSize 判断
  //（服务端 total/pageCount 实测不可靠，不伪造页码）
  useEffect(() => {
    if (!authed || !activeColumn) return;
    let alive = true;
    setList({ phase: "loading" });
    invokeCommand<InfoPage>("get_info_list", {
      columnId: activeColumn,
      page,
      pageSize: PAGE_SIZE,
    }).then((r) => {
      if (!alive) return;
      if (r.success && r.data) {
        setList(r.data.items.length > 0 ? { phase: "ready", data: r.data } : { phase: "empty" });
      } else {
        setList({ phase: "error", message: r.message ?? "资讯列表获取失败" });
      }
    });
    return () => {
      alive = false;
    };
  }, [authed, activeColumn, page, reloadTick]);

  const fetchDetail = (title: string, url: string) => {
    setOpenErr(null);
    setDetail({ phase: "loading", title, url });
    invokeCommand<InfoDetail>("get_info_detail", { url }).then((r) => {
      setDetail((d) => {
        // 已切走（返回列表 / 打开了另一条）时丢弃过期响应
        if (d === null || d.url !== url) return d;
        if (r.success && r.data) return { phase: "ready", title, data: r.data, url };
        return { phase: "error", title, url, message: r.message ?? "正文获取失败" };
      });
    });
  };

  // needsBrowser 的正文：打开原文（校园域进应用内 webview；域外降级/失败提示由
  // browserStore 内部处理，此处仅兜 invoke 层异常）
  const openInBrowser = (url: string) => {
    setOpenErr(null);
    browserOpen(url).catch(() => setOpenErr("打开浏览器失败"));
  };

  const retry = () => setReloadTick((t) => t + 1);
  const switchColumn = (id: string) => {
    if (id === activeColumn) return;
    setPage(1);
    setActiveColumn(id);
  };

  const columnList = columns.phase === "ready" ? columns.data : [];
  const listData = list.phase === "ready" ? list.data : null;
  const hasNext = listData != null && listData.items.length === PAGE_SIZE;

  return (
    <section className="mx-auto mt-8 max-w-3xl px-4">
      <PanelHeader
        title="资讯"
        description="校园通知、要闻与快讯 · 来自融合门户"
        domain="info"
      />

      {!authed ? (
        <Surface>
          <EmptyState
            icon={Newspaper}
            domain="info"
            title="登录后查看校园资讯"
            hint="登录后展示融合门户的通知公告、校园要闻与校园快讯。"
            action={<Button onClick={openLoginDialog}>登录</Button>}
          />
        </Surface>
      ) : (
        <>
          {/* 栏目 rail：后端固定返回 7 栏（订阅 + 实测全量兜底），顺序由后端排定 */}
          {columns.phase === "error" ? (
            <Surface
              accent="info"
              className="flex items-center justify-between gap-3 py-3 pl-4 pr-2"
            >
              <p className="min-w-0 truncate text-body text-text-2">
                资讯栏目获取失败：{columns.message}
              </p>
              <Button variant="outline" size="sm" className="shrink-0" onClick={retry}>
                重试
              </Button>
            </Surface>
          ) : columns.phase === "loading" ? (
            <div aria-hidden className="flex flex-wrap gap-2">
              {[0, 1, 2, 3, 4].map((i) => (
                <span
                  key={i}
                  className="h-9 w-16 animate-pulse rounded-control border border-line bg-line"
                />
              ))}
            </div>
          ) : (
            <div role="tablist" aria-label="资讯分类" className="flex flex-wrap gap-2">
              {columnList.map((c) => (
                <button
                  key={c.id}
                  type="button"
                  role="tab"
                  aria-selected={c.id === activeColumn}
                  onClick={() => switchColumn(c.id)}
                  className={cn(
                    "rounded-control border px-3 py-1.5 text-caption transition-colors duration-[var(--dur-fast)] ease-out-soft",
                    c.id === activeColumn
                      ? "border-info/30 bg-info/10 font-medium text-info"
                      : "border-line bg-surface text-text-2 hover:border-line-strong hover:text-text",
                  )}
                >
                  {c.name}
                </button>
              ))}
            </div>
          )}

          {/* 内嵌正文视图：覆盖列表；返回后列表状态保留 */}
          {detail ? (
            <Surface className="mt-4">
              <div className="flex items-center gap-2 border-b border-line px-2 py-2">
                <Button variant="ghost" size="sm" onClick={() => setDetail(null)}>
                  <ArrowLeft aria-hidden="true" />
                  返回列表
                </Button>
                <span className="min-w-0 truncate text-body font-medium text-text">
                  {detail.title}
                </span>
              </div>
              {detail.phase === "loading" ? (
                <div aria-hidden className="space-y-3 px-4 py-5">
                  {[80, 100, 60, 90, 40].map((w, i) => (
                    <div
                      key={i}
                      className="h-4 animate-pulse rounded bg-line"
                      style={{ width: `${w}%` }}
                    />
                  ))}
                </div>
              ) : detail.phase === "error" ? (
                <EmptyState
                  compact
                  icon={Rss}
                  domain="info"
                  title="正文获取失败"
                  hint={detail.message}
                  action={
                    <Button
                      variant="outline"
                      size="sm"
                      onClick={() => fetchDetail(detail.title, detail.url)}
                    >
                      重试
                    </Button>
                  }
                />
              ) : detail.data.needsBrowser ? (
                // 正文受官网鉴权保护（auth 开门页/非 2xx）：明确说明 + 浏览器打开，
                // 不走错误态/重试（重试无效，站点侧拦截与网络无关）
                <EmptyState
                  compact
                  icon={ExternalLink}
                  domain="info"
                  title="正文需在浏览器中查看"
                  hint="该栏目正文由学校官网鉴权保护，无法在应用内展示。"
                  action={
                    <>
                      <Button
                        variant="outline"
                        size="sm"
                        onClick={() => openInBrowser(detail.data.url)}
                      >
                        <ExternalLink aria-hidden="true" />
                        打开原文
                      </Button>
                      {openErr && <p className="text-caption text-text-2">{openErr}</p>}
                    </>
                  }
                />
              ) : (
                <div className={ARTICLE_CLASS}>
                  <h2 className="mb-4 text-display-s font-semibold text-text">
                    {detail.data.title}
                  </h2>
                  {/* 附件区：空列表不渲染区块；PDF 进查看器弹层，其余本机下载 */}
                  {(detail.data.attachments ?? []).length > 0 && (
                    <section aria-label="附件" className="mt-2 mb-4">
                      <div className="mb-2 text-caption font-medium text-text-2">
                        附件 · {detail.data.attachments.length}
                      </div>
                      <ul className="space-y-2">
                        {detail.data.attachments.map((att, i) => (
                          <AttachmentItem key={`${att.url}-${i}`} att={att} onPreview={setPdfView} />
                        ))}
                      </ul>
                    </section>
                  )}
                  {/* 后端已按标签/属性白名单清洗（剔除 script/事件属性、相对地址转
                      绝对），前端不二次清洗；正文内 <a> 导航统一拦截 */}
                  <article
                    onClick={stopLinkNav}
                    dangerouslySetInnerHTML={{ __html: detail.data.html ?? "" }}
                  />
                </div>
              )}
            </Surface>
          ) : (
            <>
              {/* 列表：加载骨架 */}
              {list.phase === "loading" && (
                <div aria-hidden className="mt-4 space-y-2">
                  {[0, 1, 2, 3].map((i) => (
                    <Surface key={i} className="flex items-center gap-4 px-4 py-3">
                      <span className="h-4 w-20 shrink-0 animate-pulse rounded bg-line" />
                      <span className="h-4 min-w-0 flex-1 animate-pulse rounded bg-line" />
                      <span className="h-4 w-20 shrink-0 animate-pulse rounded bg-line" />
                    </Surface>
                  ))}
                </div>
              )}
              {/* 列表：出错可重试 */}
              {list.phase === "error" && (
                <Surface className="mt-4">
                  <EmptyState
                    icon={Rss}
                    domain="info"
                    title="资讯列表获取失败"
                    hint={list.message}
                    action={
                      <Button variant="outline" onClick={retry}>
                        重试
                      </Button>
                    }
                  />
                </Surface>
              )}
              {/* 列表：空态 */}
              {list.phase === "empty" && (
                <Surface className="mt-4">
                  <EmptyState
                    icon={Rss}
                    domain="info"
                    title="该栏目暂无资讯"
                    hint="当前栏目下没有可展示的内容。"
                  />
                </Surface>
              )}
              {/* 列表：有数据 */}
              {list.phase === "ready" && (
                <>
                  <div className="mt-4 space-y-2">
                    {listData?.items.map((item: InfoItem) => (
                      <button
                        key={item.id}
                        type="button"
                        onClick={() => fetchDetail(item.title, item.url)}
                        className="block w-full rounded-[var(--radius)] text-left"
                      >
                        <Surface hover className="flex items-center gap-3 px-4 py-3">
                          <span className="shrink-0 text-caption text-info">
                            {item.columnTitle}
                          </span>
                          <span className="min-w-0 flex-1 truncate text-body text-text">
                            {item.title}
                          </span>
                          {item.dept && (
                            <span className="hidden shrink-0 text-caption text-text-2 sm:inline">
                              {item.dept}
                            </span>
                          )}
                          <span className="tabular-num shrink-0 text-caption text-text-2">
                            {dateOf(item.publishTime)}
                          </span>
                        </Surface>
                      </button>
                    ))}
                  </div>
                  {/* 分页：下一页按满页判断（服务端 pageCount 不可靠） */}
                  <div className="mt-4 flex items-center justify-center gap-3">
                    <Button
                      variant="outline"
                      size="sm"
                      disabled={page <= 1}
                      onClick={() => setPage((p) => Math.max(1, p - 1))}
                    >
                      <ChevronLeft aria-hidden="true" />
                      上一页
                    </Button>
                    <span className="tabular-num text-caption text-text-2">第 {page} 页</span>
                    <Button
                      variant="outline"
                      size="sm"
                      disabled={!hasNext}
                      onClick={() => setPage((p) => p + 1)}
                    >
                      下一页
                      <ChevronRight aria-hidden="true" />
                    </Button>
                  </div>
                </>
              )}
            </>
          )}
        </>
      )}

      {pdfView && (
        <Suspense fallback={null}>
          <PdfViewerDialog
            name={pdfView.name}
            url={pdfView.url}
            onClose={() => setPdfView(null)}
          />
        </Suspense>
      )}
    </section>
  );
}
