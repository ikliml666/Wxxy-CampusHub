import { RefreshCw } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import { cn } from "@/shared/cn";
import { invokeCommand } from "@/shared/tauriApi";
import type { EcardSecurePad } from "@/shared/types";
/**
 * 通用安全键盘（M4.5 批 3）。挂载即调 `get_ecard_secure_keyboard` 取一把键盘，
 * 渲染 `keys` 为网格按钮，把用户点击的**位置下标序列**交给 `onDone`。
 *
 * # 安全红线（与电费充值 `RechargeFlow.passwordMap` 同构，逐条落在此文件）
 *
 * - **提交的永远是位置下标，不是字符**：`keys[i]` 只用于渲染按键文字（官方键盘本就
 *   可视），提交体只有 `padId + positions`；密码明文只由后端按「位置 → 字符」映射拼装。
 * - **绝不回显已输入的字符**：占位只显示 `•` 与位数；`aria-label` 只写位置序号
 *   （「第 3 键」），不写按键字符——防读屏读出密码内容。
 * - **不缓存不落盘**：`padId`/`keys`/已点下标只存活于本组件状态；`padId` 是一次性 id，
 *   键盘过期（提交过一次、超时）由用户点「重新获取键盘」换新，不写 localStorage。
 * - **布局指纹只用于本地一致性判定**：`keysFingerprint` 供父组件在「修改密码」场景
 *   比对两次新密码是否可比（同布局才可比）。它随键盘本身可视（按键字符就显示在屏幕
 *   上），但与下标组合即可还原密码，因此同样只存内存、不进日志/错误文案/localStorage。
 */

/** 键盘输入结果（原样回传给写操作命令；字段语义见文件头注）。 */
export interface KeypadInput {
  padId: string;
  positions: number[];
  keysFingerprint: string;
  /**
   * `plain` 模式：用户选择**系统键盘**直接输入明文（官方乱序键盘的反直觉体验，
   * 用户显式要求）。`padId`/`positions` 为空串/空数组，提交走对应的 `*_plain` 命令。
   */
  mode?: "positions" | "plain";
  /** plain 模式的明文密码（只在本机内存与 IPC 流转，不落盘不进日志） */
  plain?: string;
}

/**
 * 全键盘（91 键）按字符类型分桶，便于用户找字符。
 *
 * ⚠️ 每个键携带的 `i` 是**全局下标**——后端只按 `positions` 反查 `keys[i]`，
 * 分组纯粹是呈现层的分块，不改变提交语义。
 */
function standardGroups(
  keys: string[],
): { label: string; items: { k: string; i: number }[] }[] {
  const buckets = [
    { label: "数字", items: [] as { k: string; i: number }[] },
    { label: "大写字母", items: [] as { k: string; i: number }[] },
    { label: "小写字母", items: [] as { k: string; i: number }[] },
    { label: "符号", items: [] as { k: string; i: number }[] },
  ];
  keys.forEach((k, i) => {
    const idx = /[0-9]/.test(k) ? 0 : /[A-Z]/.test(k) ? 1 : /[a-z]/.test(k) ? 2 : 3;
    buckets[idx].items.push({ k, i });
  });
  return buckets.filter((b) => b.items.length > 0);
}

/**
 * 全键盘（91 键）的**分区 tab** 呈现：数字 / 大写 / 小写 / 符号 一次只显示一区，
 * 摊平 91 键会让弹层长到没法用（用户反馈「排版一点也不方便」）。
 * 分区只影响显示，键携带的仍是全局下标。查询密码默认是数字（身份证后六位），故首区「数字」。
 */
