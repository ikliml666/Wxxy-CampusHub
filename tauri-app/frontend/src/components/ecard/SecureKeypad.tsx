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

  const refetch = useCallback(async () => {
    setPhase("loading");
    setLoadErr("");
    setShownErr("");
    setPositions([]);
    const r = await invokeCommand<EcardSecurePad>("get_ecard_secure_keyboard", {
      kind,
    });
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

      {phase === "ready" && pad && (
        <div
          className={cn(
            "mt-3 grid gap-1.5",
            kind === "number"
              ? "max-w-[17rem] grid-cols-3"
              : "grid-cols-4 sm:grid-cols-6",
          )}
        >
          {kind === "number" ? (
            <>
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
            </>
          ) : (
            <>
              {pad.keys.map((k, i) => renderKey(k, i))}
              <button
                type="button"
                className={cn(keyCls, "col-span-2")}
                disabled={busy}
                onClick={() => setPositions((s) => s.slice(0, -1))}
              >
                删除
              </button>
            </>
          )}
        </div>
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
    </div>
  );
}
