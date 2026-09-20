import JsBarcode from "jsbarcode";
import { RefreshCw } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import QRCode from "react-qr-code";
import { Surface } from "@/components/Surface";
import { Button } from "@/components/ui/button";
import { invokeCommand } from "@/shared/tauriApi";
import type { EcardPaycode, EcardPaycodeSettings } from "@/shared/types";

/**
 * 付款码子页（一期，对齐官方 H5 plat/pay）：
 * 条码（canvas）+ 二维码（同串）+ 支付方式/余额 + 有效期倒计时（归零自动重取）
 * + 手动刷新 + 脱机二维码开关状态（只读展示，写操作下一批）。
 *
 * ⚠️ 编码格式待真机扫码验证：一期选 CODE128（JsBarcode 缺省）；POS 不识别时改
 * ITF / Code39 只需改下方 format 一行参数。
 *
 * 凭据红线：`barcode` 数字串是动态支付凭据——不写 console.log、不进 localStorage、
 * 不进错误文案 / aria-label；除条码图与「查看数字」主动展开的展示外不留存。
 */

/** expires 兼容判定：>1e9 当秒级时间戳（绝对时刻），否则当「有效期秒数」；<=0 = 无效。 */
function expiryAtMs(expires: number, now: number): number | null {
  if (expires > 1_000_000_000) return expires * 1000;
  if (expires > 0) return now + expires * 1000;
  return null;
}

/** 倒计时 mm:ss。 */
function fmtMmSs(total: number): string {
  const s = Math.max(0, total);
  return `${String(Math.floor(s / 60)).padStart(2, "0")}:${String(s % 60).padStart(2, "0")}`;
}

/** 数字串 4 位一组便于核对（仅展示排版，不改凭据内容）。 */
const group4 = (s: string) => s.replace(/(.{4})/g, "$1 ").trim();