function StandardFullPad({
  pad,
  busy,
  keyCls,
  press,
  onBackspace,
}: {
  pad: EcardSecurePad;
  busy: boolean;
  keyCls: string;
  press: (i: number) => void;
  onBackspace: () => void;
}) {
  const groups = standardGroups(pad.keys);
  const [active, setActive] = useState(groups[0]?.label ?? "");
  const current = groups.find((g) => g.label === active) ?? groups[0];
  return (
    <div className="mt-3">
      <div role="tablist" aria-label="键盘分区" className="flex flex-wrap gap-1.5">
        {groups.map((g) => (
          <button
            key={g.label}
            type="button"
            role="tab"
            aria-selected={g.label === current?.label}
            disabled={busy}
            onClick={() => setActive(g.label)}
            className={cn(
              "rounded-control border px-2.5 py-1 text-caption transition-colors duration-[var(--dur-fast)] ease-out-soft",
              g.label === current?.label
                ? "border-[var(--color-wallet)] bg-[var(--color-wallet)] font-medium text-white"
                : "border-line bg-surface text-text-2 hover:border-line-strong",
            )}
          >
            {g.label}
            <span className="tabular-num ml-1 opacity-70">{g.items.length}</span>
          </button>
        ))}
      </div>
      <div className="mt-2 grid grid-cols-6 gap-1.5 sm:grid-cols-9">
        {(current?.items ?? []).map(({ k, i }) => (
          <button
            key={i}
            type="button"
            className={keyCls}
            disabled={busy}
            aria-label={`第 ${i + 1} 键`}
            onClick={() => press(i)}
          >
            {k}
          </button>
        ))}
      </div>
      <div className="mt-2 grid grid-cols-2 gap-1.5">
        <button
          type="button"
          className={cn(keyCls, "w-full")}
          disabled={busy}
          onClick={onBackspace}
        >
          删除
        </button>
        <button
          type="button"
          className={cn(keyCls, "w-full")}
          disabled={busy}
          onClick={() => setActive(groups[(groups.findIndex((g) => g.label === current?.label) + 1) % groups.length]?.label ?? "")}
        >
          下一分区
        </button>
      </div>
    </div>
  );
}

