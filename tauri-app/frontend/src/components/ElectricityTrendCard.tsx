import { RefreshCw, TrendingUp } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { Surface } from "@/components/Surface";
import { Button } from "@/components/ui/button";
import { invokeCommand } from "@/shared/tauriApi";
import type { ElectricityHistoryEntry } from "@/shared/types";

/**
 * 电费变化卡（M4 批 3）：**自采日余额快照**（`get_electricity_history`，纯本地读不需登录）
 * 的 SVG 折线 + 日均消耗 + 按当前消耗预估可用天数。
 *
 * 数据纪律（任务书 §2/§5，逐条落实）：
 * - 学校侧没有每日余额序列，曲线**只能来自客户端自采**——应用没开的日子就是空档，如实留空；
 * - `balance === null` = 那天**没采到金额**：断线处理，**不插值、不连过缺口、不画成 0**；
 * - 有效数据点 < 2 个不画线（不许画假线），给空状态文案说明「还没采到」与怎么补。
 */

/** 折线窗口：最近 30 天（后端 `days` 上限 3650，这里取一屏可读的小窗口）。 */
const DAYS = 30;

/** "YYYY-MM-DD" → 天数差（同源本地日期串，直接 Date.parse 安全）。 */
function dayDiff(from: string, to: string): number {
  return Math.round((Date.parse(to) - Date.parse(from)) / 86_400_000);
}

type Phase = "loading" | "ready" | "error";

/** 一段连续有效点（中间没有 null 断口）。 */
interface Segment {
  key: string;
  /** viewBox 坐标（0..100 × 0..40），已按 min/max 归一化 */
  points: string;
}

