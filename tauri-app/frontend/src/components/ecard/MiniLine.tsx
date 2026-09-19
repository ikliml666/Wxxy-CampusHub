import { cn } from "@/shared/cn";

/**
 * 迷你折线（M4.5 一卡通统计用）：无依赖手绘 SVG，与 `ElectricityTrendCard` 同一做法——
 * `viewBox="0 0 100 40"` + `preserveAspectRatio="none"` 拉伸铺满 +
 * `vectorEffect="non-scaling-stroke"` 保住描边不随拉伸变形；颜色走 `currentColor`
 * （默认容器给域色 `text-wallet`，调用方可覆盖）。
 *
 * 与电费余额线的语义差异：这里的 `amountYuan` 是统计接口返回的**真实零值**
 * （当天没消费 = 0），0 照画、**不断线**——与电费「null = 没采到」的断线语义不同。
 * 极值标注：最高 / 最低各一个 8px 圆点 + 值（`tabular-num`），不画坐标轴网格。
 */

/** 一个数据点：横轴标签（进 aria-label）+ 纵轴金额（元）。 */
export interface MiniLinePoint {
  label: string;
  amountYuan: number;
}

export function MiniLine({
  points,
  height = 96,
  className,
  ariaLabel,
}: {
  points: MiniLinePoint[];
  height?: number;
  className?: string;
  ariaLabel: string;
}) {
  // 不足 2 个点无法成线：给 caption 提示，不画空图（与电费卡「不画假线」同纪律）
  if (points.length < 2) {
    return <p className="text-caption text-text-2">数据点不足 2 个，暂不能画出趋势。</p>;
  }

  const values = points.map((p) => p.amountYuan);
  const min = Math.min(...values);
  const max = Math.max(...values);
  const maxIdx = values.indexOf(max);
  const minIdx = values.indexOf(min);
  // 归一化到 viewBox y ∈ [4, 30]：底部留出空间给最低点的值标注，不被裁掉
  const yOf = (v: number) => (max === min ? 17 : 30 - ((v - min) / (max - min)) * 26);
  const xPct = (i: number) => (i / (points.length - 1)) * 100;
  const yPct = (v: number) => (yOf(v) / 40) * 100;
  const polyline = points
    .map((p, i) => `${xPct(i).toFixed(2)},${yOf(p.amountYuan).toFixed(2)}`)
    .join(" ");

  /** 标注的水平对齐：首点左对齐、末点右对齐、其余居中，避免标出容器外。 */
  const labelShiftX = (i: number) =>
    i === 0 ? "translateX(0)" : i === points.length - 1 ? "translateX(-100%)" : "translateX(-50%)";

  // 全持平（含全 0）时 max === min：一条水平线 + 中点单标注，两个极值重合只标一处
  const marks =
    max === min
      ? [{ i: Math.floor((points.length - 1) / 2), v: max, above: false }]
      : [
          { i: maxIdx, v: max, above: true },
          { i: minIdx, v: min, above: false },
        ];

  return (
    <div
      role="img"
      aria-label={
        max === min
          ? `${ariaLabel}，各点均为 ${max.toFixed(2)}`
          : `${ariaLabel}，最高 ${max.toFixed(2)}，最低 ${min.toFixed(2)}`
      }
      className={cn("relative w-full text-wallet", className)}
      style={{ height }}
    >
      <svg
        viewBox="0 0 100 40"
        preserveAspectRatio="none"
        aria-hidden
        className="h-full w-full"
      >
        <polyline
          points={polyline}
          fill="none"
          stroke="currentColor"
          strokeWidth={2}
          strokeLinecap="round"
          strokeLinejoin="round"
          vectorEffect="non-scaling-stroke"
        />
      </svg>

      {/* 极值圆点与值标注：SVG 被非等比拉伸，圆点画在 svg 里会变椭圆，故用 HTML 覆盖层。
          top 用 CSS max()/min() 收在容器内；水平对齐按首/中/末三点防越界。 */}
      {marks.map((m) => (
        <span
          key={`dot-${m.i}`}
          aria-hidden
          className="absolute size-2 rounded-full bg-current"
          style={{
            left: `${xPct(m.i)}%`,
            top: `${yPct(m.v)}%`,
            transform: "translate(-50%, -50%)",
            boxShadow: "0 0 0 2px var(--color-surface)",
          }}
        />
      ))}
      {marks.map((m) => (
        <span
          key={`label-${m.i}`}
          aria-hidden
          className="tabular-num absolute text-caption leading-none text-text-2"
          style={{
            left: `${xPct(m.i)}%`,
            top: m.above
              ? `max(0px, calc(${yPct(m.v)}% - 18px))`
              : `min(calc(100% - 14px), calc(${yPct(m.v)}% + 8px))`,
            transform: labelShiftX(m.i),
          }}
        >
          {m.v.toFixed(2)}
        </span>
      ))}
    </div>
  );
}
