import { ChevronDown, Receipt, Search } from "lucide-react";
import { useEffect, useState } from "react";
import { EmptyState } from "@/components/EmptyState";
import { Surface } from "@/components/Surface";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { invokeCommand } from "@/shared/tauriApi";
import { cn } from "@/shared/cn";
import type {
  EcardTransaction,
  EcardTransactions,
  EcardTurnoverType,
} from "@/shared/types";

/**
 * 一卡通「账单/流水」子页（M4.5，自取数）。筛选维度与参数名严格对齐后端
 * `get_ecard_transactions(account?, page, size?, type?, typeId?, info?, orderId?)`：
 * - 方向 `type`：`"1"` 收入 / `"2"` 支出 / 不传全量；
 * - 分类 `typeId`：选项来自 `get_ecard_types`，**接口失败只降级分类筛选条**、列表照常；
 * - 关键词 `info`；单条详情走 `{ orderId }`（服务端回 total=1 的一页）。
 * 切换任一筛选回第 1 页重取；「加载更多」按 `records.length < total`（与钱包页同写法）。
 */

const PAGE_SIZE = 15;

/** 列表三态：loading = 本页在取（首屏骨架 /「加载更多」按钮态）。 */
type ListPhase = "loading" | "idle" | "error";

/** 单条详情的展开状态：按 orderId 缓存，展开时缺缓存才发请求。 */
type DetailState =
  | { orderId: null }
  | { orderId: string; phase: "loading" }
  | { orderId: string; phase: "ready"; record: EcardTransaction }
  | { orderId: string; phase: "error"; message: string };

/** 展开中的详情（判别收窄后的视图）：orderId 对不上或未展开 → null。 */
type OpenDetail = Extract<DetailState, { orderId: string }>;

/** `"YYYY-MM-DD HH:MM:SS"` → `"MM-DD HH:MM"`（流水不跨年，行内保持紧凑）。 */
function shortTime(t: string): string {
  return t.slice(5, 16);
}

/** 金额展示：符号按方向字段算，金额取绝对值（后端符号口径不一致时也不出 `--`）。 */
function signedAmount(v: number, income: boolean): string {
  return `${income ? "+" : "-"}${Math.abs(v).toFixed(2)}`;
}