export function ElectricityTrendCard({
  roomId,
  roomLabel,
  reloadTick,
}: {
  /** 「我的宿舍」的 `SavedRoom.id`；null = 未绑定（不发起请求，直接给引导空态） */
  roomId: string | null;
  roomLabel: string | null;
  /** 父组件在采集/绑定变化后自增，触发重读本地历史 */
  reloadTick: number;
}) {
  const [entries, setEntries] = useState<ElectricityHistoryEntry[]>([]);
  const [phase, setPhase] = useState<Phase>("loading");
  const [error, setError] = useState("");
  const [tick, setTick] = useState(0);

  useEffect(() => {
    if (!roomId) {
      setPhase("ready");
      setEntries([]);
      return;
    }
    let alive = true;
    setPhase("loading");
    invokeCommand<ElectricityHistoryEntry[]>("get_electricity_history", {
      roomId,
      days: DAYS,
    }).then((r) => {
      if (!alive) return;
      if (r.success && r.data) {
        setEntries(r.data);
        setPhase("ready");
      } else {
        setError(r.message ?? "本地历史读取失败");
        setPhase("error");
      }
    });
    return () => {
      alive = false;
    };
  }, [roomId, reloadTick, tick]);

  // 纯图形推导放 useMemo：entries（时间升序）→ 有效点 + 连续段 + 统计
  const chart = useMemo(() => {
    const valid = entries
      .map((e, i) => ({ i, date: e.date, balance: e.balance }))
      .filter((p): p is { i: number; date: string; balance: number } => p.balance != null);
    const stats =
      valid.length >= 2
        ? (() => {
            const first = valid[0]!;
            const last = valid[valid.length - 1]!;
            // 逐对相邻有效点累加「下降量」（充值导致的上升不计入消耗）
            let decrease = 0;
            for (let k = 1; k < valid.length; k += 1) {
              const prev = valid[k - 1]!;
              const cur = valid[k]!;
              if (prev.balance > cur.balance) decrease += prev.balance - cur.balance;
            }
            const days = dayDiff(first.date, last.date);
            const avg = days > 0 ? decrease / days : null;
            const estimate =
              avg != null && avg > 0 && last.balance > 0 ? last.balance / avg : null;
            return { first, last, avg, estimate };
          })()
        : null;
    let segments: Segment[] = [];
    if (valid.length >= 2 && entries.length >= 2) {
      const balances = valid.map((p) => p.balance);
      const min = Math.min(...balances);
      const max = Math.max(...balances);
      const yOf = (v: number) => (max === min ? 20 : 36 - ((v - min) / (max - min)) * 32);
      let run: string[] = [];
      let vi = 0;
      const flush = () => {
        if (run.length >= 2) {
          segments.push({ key: `seg-${segments.length}`, points: run.join(" ") });
        }
        run = [];
      };
      for (let i = 0; i < entries.length; i += 1) {
        const p = valid[vi];
        if (p && p.i === i) {
          const x = (i / (entries.length - 1)) * 100;
          run.push(`${x.toFixed(2)},${yOf(p.balance).toFixed(2)}`);
          vi += 1;
        } else {
          flush();
        }
      }
      flush();
    }
    return { valid, stats, segments };
  }, [entries]);

  const { valid, stats, segments } = chart;

  return (
    <Surface accent="wallet" className="px-4 py-4">
      <div className="flex items-baseline justify-between gap-3">
        <p className="text-body font-medium text-text">电费变化</p>
        {roomLabel && (
          <span className="min-w-0 truncate text-caption text-text-2">{roomLabel}</span>
        )}
      </div>
      <p className="mt-0.5 text-caption text-text-2">
        来自本机每日自动采集（学校侧没有该数据）；缺口 = 当天没采集到。
      </p>

      {phase === "error" ? (
        <div className="mt-3 flex items-center justify-between gap-3">
          <p className="min-w-0 text-body text-text-2">{error}</p>
          <Button
            variant="outline"
            size="sm"
            className="shrink-0"
            onClick={() => setTick((t) => t + 1)}
          >
            <RefreshCw />
            重试
          </Button>
        </div>
      ) : !roomId ? (
        /* 空态用矮提示条而非大空态卡：与绑定后「只有 1 个点」的空态高度接近，
           避免右列在绑定前后大幅跳动（M4 点验布局反馈）。 */
        <div className="mt-3 flex items-start gap-2.5 rounded-inner border border-line bg-surface-2 px-3 py-3">
          <TrendingUp aria-hidden className="text-wallet mt-0.5 size-4 shrink-0" />
          <div className="min-w-0">
            <p className="text-body text-text">还没有绑定宿舍</p>
            <p className="mt-0.5 text-caption text-text-2">
              在左侧查到房间后点「绑定为我的宿舍」，之后每次打开应用会自动记录当日余额。
            </p>
          </div>
        </div>
      ) : phase === "loading" ? (
        <div aria-hidden className="mt-3 space-y-2">
          <div className="h-24 animate-pulse rounded-inner bg-line" />
          <div className="h-4 w-2/3 animate-pulse rounded bg-line" />
        </div>
      ) : valid.length < 2 ? (
        <div className="mt-3 flex items-start gap-2.5 rounded-inner border border-line bg-surface-2 px-3 py-3">
          <TrendingUp aria-hidden className="text-wallet mt-0.5 size-4 shrink-0" />
          <div className="min-w-0">
            <p className="text-body text-text">还采不到趋势</p>
            <p className="mt-0.5 text-caption text-text-2">
              {valid.length === 0
                ? "最近 30 天没有有效的余额记录。应用启动时会自动补采当日余额，也可以查到房间后点「立即采集」。"
                : "最近 30 天只有 1 个有效余额记录，再过几天就能画出变化。"}
            </p>
          </div>
        </div>
      ) : (
        <>
          {/* 折线：preserveAspectRatio=none 拉伸铺满；vector-effect 保住描边不随 y 拉伸变形 */}
          <svg
            viewBox="0 0 100 40"
            preserveAspectRatio="none"
            aria-hidden
            className="text-wallet mt-3 h-24 w-full"
          >
            {segments.map((s) => (
              <polyline
                key={s.key}
                points={s.points}
                fill="none"
                stroke="currentColor"
                strokeWidth={2}
                strokeLinecap="round"
                strokeLinejoin="round"
                vectorEffect="non-scaling-stroke"
              />
            ))}
          </svg>
          <dl className="mt-3 space-y-1.5">
            <div className="flex items-baseline justify-between gap-3">
              <dt className="text-caption text-text-2">最新余额</dt>
              <dd
                className={`tabular-num text-body font-medium ${
                  stats!.last.balance < 0 ? "text-alert" : "text-text"
                }`}
              >
                ¥ {stats!.last.balance.toFixed(2)}
                <span className="text-caption text-text-2">（{stats!.last.date}）</span>
              </dd>
            </div>
            <div className="flex items-baseline justify-between gap-3">
              <dt className="text-caption text-text-2">日均消耗（统计期内）</dt>
              <dd className="tabular-num text-body text-text">
                {stats!.avg != null && stats!.avg > 0
                  ? `¥ ${stats!.avg.toFixed(2)} / 天`
                  : "—"}
              </dd>
            </div>
            <div className="flex items-baseline justify-between gap-3">
              <dt className="text-caption text-text-2">按当前消耗预估可用</dt>
              <dd className="tabular-num text-body text-text">
                {stats!.estimate != null ? `约 ${Math.floor(stats!.estimate)} 天` : "—"}
              </dd>
            </div>
          </dl>
          {stats!.avg != null && stats!.avg <= 0 && (
            <p className="mt-2 text-caption text-text-2">
              统计期内余额没有下降（可能还没住人使用或刚充值），暂无法估算可用天数。
            </p>
          )}
        </>
      )}
    </Surface>
  );
}
