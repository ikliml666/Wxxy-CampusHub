import { RefreshCw } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import { cn } from "@/shared/cn";
import { invokeCommand } from "@/shared/tauriApi";
import type { EcardSecurePad } from "@/shared/types";
/**
 * 通用安全键盘（M4.5 批 3；2026-09-20 修正键面形态）。挂载即调
 * `get_ecard_secure_keyboard` 取一把键盘，渲染为官方同款**图片九宫格**，
 * 把用户点击的**位置下标序列**交给 `onDone`。
 *
 * # 协议关键（2026-09-20 官方逆向实锤，推翻旧版「显示 keys 字符」的实现）
 *
 * `keyboard` 接口 `keys`（numberKeyboard）是**伪字符数组**（如 `PeEpv7_iR\`），
 * 与用户想输的数字无关；`images[i]`（numberKeyboardImage，每键一张 PNG）画的才是
 * 用户要按的数字。服务端按 uuid 批次维护「数字 ↔ 伪字符」映射：用户点图片数字
 * （位置 i）→ 提交 positions → 后端按位置取 `keys[i]`（伪字符）拼 pwd → 服务端
 * 还原出数字校验。键面因此**绝不能显示 keys 字符**（用户会看到一堆与密码无关的
 * 字母符号、无从输入——正是「密码一直错误」的根因），必须渲染 images。
 *
 * # 安全红线
 *
 * - **提交的永远是位置下标，不是字符**：密码明文只由后端按「位置 → 伪字符」拼装。
 * - **绝不回显已输入内容**：占位只显示 `•` 与位数；`aria-label` 只写位置序号
 *   （「第 3 键」），不写按键字符——防读屏读出密码内容。
 * - **不缓存不落盘**：`padId`/已点下标只存活于本组件状态；`padId` 一次性，键盘过期
 *   由用户点「重新获取键盘」换新，不写 localStorage。
 * - 系统键盘明文直输在协议上**不可行**（明文脱离批次映射，服务端还原必失败——实测
 *   60005），故不提供该模式；图片九宫格即官方输入形态。
 */

/** 键盘输入结果（原样回传给写操作命令；字段语义见文件头注）。 */
export interface KeypadInput {
  padId: string;
  positions: number[];
  keysFingerprint: string;
}

export function SecureKeypad({
  length = 6,
  title,
  busy = false,
  onDone,
  onCancel,
  error,
}: {
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
    // 学校键盘接口偶发抖动（用户反馈「经常失败」）：失败自动重试一次，仍失败才进错误态
    let r = await invokeCommand<EcardSecurePad>("get_ecard_secure_keyboard", {
      kind: "number",
    });
    if (!(r.success && r.data)) {
      r = await invokeCommand<EcardSecurePad>("get_ecard_secure_keyboard", {
        kind: "number",
      });
    }
    if (r.success && r.data) {
      setPad(r.data);
      setPhase("ready");
    } else {
      setLoadErr(r.message ?? "安全键盘获取失败");
      setPhase("error");
    }
  }, []);

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
    "rounded-control border border-line bg-surface",
    "transition-colors duration-[var(--dur-fast)] ease-out-soft hover:border-line-strong",
    "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--color-wallet)] focus-visible:ring-offset-1 focus-visible:ring-offset-bg",
    "disabled:pointer-events-none disabled:opacity-50",
  );

  /** 官方图片键面：键面画的是数字（服务端批次映射），提交的是该键位置下标。 */
  const renderKey = (i: number) => (
    // 红线：aria-label 只写位置序号，绝不写按键字符（防读屏泄露密码内容）
    <button
      key={i}
      type="button"
      className={cn(keyCls, "flex h-11 items-center justify-center")}
      disabled={busy}
      aria-label={`第 ${i + 1} 键`}
      onClick={() => press(i)}
    >
      <img
        src={`data:image/png;base64,${pad?.images[i] ?? ""}`}
        alt=""
        aria-hidden
        className="h-7 w-7 object-contain"
        draggable={false}
      />
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
        <div className="mt-3 grid max-w-[17rem] grid-cols-3 gap-1.5">
          {/* 官方数字键盘布局：前 9 键 3×3，第 10 键前留一空格，末格删除 */}
          {pad.keys.slice(0, 9).map((_, i) => renderKey(i))}
          {pad.keys.length > 9 && <span aria-hidden />}
          {pad.keys.slice(9).map((_, i) => renderKey(9 + i))}
          <button
            type="button"
            className={cn(keyCls, "h-11 text-caption font-medium text-text")}
            disabled={busy}
            onClick={() => setPositions((s) => s.slice(0, -1))}
          >
            删除
          </button>
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