export function EcardBillView({
  accounts,
}: {
  /** 可切换的卡（容器面板给）；为空则不带 account 查询（后端查本人全部）。 */
  accounts: { account: string; label: string }[];
}) {
  const [accountIdx, setAccountIdx] = useState(0);
  const [direction, setDirection] = useState<"all" | "in" | "out">("all");
  const [typeId, setTypeId] = useState<string>("");
  const [keywordText, setKeywordText] = useState("");
  const [keyword, setKeyword] = useState("");

  const [types, setTypes] = useState<EcardTurnoverType[]>([]);
  const [typesOk, setTypesOk] = useState(true);

  const [page, setPage] = useState(1);
  const [records, setRecords] = useState<EcardTransaction[]>([]);
  const [total, setTotal] = useState(0);
  const [listPhase, setListPhase] = useState<ListPhase>("loading");
  const [listError, setListError] = useState("");
  const [reloadTick, setReloadTick] = useState(0);

  const [detail, setDetail] = useState<DetailState>({ orderId: null });

  const account = accounts[accountIdx]?.account;

  // 分类字典：失败只降级分类筛选条（typesOk = false ⇒ 隐藏 chips），列表照常
  useEffect(() => {
    let alive = true;
    invokeCommand<EcardTurnoverType[]>("get_ecard_types").then((r) => {
      if (!alive) return;
      if (r.success && r.data) {
        setTypes([...r.data].sort((a, b) => a.showOrder - b.showOrder));
        setTypesOk(true);
      } else {
        setTypes([]);
        setTypesOk(false);
      }
    });
    return () => {
      alive = false;
    };
  }, []);

  // 流水列表：任一筛选变化都回第 1 页（筛选 setter 里统一 setPage(1)）
  useEffect(() => {
    let alive = true;
    setListPhase("loading");
    invokeCommand<EcardTransactions>("get_ecard_transactions", {
      ...(account ? { account } : {}),
      page,
      size: PAGE_SIZE,
      ...(direction === "all" ? {} : { type: direction === "in" ? "1" : "2" }),
      ...(typeId ? { typeId } : {}),
      ...(keyword ? { info: keyword } : {}),
    }).then((r) => {
      if (!alive) return;
      const data = r.data;
      if (r.success && data) {
        setTotal(data.total);
        setRecords((prev) =>
          page === 1
            ? data.records
            : [...prev, ...data.records],
        );
        setListPhase("idle");
      } else {
        setListError(r.message ?? "交易记录获取失败");
        setListPhase("error");
      }
    });
    return () => {
      alive = false;
    };
  }, [account, page, direction, typeId, keyword, reloadTick]);

  /** 任一筛选变更：展开详情收起、回第 1 页重取。 */
  const applyFilter = (mutate: () => void) => {
    setDetail({ orderId: null });
    mutate();
    setPage(1);
    setRecords([]);
  };

  const clearFilters = () =>
    applyFilter(() => {
      setDirection("all");
      setTypeId("");
      setKeywordText("");
      setKeyword("");
    });

  const retry = () => {
    setDetail({ orderId: null });
    setPage(1);
    setRecords([]);
    setReloadTick((t) => t + 1);
  };

  /** 展开/收起一行详情；展开且无缓存时用 orderId 取单条（total=1）。 */
  const toggleDetail = (orderId: string) => {
    if (detail.orderId === orderId) {
      setDetail({ orderId: null });
      return;
    }
    setDetail({ orderId, phase: "loading" });
    invokeCommand<EcardTransactions>("get_ecard_transactions", { orderId }).then((r) => {
      const rec = r.data?.records[0];
      if (r.success && rec) {
        setDetail({ orderId, phase: "ready", record: rec });
      } else {
        setDetail({ orderId, phase: "error", message: r.message ?? "详情获取失败" });
      }
    });
  };

  const hasMore = listPhase !== "error" && records.length > 0 && records.length < total;
  const filtered = direction !== "all" || typeId !== "" || keyword !== "";

  return (
    <div className="flex flex-col gap-3">
      {/* 多卡切换：单卡不渲染 */}
      {accounts.length > 1 && (
        <div
          role="tablist"
          aria-label="切换账单卡"
          className="flex gap-1 self-start rounded-control border border-line bg-surface-2 p-1"
        >
          {accounts.map((a, i) => (
            <button
              key={a.account}
              type="button"
              role="tab"
              aria-selected={i === accountIdx}
              className={cn(
                "min-h-8 rounded-control px-3 text-caption transition-colors duration-[var(--dur-fast)] ease-out-soft",
                i === accountIdx
                  ? "bg-surface font-medium text-text shadow-card"
                  : "text-text-2 hover:text-text",
              )}
              onClick={() => applyFilter(() => setAccountIdx(i))}
            >
              {a.label}
            </button>
          ))}
        </div>
      )}

      {/* 筛选条：方向 + 分类（字典失败时降级隐藏）+ 关键词 */}
      <Surface className="px-4 py-4">
        <div className="flex flex-wrap items-center gap-2">
          <div
            role="tablist"
            aria-label="按方向筛选"
            className="flex gap-1 rounded-control border border-line bg-surface-2 p-1"
          >
            {(
              [
                { key: "all", label: "全部" },
                { key: "in", label: "收入" },
                { key: "out", label: "支出" },
              ] as const
            ).map((opt) => (
              <button
                key={opt.key}
                type="button"
                role="tab"
                aria-selected={direction === opt.key}
                className={cn(
                  "min-h-8 rounded-control px-3 text-caption transition-colors duration-[var(--dur-fast)] ease-out-soft",
                  direction === opt.key
                    ? "bg-surface font-medium text-text shadow-card"
                    : "text-text-2 hover:text-text",
                )}
                onClick={() => applyFilter(() => setDirection(opt.key))}
              >
                {opt.label}
              </button>
            ))}
          </div>
          <form
            className="flex min-w-0 flex-1 gap-2"
            onSubmit={(e) => {
              e.preventDefault();
              applyFilter(() => setKeyword(keywordText.trim()));
            }}
          >
            <Input
              value={keywordText}
              placeholder="搜摘要 / 地点，回车筛选"
              aria-label="关键词筛选"
              className="min-w-0"
              onChange={(e) => setKeywordText(e.target.value)}
            />
            <Button type="submit" variant="outline" size="icon" aria-label="应用关键词筛选">
              <Search aria-hidden />
            </Button>
          </form>
        </div>
        {typesOk && types.length > 0 && (
          <div className="mt-2 flex flex-wrap gap-1.5">
            <Button
              variant={typeId === "" ? "default" : "outline"}
              size="xs"
              aria-pressed={typeId === ""}
              onClick={() => applyFilter(() => setTypeId(""))}
            >
              全部分类
            </Button>
            {types.map((t) => (
              <Button
                key={t.id}
                variant={typeId === String(t.id) ? "default" : "outline"}
                size="xs"
                aria-pressed={typeId === String(t.id)}
                onClick={() => applyFilter(() => setTypeId(String(t.id)))}
              >
                {t.name}
              </Button>
            ))}
          </div>
        )}
      </Surface>

      {/* 流水列表卡：加载 / 错误重试 / 空态 / 分页追加 */}
      <Surface className="px-4 py-4">
        <div className="flex items-baseline justify-between gap-3">
          <p className="text-body font-medium text-text">交易记录</p>
          {total > 0 && (
            <span className="tabular-num shrink-0 text-caption text-text-2">共 {total} 条</span>
          )}
        </div>

        {listPhase === "error" ? (
          <div className="mt-3 flex items-center justify-between gap-3">
            <p className="min-w-0 text-body text-text-2">获取失败：{listError}</p>
            <Button variant="outline" size="sm" className="shrink-0" onClick={retry}>
              重试
            </Button>
          </div>
        ) : records.length === 0 ? (
          listPhase === "loading" ? (
            <div aria-hidden className="mt-3 space-y-2">
              {[0, 1, 2].map((i) => (
                <div key={i} className="h-10 animate-pulse rounded bg-line" />
              ))}
            </div>
          ) : filtered ? (
            <EmptyState
              compact
              icon={Receipt}
              domain="wallet"
              title="没有符合条件的记录"
              action={
                <Button variant="outline" size="sm" onClick={clearFilters}>
                  清除筛选
                </Button>
              }
            />
          ) : (
            <EmptyState
              compact
              icon={Receipt}
              domain="wallet"
              title="暂无交易记录"
              hint="还没有一卡通消费或充值记录。"
            />
          )
        ) : (
          <ul className="mt-2 divide-y divide-line">
            {records.map((r, i) => {
              const income = r.isIncome;
              const orderId = r.orderId;
              const open: OpenDetail | null =
                detail.orderId !== null && detail.orderId === orderId ? detail : null;
              const expanded = open !== null;
              const summary = r.summary || r.payName || "交易";
              return (
                <li key={`${r.time}-${orderId || i}`}>
                  <button
                    type="button"
                    aria-expanded={expanded}
                    className="flex w-full items-baseline gap-3 rounded-control py-2.5 text-left transition-colors duration-[var(--dur-fast)] ease-out-soft hover:bg-surface-2"
                    onClick={() => orderId && toggleDetail(orderId)}
                  >
                    <span className="min-w-0 flex-1">
                      <span className="block truncate text-body text-text">{summary}</span>
                      <span className="mt-0.5 block truncate text-caption text-text-2">
                        {[
                          shortTime(r.time),
                          r.locationName,
                          r.payName,
                        ]
                          .filter(Boolean)
                          .join(" · ")}
                      </span>
                    </span>
                    <span
                      className={cn(
                        "tabular-num shrink-0 text-body",
                        income ? "text-wallet" : "text-text",
                      )}
                    >
                      {signedAmount(r.amountYuan, income)}
                    </span>
                    {orderId && (
                      <ChevronDown
                        aria-hidden
                        className={cn(
                          "size-4 shrink-0 text-text-2 transition-transform duration-[var(--dur-fast)] ease-out-soft",
                          expanded && "rotate-180",
                        )}
                      />
                    )}
                  </button>
                  {expanded && open && (
                    <div className="mb-2.5 rounded-inner border border-line bg-surface-2 px-3 py-2.5">
                      {open.phase === "loading" ? (
                        <div aria-hidden className="h-12 animate-pulse rounded bg-line" />
                      ) : open.phase === "error" ? (
                        <div className="flex items-center justify-between gap-3">
                          <p className="min-w-0 text-caption text-text-2">{open.message}</p>
                          <Button
                            variant="outline"
                            size="xs"
                            className="shrink-0"
                            onClick={() => toggleDetail(orderId)}
                          >
                            重试
                          </Button>
                        </div>
                      ) : (
                        <dl className="grid grid-cols-2 gap-x-4 gap-y-1 sm:grid-cols-3">
                          {(
                            [
                              { label: "交易后余额", value: open.record.cardBalanceYuan, money: true },
                              { label: "类型", value: open.record.turnoverType },
                              { label: "分类", value: open.record.labelName },
                              { label: "分类备注", value: open.record.labelRemark },
                            ] as { label: string; value: string | number | null; money?: boolean }[]
                          )
                            .filter((f) => f.value !== undefined && f.value !== null && f.value !== "")
                            .map((f) => (
                              <div key={f.label} className="min-w-0">
                                <dt className="text-caption text-text-2">{f.label}</dt>
                                <dd className="tabular-num truncate text-caption text-text">
                                  {f.money && typeof f.value === "number"
                                    ? `¥ ${f.value.toFixed(2)}`
                                    : String(f.value)}
                                </dd>
                              </div>
                            ))}
                        </dl>
                      )}
                    </div>
                  )}
                </li>
              );
            })}
          </ul>
        )}

        {hasMore && (
          <div className="mt-3 flex justify-center">
            <Button
              variant="outline"
              size="sm"
              disabled={listPhase === "loading"}
              onClick={() => setPage((p) => p + 1)}
            >
              {listPhase === "loading" ? "加载中…" : "加载更多"}
            </Button>
          </div>
        )}
      </Surface>
    </div>
  );
}
