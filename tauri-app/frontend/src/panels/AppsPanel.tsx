import { useEffect, useState } from "react";
import { LayoutGrid, Puzzle } from "lucide-react";
import { EmptyState } from "@/components/EmptyState";
import { PanelHeader } from "@/components/PanelHeader";
import { Surface } from "@/components/Surface";
import { Button } from "@/components/ui/button";
import { useAuthStore } from "@/stores/authStore";
import { useUiStore } from "@/stores/uiStore";
import type { AppCatalog, AppItem } from "@/shared/types";
import { invokeCommand } from "@/shared/tauriApi";
import { cn } from "@/shared/cn";

/** 目录四态（图标已由后端代拉为 data URL，失败的条目 iconUrl 为 null）。 */
type CatalogState =
  | { phase: "loading" }
  | { phase: "ready"; data: AppCatalog }
  | { phase: "empty" }
  | { phase: "error"; message: string };

/** 应用图标：有 data URL 用 img，否则占位图标（不伪造图片）。 */
function AppIcon({ item }: { item: AppItem }) {
  if (item.iconUrl) {
    return (
      <img
        src={item.iconUrl}
        alt=""
        className="size-9 shrink-0 rounded-inner bg-surface object-contain"
      />
    );
  }
  return (
    <span
      aria-hidden
      className="flex size-9 shrink-0 items-center justify-center rounded-inner bg-sched/10 text-sched"
    >
      <Puzzle className="size-4" />
    </span>
  );
}

/** 可达性徽标：webvpn / unavailable 给出可见徽标，cas / external 不标注（直开）。 */
function accessBadge(access: AppItem["access"]): { label: string; muted: boolean } | null {
  if (access === "webvpn") return { label: "需校园网/WebVPN", muted: false };
  if (access === "unavailable") return { label: "暂不可用", muted: true };
  return null;
}

/** 单个应用卡：点击经后端 open_app 在系统浏览器打开（协议白名单在后端强制）；
 * 可达性徽标按后端附录 A 实测表推导。 */
function AppCard({
  item,
  onOpen,
}: {
  item: AppItem;
  onOpen: (item: AppItem) => void;
}) {
  const badge = accessBadge(item.access);
  return (
    <button
      type="button"
      onClick={() => onOpen(item)}
      className="block w-full rounded-[var(--radius)] text-left"
    >
      <Surface hover className="flex items-center gap-3 px-3.5 py-3">
        <AppIcon item={item} />
        <div className="min-w-0">
          <p className="truncate text-body font-medium text-text">{item.name}</p>
          {badge && (
            <span
              className={cn(
                "mt-0.5 inline-block rounded-control border px-1.5 text-caption leading-4",
                badge.muted
                  ? "border-line bg-line/40 text-text-2"
                  : "border-sched/30 bg-sched/10 text-sched",
              )}
            >
              {badge.label}
            </span>
          )}
        </div>
      </Surface>
    </button>
  );
}

/** 分组网格骨架（加载态）。 */
function GroupSkeleton() {
  return (
    <>
      {["常用", "示例分组"].map((g) => (
        <div key={g} className="mt-5" aria-hidden>
          <span className="block h-4 w-16 animate-pulse rounded bg-line" />
          <div className="mt-2 grid grid-cols-3 gap-3">
            {[1, 2, 3].map((i) => (
              <Surface key={i} className="flex items-center gap-3 px-3.5 py-3">
                <span className="size-9 shrink-0 animate-pulse rounded-inner bg-line" />
                <span className="h-4 min-w-0 flex-1 animate-pulse rounded bg-line" />
              </Surface>
            ))}
          </div>
        </div>
      ))}
    </>
  );
}

