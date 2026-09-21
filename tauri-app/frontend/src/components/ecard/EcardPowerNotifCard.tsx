import { useEffect, useState } from "react";
import { BellElectric, RefreshCw } from "lucide-react";
import { Surface } from "@/components/Surface";
import { Button } from "@/components/ui/button";
import { getNotificationSettings, saveNotificationSettings } from "@/shared/tauriApi";
import type { NotificationSettings } from "@/shared/types";
import { cn } from "@/shared/cn";

/** 小号开关：轨道纯色瞬时切换，滑块只做 transform 过渡（与 SettingsPanel 同款）。 */
function ToggleSwitch({
  checked,
  onToggle,
  ariaLabel,
}: {
  checked: boolean;
  onToggle: () => void;
  ariaLabel: string;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={ariaLabel}
      onClick={onToggle}
      className={cn(
        "relative h-6 w-10 shrink-0 rounded-full border transition-colors duration-[var(--dur-fast)] ease-out-soft",
        checked ? "border-brand bg-brand" : "border-line bg-surface-2 hover:border-line-strong",
      )}
    >
      <span
        aria-hidden
        className={cn(
          "absolute top-1/2 left-[3px] size-4 -translate-y-1/2 rounded-full bg-surface shadow-card transition-transform duration-[var(--dur-base)] ease-out-soft",
          checked && "translate-x-[18px]",
        )}
      />
    </button>
  );
}

/**
 * 电费通知设置卡（设置页 → 一卡通电费子页迁移）：开关 + 阈值 + 检查频率只读。
 * 保存契约与 SettingsPanel 原实现一致：阈值非数字 / 负数前端校验拦截，
 * 合法才整包 saveNotificationSettings 落盘（其余字段原样回传）。
 */
export function EcardPowerNotifCard() {
  const [phase, setPhase] = useState<"loading" | "ready" | "error">("loading");
  const [settings, setSettings] = useState<NotificationSettings | null>(null);
  const [elecOn, setElecOn] = useState(true);
  const [thresholdText, setThresholdText] = useState("10");
  const [saving, setSaving] = useState(false);
  const [msg, setMsg] = useState<{ kind: "ok" | "err"; text: string } | null>(null);
  const [reloadTick, setReloadTick] = useState(0);

  useEffect(() => {
    let alive = true;
    setPhase("loading");
    setMsg(null);
    getNotificationSettings().then((r) => {
      if (!alive) return;
      if (r.success && r.data) {
        setSettings(r.data);
        setElecOn(r.data.electricityEnabled);
        setThresholdText(String(r.data.electricityThresholdYuan));
        setPhase("ready");
      } else {
        setPhase("error");
      }
    });
    return () => {
      alive = false;
    };
  }, [reloadTick]);

  if (phase === "loading") {
    return (
      <Surface className="px-4 py-4">
        <p className="text-body font-medium text-text">电费通知</p>
        <div aria-hidden className="mt-3 space-y-2">
          {[0, 1, 2].map((i) => (
            <span key={i} className="block h-9 animate-pulse rounded bg-line" />
          ))}
        </div>
      </Surface>
    );
  }

  if (phase === "error") {
    return (
      <Surface className="px-4 py-4">
        <p className="text-body font-medium text-text">电费通知</p>
        <div className="mt-3 flex items-center justify-between gap-3 rounded-inner border border-line bg-surface-2 px-3 py-2.5">
          <p className="text-caption text-text-2">电费通知设置获取失败</p>
          <Button variant="outline" size="sm" onClick={() => setReloadTick((t) => t + 1)}>
            <RefreshCw aria-hidden className="size-3.5" />
            重试
          </Button>
        </div>
      </Surface>
    );
  }

  const toggleElec = () => {
    setElecOn((v) => !v);
    setMsg(null);
  };

  const save = () => {
    if (!settings) return;
    const raw = thresholdText.trim();
    const t = Number(raw);
    if (raw === "" || !Number.isFinite(t)) {
      setMsg({ kind: "err", text: "电费提醒阈值必须是数字" });
      return;
    }
    if (t < 0) {
      setMsg({ kind: "err", text: "电费提醒阈值不能为负数" });
      return;
    }
    const next: NotificationSettings = {
      ...settings,
      electricityEnabled: elecOn,
      electricityThresholdYuan: t,
    };
    setSaving(true);
    setMsg(null);
    saveNotificationSettings(next)
      .then((r) => {
        if (r.success) {
          setSettings(next);
          setMsg({ kind: "ok", text: "电费通知设置已保存" });
        } else {
          setMsg({ kind: "err", text: r.message ?? "保存通知设置失败" });
        }
      })
      .finally(() => setSaving(false));
  };

  return (
    <Surface accent="wallet" className="px-4 py-4">
      <div className="flex items-center gap-2">
        <BellElectric aria-hidden className="size-4 text-wallet" />
        <p className="text-body font-medium text-text">电费通知</p>
      </div>
      <div className="mt-3 space-y-2">
        <div className="flex items-center justify-between gap-3 rounded-inner border border-line bg-surface-2 px-3 py-2.5">
          <div className="min-w-0">
            <p className="text-body text-text">电费提醒</p>
            <p className="text-caption text-text-2">绑定宿舍余额低于阈值时提醒（每天最多一次）</p>
          </div>
          <ToggleSwitch ariaLabel="电费提醒" checked={elecOn} onToggle={toggleElec} />
        </div>
        <div className="flex items-center justify-between gap-3 rounded-inner border border-line bg-surface-2 px-3 py-2.5">
          <div className="min-w-0">
            <p className="text-body text-text">电费提醒阈值</p>
            <p className="text-caption text-text-2">余额低于该值时提醒（元）</p>
          </div>
          <input
            type="number"
            min={0}
            step={0.5}
            value={thresholdText}
            onChange={(e) => setThresholdText(e.target.value)}
            aria-label="电费提醒阈值（元）"
            className="tabular-num w-24 shrink-0 rounded-control border border-line bg-surface px-2 py-1.5 text-right text-body text-text outline-none transition-colors duration-[var(--dur-fast)] ease-out-soft focus:border-brand"
          />
        </div>
        {settings && (
          <p className="tabular-num px-1 text-caption text-text-2">
            检查频率：每 {settings.electricityIntervalMin} 分钟
          </p>
        )}
        {msg && (
          <p
            role={msg.kind === "err" ? "alert" : "status"}
            className={cn("px-1 text-caption", msg.kind === "ok" ? "text-wallet" : "text-alert")}
          >
            {msg.text}
          </p>
        )}
        <div className="flex justify-end pt-1">
          <Button size="sm" disabled={saving} aria-busy={saving} onClick={save}>
            {saving ? "保存中…" : "保存电费通知"}
          </Button>
        </div>
      </div>
    </Surface>
  );
}