export function SecureKeypad({
  kind = "number",
  length = 6,
  title,
  busy = false,
  onDone,
  onCancel,
  error,
}: {
  /** 键盘类型：`number` 数字键盘（查询密码）；`standard` 全键盘（官方 standard 场景） */
  kind?: "number" | "standard";
  /** 需要输入的位数（点满自动 onDone） */
  length?: number;
  title: string;
  /** 提交进行中：禁用全部按键 */
  busy?: boolean;
  /** 点满 `length` 位自动回调（与官方「点满自动提交」一致） */
  onDone: (v: KeypadInput) => void;
  /** Esc / 取消按钮 */
  onCancel: () => void;
  /** 父组件回传的错误（如「密码错误」）；出现时清空已输入并提示换新键盘 */
  error?: string;
}) {
  const [pad, setPad] = useState<EcardSecurePad | null>(null);
  const [positions, setPositions] = useState<number[]>([]);
  const [phase, setPhase] = useState<"loading" | "ready" | "error">("loading");
  const [loadErr, setLoadErr] = useState("");
  /** 父组件错误与本地取键盘错误合并展示；重新获取键盘后清掉（已换新键盘，旧错误不再适用） */
  const [shownErr, setShownErr] = useState("");
  /** 系统键盘明文输入模式（用户显式切换；官方乱序键盘难以输入固定顺序密码） */
  const [altMode, setAltMode] = useState(false);
  const [plain, setPlain] = useState("");

  const refetch = useCallback(async () => {
    setPhase("loading");
    setLoadErr("");
    setShownErr("");
    setPositions([]);
    // 学校键盘接口偶发抖动（用户反馈「经常失败」）：失败自动重试一次，仍失败才进错误态
    let r = await invokeCommand<EcardSecurePad>("get_ecard_secure_keyboard", {
      kind,
    });
    if (!(r.success && r.data)) {
      r = await invokeCommand<EcardSecurePad>("get_ecard_secure_keyboard", {
        kind,
      });
    }
    if (r.success && r.data) {
      setPad(r.data);
      setPhase("ready");
    } else {
      setLoadErr(r.message ?? "安全键盘获取失败");
      setPhase("error");
    }
  }, [kind]);

  useEffect(() => {
    void refetch();
  }, [refetch]);

  // 父组件回传错误 ⇒ 清空已输入（旧键盘多半已被那次提交消耗，需换新再输）
  useEffect(() => {
    if (error) {
      setShownErr(error);
      setPositions([]);
    }
  }, [error]);

  // Esc 取消
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onCancel();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onCancel]);

  /** 点键：只记位置下标；点满自动提交。 */
  const press = (i: number) => {
    if (busy || phase !== "ready" || !pad || positions.length >= length) return;
    const next = [...positions, i];
    setPositions(next);
    if (next.length === length) {
      onDone({
        padId: pad.padId,
        positions: next,
        keysFingerprint: pad.keys.join("\u0000"),
      });
    }
  };

  const keyCls = cn(
    "tabular-num rounded-control border border-line bg-surface font-medium text-text",
    "transition-colors duration-[var(--dur-fast)] ease-out-soft hover:border-line-strong",
    "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--color-wallet)] focus-visible:ring-offset-1 focus-visible:ring-offset-bg",
    "disabled:pointer-events-none disabled:opacity-50",
    kind === "number" ? "h-11 text-title" : "h-9 min-w-0 px-1 text-caption",
  );

  const renderKey = (k: string, i: number) => (
    // 红线：aria-label 只写位置序号，绝不写按键字符（防读屏泄露密码内容）
    <button
      key={i}
      type="button"
      className={keyCls}
      disabled={busy}
      aria-label={`第 ${i + 1} 键`}
      onClick={() => press(i)}
    >
      {k}
    </button>
  );

  return (
    <div className="rounded-inner border border-line bg-surface-2 px-3 py-3">
      <div className="flex flex-wrap items-center justify-between gap-x-3 gap-y-1.5">
        <p className="text-body font-medium text-text">{title}</p>
        <div className="flex items-center gap-1">
          <Button
            variant="ghost"
            size="xs"
            disabled={busy}
            onClick={() => void refetch()}
          >
            <RefreshCw aria-hidden className="size-3" />
            重新获取键盘
          </Button>
          <Button
            variant="ghost"
            size="xs"
            disabled={busy}
            onClick={() => {
              setAltMode((m) => !m);
              setPlain("");
            }}
          >
            {altMode ? "用安全键盘" : "用系统键盘"}
          </Button>
          <Button variant="ghost" size="xs" disabled={busy} onClick={onCancel}>
            取消
          </Button>
        </div>
      </div>
      <p className="mt-1 text-caption text-text-2">
        已输入 <span className="tabular-num">{positions.length}</span> / {length}{" "}
        位，点满自动提交；按 Esc 取消
      </p>

      {/* 占位：只显示 • 与位数，绝不回显真实字符 */}
      <div aria-hidden className="mt-2 flex flex-wrap items-center gap-1.5">
        {Array.from({ length }, (_, i) => (
          <span
            key={i}
            className={cn(
              "text-title leading-none",
              i < positions.length
                ? "text-[var(--color-wallet)]"
                : "text-[var(--color-line)]",
            )}
          >
            •
          </span>
        ))}
      </div>

      {(shownErr || loadErr) && (
        <p className="mt-2 text-caption text-alert">{shownErr || loadErr}</p>
      )}

      {altMode ? (
        <div className="mt-3">
          <input
            type="password"
            className="h-10 w-full rounded-control border border-line bg-surface px-3 text-body text-text focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--color-wallet)]"
            value={plain}
            disabled={busy}
            placeholder={`输入 ${length} 位密码`}
            autoComplete="off"
            onChange={(e) => setPlain(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && plain.length >= 4 && !busy) {
                onDone({
                  mode: "plain",
                  padId: "",
                  positions: [],
                  keysFingerprint: "",
                  plain,
                });
              }
            }}
          />
          <Button
            className="mt-2 w-full"
            size="sm"
            disabled={busy || plain.trim().length < 4}
            onClick={() =>
              onDone({
                mode: "plain",
                padId: "",
                positions: [],
                keysFingerprint: "",
                plain,
              })
            }
          >
            提交密码
          </Button>
          <p className="mt-1.5 text-caption text-text-2">
            明文仅在本机内存中拼装提交，不经日志；官方乱序键盘无法输入时用这个。
          </p>
        </div>
      ) : (
        <>
      {phase === "loading" && (
        <p className="mt-3 text-caption text-text-2" aria-busy>
          正在获取安全键盘…
        </p>
      )}

      {phase === "error" && (
        <div className="mt-2">
          <Button variant="outline" size="sm" onClick={() => void refetch()}>
            重试获取键盘
          </Button>
        </div>
      )}

      {phase === "ready" && pad && kind === "number" && (
        <div className="mt-3 grid max-w-[17rem] grid-cols-3 gap-1.5">
          {/* 官方数字键盘布局：前 9 键 3×3，第 10 键前留一空格，末格删除 */}
          {pad.keys.slice(0, 9).map((k, i) => renderKey(k, i))}
          {pad.keys.length > 9 && <span aria-hidden />}
          {pad.keys.slice(9).map((k, i) => renderKey(k, 9 + i))}
          <button
            type="button"
            className={keyCls}
            disabled={busy}
            onClick={() => setPositions((s) => s.slice(0, -1))}
          >
            删除
          </button>
        </div>
      )}

      {phase === "ready" && pad && kind !== "number" && (
        <StandardFullPad pad={pad} busy={busy} keyCls={keyCls} press={press} onBackspace={() => setPositions((s) => s.slice(0, -1))} />
      )}

      {phase === "ready" && positions.length > 0 && (
        <div className="mt-2">
          <Button
            variant="ghost"
            size="xs"
            disabled={busy}
            onClick={() => setPositions([])}
          >
            清空重输
          </Button>
        </div>
      )}
        </>
      )}
    </div>
  );
}