export function EcardPaycodeView() {
  const [phase, setPhase] = useState<"loading" | "ready" | "error">("loading");
  const [data, setData] = useState<EcardPaycode | null>(null);
  /** 脱机开关：null = 状态获取失败（如实显示「未知」，不猜）。 */
  const [offline, setOffline] = useState<boolean | null>(null);
  const [errMsg, setErrMsg] = useState("");
  const [busy, setBusy] = useState(false);
  /** 「查看数字」展开态；收起即消失，不持久化。 */
  const [showDigits, setShowDigits] = useState(false);
  /** 倒计时剩余秒数；null = expires 无效，不倒计时。 */
  const [remaining, setRemaining] = useState<number | null>(null);
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const expiresAtRef = useRef(0);
  const loadingRef = useRef(false);

  const load = useCallback(async () => {
    if (loadingRef.current) return;
    loadingRef.current = true;
    setBusy(true);
    // 取码与设置并行取；设置失败不阻塞取码（该行降级显示「状态未知」）
    const [pc, st] = await Promise.all([
      invokeCommand<EcardPaycode>("get_ecard_paycode"),
      invokeCommand<EcardPaycodeSettings>("get_ecard_paycode_settings"),
    ]);
    if (pc.success && pc.data) {
      setData(pc.data);
      const at = expiryAtMs(pc.data.expires, Date.now());
      expiresAtRef.current = at ?? 0;
      setRemaining(at === null ? null : Math.max(0, Math.round((at - Date.now()) / 1000)));
      setPhase("ready");
      setShowDigits(false);
      setErrMsg("");
    } else {
      setPhase("error");
      setErrMsg(pc.message ?? "付款码获取失败");
    }
    setOffline(st.success && st.data ? st.data.offlineSwitch : null);
    setBusy(false);
    loadingRef.current = false;
  }, []);

  // 挂载即取
  useEffect(() => {
    void load();
  }, [load]);

  // 倒计时：每秒刷新；归零自动重取（expires 无效时不重取，防请求风暴）
  useEffect(() => {
    if (phase !== "ready") return;
    const timer = setInterval(() => {
      if (expiresAtRef.current <= 0) return;
      const rem = Math.max(0, Math.round((expiresAtRef.current - Date.now()) / 1000));
      setRemaining(rem);
      if (rem <= 0) void load();
    }, 1000);
    return () => clearInterval(timer);
  }, [phase, load]);

  // 条码绘制：绘制失败不阻塞页面（二维码独立渲染同串，仍可用）
  useEffect(() => {
    if (phase !== "ready" || !data || !canvasRef.current) return;
    try {
      JsBarcode(canvasRef.current, data.barcode, {
        format: "CODE128", // 编码格式待真机扫码验证；POS 不识别时改 ITF / Code39（一行参数）
        displayValue: false,
        height: 72,
        width: 1.8,
        margin: 8,
      });
    } catch {
      // 无效输入等绘制异常：条码图缺席即可，不把凭据内容带进任何提示
    }
  }, [phase, data]);

  if (phase === "loading") {
    return (
      <Surface className="px-4 py-4">
        <div aria-hidden>
          <div className="h-4 w-44 animate-pulse rounded bg-line" />
          <div className="mx-auto mt-4 h-24 w-72 animate-pulse rounded bg-line" />
          <div className="mx-auto mt-3 size-28 animate-pulse rounded bg-line" />
        </div>
      </Surface>
    );
  }

  if (phase === "error" || !data) {
    return (
      <Surface accent="wallet" className="flex items-center justify-between gap-3 px-4 py-3">
        <p className="min-w-0 text-body text-text-2">获取失败：{errMsg}</p>
        <Button variant="outline" size="sm" className="shrink-0" onClick={() => void load()}>
          重试
        </Button>
      </Surface>
    );
  }

  return (
    <div className="flex flex-col gap-3">
      <Surface accent="wallet" className="px-4 py-4">
        {/* 支付方式与余额（官方样式：「一卡通电子钱包 · ¥70.46」） */}
        <p className="text-body font-medium text-text">
          {data.payName} · ¥ {data.balanceYuan.toFixed(2)}
        </p>

        {/* 条码：线下扫码要求白底黑条，容器固定白底（不受暗色主题影响） */}
        <div className="mt-3 flex justify-center rounded-inner bg-white px-3 py-3">
          <canvas
            ref={canvasRef}
            className="h-auto max-w-full"
            role="img"
            aria-label="付款条码"
          />
        </div>

        {/* 查看数字：凭据数字串仅用户主动展开时可见 */}
        <div className="mt-1 flex justify-center">
          <Button variant="ghost" size="sm" onClick={() => setShowDigits((v) => !v)}>
            {showDigits ? "收起数字" : "查看数字"}
          </Button>
        </div>
        {showDigits && (
          <p className="tabular-num mt-1 text-center text-title tracking-wider text-text">
            {group4(data.barcode)}
          </p>
        )}

        {/* 二维码：同一串内容，枪扫条码 / 手机扫二维码均可 */}
        <div
          className="mt-3 flex justify-center rounded-inner bg-white p-3"
          role="img"
          aria-label="付款二维码"
        >
          <QRCode value={data.barcode} size={160} />
        </div>

        {/* 有效期 + 手动刷新 */}
        <div className="mt-3 flex items-center justify-between gap-3">
          <p className="text-caption text-text-2">
            {remaining === null
              ? "有效期未知，请手动刷新"
              : `有效期 ${fmtMmSs(remaining)}`}
          </p>
          <Button variant="outline" size="sm" disabled={busy} onClick={() => void load()}>
            <RefreshCw aria-hidden className="size-3.5" />
            {busy ? "刷新中…" : "手动刷新"}
          </Button>
        </div>
      </Surface>

      <Surface className="px-4 py-3">
        <div className="flex items-center justify-between gap-3">
          <p className="text-body text-text">脱机二维码</p>
          <span className="text-caption text-text-2">
            {offline === null ? "状态未知" : offline ? "已开启" : "未开启"}
          </span>
        </div>
        <p className="mt-1 text-caption text-text-2">
          学校侧开关状态，仅展示；切换操作下一批提供。
        </p>
      </Surface>
    </div>
  );
}
