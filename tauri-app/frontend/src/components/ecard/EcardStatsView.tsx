import { BarChart3, RefreshCw } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { EmptyState } from "@/components/EmptyState";
import { Surface } from "@/components/Surface";
import { MiniLine } from "@/components/ecard/MiniLine";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { invokeCommand } from "@/shared/tauriApi";
import { cn } from "@/shared/cn";
import type {
  EcardStatsAssortItem,
  EcardStatsPoint,
  EcardStatsSummary,
  EcardTurnoverType,
} from "@/shared/types";

/**
 * 一卡通「统计」子页（M4.5，自取数）。三个读命令：
 * - `get_ecard_stats_summary({ timeFrom, timeTo })` → 区间收支合计；
 * - `get_ecard_stats_series({ dateStr, dateType, statisticsDateStr, type })`：月视图
 *   `dateType="month"` + `dateStr="YYYY-MM"` + 按日；年视图 `dateType="year"` +
 *   `dateStr="YYYY"` + 按月；`type` 1 收入 / 2 支出——**两条序列分开取**，不混进一条线；
 * - `get_ecard_stats_assort({ type, timeFrom, timeTo })`：分类占比（取**支出**口径；
 *   收入只有充值/补贴两类，拆分意义有限）。`timeFrom`/`timeTo` 由前端算成
 *   `YYYY-MM-DD` 区间，服务端只吃区间。
 */

type Mode = "month" | "year" | "custom";
type Phase = "loading" | "ready" | "error";

const PAGE_MODES = [
  { key: "month", label: "本月" },
  { key: "year", label: "本年" },
  { key: "custom", label: "自定义月份" },
] as const;

function pad2(n: number): string {
  return String(n).padStart(2, "0");
}

/** 当前月份 "YYYY-MM" / 当前年份 "YYYY"（本地时区）。 */
function currentYm(): string {
  const now = new Date();
  return `${now.getFullYear()}-${pad2(now.getMonth() + 1)}`;
}

/** "YYYY-MM" → 前端算好的区间（当月 1 日 ~ 月末）+ 折线参数。 */
function monthRange(ym: string) {
  const [y, m] = ym.split("-").map(Number);
  const last = new Date(y, m, 0).getDate();
  return {
    dateStr: ym,
    dateType: "month",
    statisticsDateStr: "day",
    timeFrom: `${ym}-01`,
    timeTo: `${ym}-${pad2(last)}`,
  };
}

/** "YYYY" → 区间（01-01 ~ 12-31）+ 折线参数。 */
function yearRange(year: string) {
  return {
    dateStr: year,
    dateType: "year",
    statisticsDateStr: "month",
    timeFrom: `${year}-01-01`,
    timeTo: `${year}-12-31`,
  };
}

/** 序列无数据或全 0：不画空折线，给 caption（真实零值也如实说明）。 */
function seriesEmpty(points: EcardStatsPoint[]): boolean {
  return points.length === 0 || points.every((p) => p.amountYuan === 0);
}

/** 本地今天的 `YYYY-MM-DD`（不用 toISOString：那是 UTC，中国时区会差一天）。 */
function localToday(): string {
  const d = new Date();
  return `${d.getFullYear()}-${pad2(d.getMonth() + 1)}-${pad2(d.getDate())}`;
}

/**
 * 砍掉**尾部未来日期**：服务端按月返回整月（含未来日期、恒 0），照画会让折线一路拖到月末，
 * 看起来像「已经消费到月底」。只砍尾部——中间的零值是真实「当天没消费」，必须保留。
 * 年视图的 `label` 是 `YYYY-MM`，与 `YYYY-MM-DD` 做字符串比较依然正确（`2026-09` < `2026-09-19`）。
 */
function trimFuture(points: EcardStatsPoint[]): EcardStatsPoint[] {
  const today = localToday();
  const cut = points.findIndex((p) => p.label > today);
  return cut === -1 ? points : points.slice(0, cut);
}