export function AppsPanel() {
  const status = useAuthStore((s) => s.status);
  const openLoginDialog = useUiStore((s) => s.openLoginDialog);
  const authed = status === "authed";

  const [catalog, setCatalog] = useState<CatalogState>({ phase: "loading" });
  // 打开失败的即时反馈（成功时系统浏览器直接弹出，无需状态）
  const [openErr, setOpenErr] = useState<string | null>(null);
  // 可达性提示（webvpn：提示后仍打开原链接；unavailable：只提示不打开）
  const [accessHint, setAccessHint] = useState<string | null>(null);
  const [reloadTick, setReloadTick] = useState(0);

  // 目录加载：登录后取一次（失败可重试）
  useEffect(() => {
    if (!authed) return;
    let alive = true;
    setCatalog({ phase: "loading" });
    invokeCommand<AppCatalog>("get_app_catalog").then((r) => {
      if (!alive) return;
      if (r.success && r.data) {
        const d = r.data;
        setCatalog(
          d.groups.length > 0 || d.pinned.length > 0
            ? { phase: "ready", data: d }
            : { phase: "empty" },
        );
      } else {
        setCatalog({ phase: "error", message: r.message ?? "应用目录获取失败" });
      }
    });
    return () => {
      alive = false;
    };
  }, [authed, reloadTick]);

  // 打开策略（可达性元数据，后端按附录 A 实测表推导）：
  // - cas / external：直开（现状不变）；
  // - webvpn：给出明确提示（需校园网或 WebVPN）后仍打开原链接——校内用户
  //   无感直达，校外用户打开失败时已有解释；
  // - unavailable（实测死链/需自有登录）：只提示不打开（打开无意义）。
  const openApp = (item: AppItem) => {
    setOpenErr(null);
    setAccessHint(null);
    if (item.access === "unavailable") {
      setAccessHint(
        `「${item.name}」暂不可用：该应用链接失效或需自有账号登录，未在浏览器打开。`,
      );
      return;
    }
    if (item.access === "webvpn") {
      setAccessHint(
        `「${item.name}」需校园网或 WebVPN 环境：校外网络下可能无法访问，已尝试打开原链接。`,
      );
    }
    invokeCommand("open_app", { url: item.link, isCas: item.isCas }).then((r) => {
      if (!r.success) setOpenErr(`「${item.name}」打开失败：${r.message ?? "未知原因"}`);
    });
  };

  const data = catalog.phase === "ready" ? catalog.data : null;

  return (
    <section className="mx-auto mt-8 max-w-3xl px-4">
      <PanelHeader title="应用" description="校内服务与常用系统入口" domain="sched" />

      {!authed ? (
        <Surface>
          <EmptyState
            icon={LayoutGrid}
            domain="sched"
            title="登录后查看校园应用"
            hint="登录后展示融合门户应用中心的校内服务与常用系统。"
            action={<Button onClick={openLoginDialog}>登录</Button>}
          />
        </Surface>
      ) : (
        <>
          {/* 可达性提示（点击其他应用后清除） */}
          {accessHint && (
            <Surface accent="sched" className="px-4 py-3">
              <p className="text-caption text-text-2">{accessHint}</p>
            </Surface>
          )}

          {/* 打开失败提示（点击其他应用或重试后清除） */}
          {openErr && (
            <Surface accent="sched" className="px-4 py-3">
              <p className="text-caption text-text-2">{openErr}</p>
            </Surface>
          )}

          {catalog.phase === "loading" && <GroupSkeleton />}

          {catalog.phase === "error" && (
            <Surface className="mt-2">
              <EmptyState
                icon={Puzzle}
                domain="sched"
                title="应用目录获取失败"
                hint={catalog.message}
                action={
                  <Button variant="outline" onClick={() => setReloadTick((t) => t + 1)}>
                    重试
                  </Button>
                }
              />
            </Surface>
          )}

          {catalog.phase === "empty" && (
            <Surface className="mt-2">
              <EmptyState
                icon={Puzzle}
                domain="sched"
                title="暂无可用应用"
                hint="门户应用中心当前没有可展示的应用。"
              />
            </Surface>
          )}

          {catalog.phase === "ready" && data && (
            <>
              {/* 常用/收藏钉选区（queryMyStore；无钉选内容时整块隐藏） */}
              {data.pinned.length > 0 && (
                <div className={cn(openErr ? "mt-5" : "mt-1")}>
                  <h3 className="text-body font-medium text-text">常用</h3>
                  <div className="mt-2 grid grid-cols-3 gap-3">
                    {data.pinned.map((item) => (
                      <AppCard key={`pinned-${item.id}`} item={item} onOpen={openApp} />
                    ))}
                  </div>
                </div>
              )}

              {/* 部门分组网格（v2/queryApp 每组自带 depName，顺序按接口原样） */}
              {data.groups.map((group, gi) => (
                <div key={group.id} className={cn(gi === 0 && data.pinned.length === 0 && !openErr ? "mt-1" : "mt-5")}>
                  <h3 className="text-body font-medium text-text">{group.name}</h3>
                  <div className="mt-2 grid grid-cols-3 gap-3">
                    {group.apps.map((item) => (
                      <AppCard key={item.id} item={item} onOpen={openApp} />
                    ))}
                  </div>
                </div>
              ))}

              {data.groups.every((g) => g.apps.length === 0) && data.pinned.length === 0 && (
                <Surface className="mt-5">
                  <EmptyState
                    compact
                    icon={Puzzle}
                    domain="sched"
                    title="分组内暂无应用"
                    hint="门户应用中心的分组下当前没有可展示的应用。"
                  />
                </Surface>
              )}
            </>
          )}
        </>
      )}
    </section>
  );
}
