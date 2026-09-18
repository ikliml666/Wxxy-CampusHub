import { useEffect, useRef, useState } from "react";
import { motion, useReducedMotion } from "framer-motion";
import { LogIn, RefreshCw, X } from "lucide-react";
import { invokeCommand } from "@/shared/tauriApi";
import type { CommandResult } from "@/shared/types";
import { isCaptchaPayload, useAuthStore } from "@/stores/authStore";
import type { CaptchaPayload } from "@/stores/authStore";
import { useUiStore } from "@/stores/uiStore";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";

// 状态机：idle → logging-in（loading）→ success（关弹窗）
//   → manual（CAPTCHA_MANUAL，手动验证码 + get_captcha 换新图）/ error（红字提示）
type LoginPhase = "idle" | "logging-in" | "manual" | "error";

/** 取一张新验证码（get_captcha → { uid, pngBase64 }）；失败返回 null */
async function fetchCaptcha(): Promise<CaptchaPayload | null> {
  const r = await invokeCommand<CaptchaPayload>("get_captcha");
  return r.success && r.data ? r.data : null;
}

/** CAPTCHA_MANUAL 结果处理：优先 get_captcha 取新图（失败尝试已消耗旧 uid），
 *  响应自带 payload 作兜底；两者皆无 → null */
function manualPayloadFrom(
  r: CommandResult<unknown>,
  fresh: CaptchaPayload | null,
): CaptchaPayload | null {
  if (fresh) return fresh;
  return r.data && isCaptchaPayload(r.data) ? r.data : null;
}