export function EcardStatsView() {
  const [mode, setMode] = useState<Mode>("month");
  const [customMonth, setCustomMonth] = useState(currentYm);
  const [tick, setTick] = useState(0);

  const [phase, setPhase] = useState<Phase>("loading");
  const [error, setError] = useState("");
  const [summary, setSummary] = useState<EcardStatsSummary | null>(null);
  const [seriesOut, setSeriesOut] = useState<EcardStatsPoint[]>([]);
  const [seriesIn, setSeriesIn] = useState<EcardStatsPoint[]>([]);
  const [assort, setAssort] = useState<EcardStatsAssortItem[]>([]);
  /** 分类中文名（get_ecard_types 字典；失败静默降级用 nameEn / typeId）。 */
  const [typeNames, setTypeNames] = useState<Map<string, string>>(new Map());

  const range = useMemo(
    () => (mode === "year" ? yearRange(String(new Date().getFullYear())) : monthRange(customMonth)),
    [mode, customMonth],
  );

  useEffect(() => {
    let alive = true;
    setPhase("loading");
    // 两条序列分开取（type 2 支出 / 1 收入）；分类占比取支出口径
    Promise.all([
      invokeCommand<EcardStatsSummary>("get_ecard_stats_summary", {
        timeFrom: range.timeFrom,
        timeTo: range.timeTo,
      }),
      invokeCommand<EcardStatsPoint[]>("get_ecard_stats_series", {
        dateStr: range.dateStr,
        dateType: range.dateType,
        statisticsDateStr: range.statisticsDateStr,
        type: "2",
      }),
      invokeCommand<EcardStatsPoint[]>("get_ecard_stats_series", {
        dateStr: range.dateStr,
        dateType: range.dateType,
        statisticsDateStr: range.statisticsDateStr,
        type: "1",
      }),
      invokeCommand<EcardStatsAssortItem[]>("get_ecard_stats_assort", {
        type: "2",
        timeFrom: range.timeFrom,
        timeTo: range.timeTo,
      }),
      invokeCommand<EcardTurnoverType[]>("get_ecard_types"),
    ]).then(([s, out, inn, assortRes, typesRes]) => {
      if (!alive) return;
      if (
        s.success &&
        s.data &&
        out.success &&
        out.data &&
        inn.success &&
        inn.data &&
        assortRes.success &&
        assortRes.data
      ) {
        setSummary(s.data);
        setSeriesOut(trimFuture(out.data));
        setSeriesIn(trimFuture(inn.data));
        setAssort(assortRes.data);
        setPhase("ready");
      } else {
        setError(
          s.message || out.message || inn.message || assortRes.message || "统计获取失败",
        );
        setPhase("error");
      }
      // 分类字典失败不影响主统计，只降级分类中文名
      if (typesRes.success && typesRes.data) {
        setTypeNames(new Map(typesRes.data.map((t) => [String(t.id), t.name])));
      }
    });
    return () => {
      alive = false;
    };
  }, [range, tick]);

  /** 分类聚合的渲染数据：条宽 = 该分类 / 最大分类；百分比 = 占总额（1 位小数）。 */
  const assortRows = useMemo(() => {
    const items = assort.filter((a) => a.amountYuan !== 0);
    const total = items.reduce((sum, a) => sum + a.amountYuan, 0);
    const max = Math.max(0, ...items.map((a) => a.amountYuan));
    return items
      .sort((a, b) => b.amountYuan - a.amountYuan)
      .map((a) => ({
        key: a.typeId,
        name: typeNames.get(a.typeId) || a.nameEn || a.typeId,
        amountYuan: a.amountYuan,
        barPct: max > 0 ? (a.amountYuan / max) * 100 : 0,
        sharePct: total > 0 ? (a.amountYuan / total) * 100 : 0,
      }));
  }, [assort, typeNames]);

  const balance = summary ? summary.incomeYuan - summary.expensesYuan : 0;
  const rangeLabel =
    mode === "year" ? `${range.timeFrom.slice(0, 4)} 年` : `${range.dateStr} 月`;

  return (
    <div className="flex flex-col gap-3">
      {/* 时间维度切换：本月 / 本年 / 自定义月份（原生 input[type=month]） */}
      <Surface className="px-4 py-4">
        <div className="flex flex-wrap items-center gap-2">
          <div
            role="tablist"
            aria-label="统计时间范围"
            className="flex gap-1 rounded-control border border-line bg-surface-2 p-1"
          >
            {PAGE_MODES.map((m) => (
              <button
                key={m.key}
                type="button"
                role="tab"
                aria-selected={mode === m.key}
                className={cn(
                  "min-h-8 rounded-control px-3 text-caption transition-colors duration-[var(--dur-fast)] ease-out-soft",
                  mode === m.key
                    ? "bg-surface font-medium text-text shadow-card"
                    : "text-text-2 hover:text-text",
                )}
                onClick={() => setMode(m.key)}
              >
                {m.label}
              </button>
            ))}
          </div>
          {mode === "custom" && (
            <Input
              type="month"
              value={customMonth}
              aria-label="选择统计月份"
              className="w-40"
              onChange={(e) => e.target.value && setCustomMonth(e.target.value)}
            />
          )}
          <span className="ml-auto text-caption text-text-2">
            {range.timeFrom} ~ {range.timeTo}
          </span>
        </div>
      </Surface>

      {phase === "loading" ? (
        <div aria-hidden className="flex flex-col gap-3">
          <div className="grid grid-cols-3 gap-3">
            {[0, 1, 2].map((i) => (
              <div key={i} className="h-20 animate-pulse rounded-card bg-line" />
            ))}
          </div>
          <div className="h-40 animate-pulse rounded-card bg-line" />
          <div className="h-28 animate-pulse rounded-card bg-line" />
        </div>
      ) : phase === "error" ? (
        <Surface accent="info" className="flex items-center justify-between gap-3 px-4 py-3">
          <p className="min-w-0 text-body text-text-2">获取失败：{error}</p>
          <Button
            variant="outline"
            size="sm"
            className="shrink-0"
            onClick={() => setTick((t) => t + 1)}
          >
            <RefreshCw aria-hidden />
            重试
          </Button>
        </Surface>
      ) : (
        <>
          {/* 三个数字卡：支出 / 收入 / 结余（负数告警红） */}
          <div className="grid grid-cols-3 gap-3">
            {[
              { label: "支出", value: summary?.expensesYuan ?? 0, negative: false },
              { label: "收入", value: summary?.incomeYuan ?? 0, negative: false },
              { label: "结余", value: balance, negative: balance < 0 },
            ].map((card) => (
              <Surface key={card.label} className="px-4 py-4">
                <p className="text-caption text-text-2">{card.label}</p>
                <p
                  className={cn(
                    "tabular-num mt-2 text-title font-semibold",
                    card.negative ? "text-alert" : "text-text",
                  )}
                >
                  ¥ {card.value.toFixed(2)}
                </p>
              </Surface>
            ))}
          </div>

          {/* 收支趋势：两条序列分开画，互不混线；空 / 全 0 给 caption */}
          <Surface accent="wallet" className="px-4 py-4">
            <div className="flex items-baseline justify-between gap-3">
              <p className="text-body font-medium text-text">收支趋势</p>
              <span className="text-caption text-text-2">{rangeLabel}（按{mode === "year" ? "月" : "日"}）</span>
            </div>
            {(["out", "in"] as const).map((side) => {
              const points = side === "out" ? seriesOut : seriesIn;
              const name = side === "out" ? "支出" : "收入";
              return (
                <div key={side} className="mt-3">
                  <p className="text-caption text-text-2">{name}</p>
                  {seriesEmpty(points) ? (
                    <p className="mt-2 text-caption text-text-2">该区间没有流水。</p>
                  ) : (
                    <MiniLine
                      points={points}
                      height={88}
                      ariaLabel={`${rangeLabel}${name}趋势，共 ${points.length} 个数据点`}
                    />
                  )}
                </div>
              );
            })}
          </Surface>

          {/* 支出分类占比：条形宽 = 该分类 / 最大分类；单分类自然只有一行 100% 条 */}
          <Surface className="px-4 py-4">
            <div className="flex items-baseline justify-between gap-3">
              <p className="text-body font-medium text-text">支出分类</p>
              <span className="text-caption text-text-2">{rangeLabel}</span>
            </div>
            {assortRows.length === 0 ? (
              <EmptyState
                compact
                icon={BarChart3}
                domain="wallet"
                title="该区间没有支出分类数据"
              />
            ) : (
              <ul className="mt-3 space-y-2.5">
                {assortRows.map((row) => (
                  <li key={row.key}>
                    <div className="flex items-baseline justify-between gap-3">
                      <span className="min-w-0 truncate text-body text-text">{row.name}</span>
                      <span className="tabular-num shrink-0 text-caption text-text-2">
                        ¥ {row.amountYuan.toFixed(2)} · {row.sharePct.toFixed(1)}%
                      </span>
                    </div>
                    <div
                      aria-hidden
                      className="mt-1 h-1.5 overflow-hidden rounded-full bg-surface-2"
                    >
                      <div
                        className="h-full rounded-full bg-wallet"
                        style={{ width: `${Math.max(row.barPct, 2)}%` }}
                      />
                    </div>
                  </li>
                ))}
              </ul>
            )}
          </Surface>
        </>
      )}
    </div>
  );
}