export function LoginDialog() {
  const open = useUiStore((s) => s.loginDialogOpen);
  const closeLoginDialog = useUiStore((s) => s.closeLoginDialog);
  const accounts = useAuthStore((s) => s.accounts);
  const refreshAccounts = useAuthStore((s) => s.refreshAccounts);
  const login = useAuthStore((s) => s.login);
  const loginManual = useAuthStore((s) => s.loginManual);
  const loginSaved = useAuthStore((s) => s.loginSaved);

  const [phase, setPhase] = useState<LoginPhase>("idle");
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState(""); // 仅表单 state，绝不进 localStorage
  const [captchaCode, setCaptchaCode] = useState("");
  const [captcha, setCaptcha] = useState<CaptchaPayload | null>(null);
  const [errorMessage, setErrorMessage] = useState<string | null>(null);
  const [pendingSaved, setPendingSaved] = useState<string | null>(null);

  const usernameRef = useRef<HTMLInputElement>(null);
  const reduceMotion = useReducedMotion();
  const logging = phase === "logging-in" || pendingSaved !== null;

  // 打开：重置上次残留 + 兜底刷新账号列表 + 聚焦学号框
  useEffect(() => {
    if (!open) return;
    setPhase("idle");
    setErrorMessage(null);
    setCaptcha(null);
    setCaptchaCode("");
    setPassword("");
    void refreshAccounts();
    const t = window.setTimeout(() => usernameRef.current?.focus(), 0);
    return () => window.clearTimeout(t);
  }, [open, refreshAccounts]);

  // Esc 关闭 + 弹层期间锁背景滚动
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") closeLoginDialog();
    };
    window.addEventListener("keydown", onKey);
    const prevOverflow = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    return () => {
      window.removeEventListener("keydown", onKey);
      document.body.style.overflow = prevOverflow;
    };
  }, [open, closeLoginDialog]);

  if (!open) return null;

  // 登录系命令结果统一收口：成功 → 关弹窗；CAPTCHA_MANUAL → 手动模式；其余 → 红字
  const applyAuthResult = async (r: CommandResult<unknown>, fromManual: boolean) => {
    if (r.success) {
      setPassword("");
      setPhase("idle");
      closeLoginDialog();
      return;
    }
    if (r.message === "CAPTCHA_MANUAL") {
      const fresh = await fetchCaptcha();
      const payload = manualPayloadFrom(r, fresh);
      if (payload) {
        setCaptcha(payload);
        setCaptchaCode("");
        setErrorMessage(fromManual ? "验证码不正确，请重新输入" : null);
        setPhase("manual");
        return;
      }
      setErrorMessage("验证码获取失败，请稍后重试");
      setPhase("error");
      return;
    }
    setErrorMessage(r.message ?? "登录失败，请稍后重试");
    setPhase("error");
  };

  const handleLogin = async () => {
    if (logging) return;
    if (!username.trim() || !password) {
      setErrorMessage("请输入学号和密码");
      setPhase("error");
      return;
    }
    setErrorMessage(null);
    setPhase("logging-in");
    const r = await login(username.trim(), password);
    await applyAuthResult(r, false);
  };

  const handleManualRetry = async () => {
    if (logging || !captcha) return;
    if (!captchaCode.trim()) {
      setErrorMessage("请输入验证码答案");
      setPhase("manual");
      return;
    }
    setErrorMessage(null);
    setPhase("logging-in");
    const r = await loginManual({
      username: username.trim(),
      password,
      captchaUid: captcha.uid,
      captchaCode: captchaCode.trim(),
    });
    await applyAuthResult(r, true);
  };

  // 换一张验证码（get_captcha）
  const handleRefreshCaptcha = async () => {
    const p = await fetchCaptcha();
    if (p) {
      setCaptcha(p);
      setCaptchaCode("");
    } else {
      setErrorMessage("验证码获取失败，请稍后重试");
      setPhase("error");
    }
  };

  // 已保存账号免密登录；若也被要求验证码则回落到表单手动模式
  const handleSavedLogin = async (saved: string) => {
    if (logging) return;
    setErrorMessage(null);
    setPendingSaved(saved);
    const r = await loginSaved(saved);
    if (r.success) {
      setPendingSaved(null);
      closeLoginDialog();
      return;
    }
    setPendingSaved(null);
    if (r.message === "CAPTCHA_MANUAL") {
      const fresh = await fetchCaptcha();
      const payload = manualPayloadFrom(r, fresh);
      if (payload) {
        if (!username.trim()) setUsername(saved);
        setCaptcha(payload);
        setCaptchaCode("");
        setPhase("manual");
        return;
      }
    }
    setErrorMessage(r.message ?? "登录失败，请稍后重试");
    setPhase("error");
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 px-4"
      onMouseDown={(e) => {
        // 只在点遮罩空白处关闭（点卡片不关）
        if (e.target === e.currentTarget) closeLoginDialog();
      }}
    >
      <motion.div
        role="dialog"
        aria-modal="true"
        aria-label="登录锡院助手"
        initial={reduceMotion ? { opacity: 0 } : { opacity: 0, y: 6, scale: 0.98 }}
        animate={{ opacity: 1, y: 0, scale: 1 }}
        transition={
          reduceMotion
            ? { duration: 0.12 }
            : { duration: 0.18, ease: [0.22, 1, 0.36, 1] }
        }
        className="relative w-full max-w-sm rounded-card border border-line bg-surface p-5 shadow-pop"
      >
        <button
          type="button"
          aria-label="关闭登录弹窗"
          onClick={closeLoginDialog}
          className="absolute right-3 top-3 flex size-7 items-center justify-center rounded-full text-text-2 transition-colors duration-[var(--dur-fast)] hover:bg-surface-2 hover:text-text"
        >
          <X className="size-4" aria-hidden="true" />
        </button>

        <div className="mb-4 flex items-center gap-3">
          <span
            aria-hidden="true"
            className="flex size-7 shrink-0 select-none items-center justify-center rounded-[9px] text-[15px] font-bold text-white"
            style={{
              backgroundImage:
                "linear-gradient(140deg, var(--color-brand), var(--color-info))",
            }}
          >
            锡
          </span>
          <div>
            <h2 className="text-title font-semibold text-text">登录锡院助手</h2>
            <p className="text-caption text-text-2">使用校园门户账号登录</p>
          </div>
        </div>

        <form
          className="space-y-3"
          onSubmit={(e) => {
            e.preventDefault();
            void handleLogin();
          }}
        >
          <label className="block">
            <span className="mb-1 block text-caption text-text-2">学号</span>
            <Input
              ref={usernameRef}
              value={username}
              onChange={(e) => setUsername(e.target.value)}
              autoComplete="username"
              placeholder="请输入学号"
              disabled={logging}
            />
          </label>
          <label className="block">
            <span className="mb-1 block text-caption text-text-2">密码</span>
            <Input
              type="password"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              autoComplete="current-password"
              placeholder="请输入密码"
              disabled={logging}
            />
          </label>

          {phase === "manual" && captcha && (
            <div className="space-y-2 rounded-inner border border-line bg-surface-2 p-3">
              <div className="flex items-center justify-between">
                <span className="text-caption text-text-2">验证码</span>
                <button
                  type="button"
                  aria-label="换一张验证码"
                  disabled={logging}
                  onClick={() => void handleRefreshCaptcha()}
                  className="flex items-center gap-1 rounded-control px-1 text-caption text-text-2 transition-colors duration-[var(--dur-fast)] hover:text-text disabled:pointer-events-none disabled:opacity-50"
                >
                  <RefreshCw className="size-3.5" aria-hidden="true" />
                  换一张
                </button>
              </div>
              <div className="flex items-center gap-3">
                <img
                  src={`data:image/png;base64,${captcha.pngBase64}`}
                  alt="登录验证码"
                  className="rounded-control border border-line bg-white"
                />
                <Input
                  value={captchaCode}
                  onChange={(e) => setCaptchaCode(e.target.value)}
                  placeholder="算式答案"
                  aria-label="验证码答案"
                  className="flex-1"
                  disabled={logging}
                />
              </div>
              <Button
                type="button"
                variant="outline"
                size="sm"
                className="w-full"
                disabled={logging}
                aria-busy={logging}
                onClick={() => void handleManualRetry()}
              >
                重试
              </Button>
              <p className="text-caption text-text-2">
                自动识别验证码未成功，请输入图片中算式的答案。
              </p>
            </div>
          )}

          {errorMessage && phase !== "logging-in" && (
            <p className="text-body text-alert" role="alert" aria-live="polite">
              {errorMessage}
            </p>
          )}

          <Button
            type="submit"
            className="w-full"
            disabled={logging}
            aria-busy={phase === "logging-in"}
          >
            <LogIn className="size-4" aria-hidden="true" />
            {phase === "logging-in" ? "正在识别验证码…" : "登录"}
          </Button>
        </form>

        {accounts.length > 0 && (
          <section className="mt-4 border-t border-line pt-3">
            <p className="mb-2 text-caption font-medium text-text-2">
              已保存账号 · 点击免密登录
            </p>
            <div className="flex flex-wrap gap-1.5">
              {accounts.map((account) => (
                <button
                  key={account.username}
                  type="button"
                  disabled={logging}
                  aria-busy={pendingSaved === account.username}
                  onClick={() => void handleSavedLogin(account.username)}
                  title={account.lastLogin}
                  className="rounded-full border border-line bg-surface-2 px-3 py-1.5 text-caption text-text transition-colors duration-[var(--dur-fast)] ease-out-soft hover:border-brand hover:text-brand disabled:pointer-events-none disabled:opacity-50"
                >
                  {pendingSaved === account.username
                    ? "登录中…"
                    : (account.displayName ?? account.username)}
                </button>
              ))}
            </div>
          </section>
        )}
      </motion.div>
    </div>
  );
}
