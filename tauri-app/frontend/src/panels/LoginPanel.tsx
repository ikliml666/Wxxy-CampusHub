import { useEffect, useState } from "react";
import { invokeCommand } from "@/shared/tauriApi";
import type { CommandResult } from "@/shared/types";
import { useUiStore } from "@/stores/uiStore";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";

// 状态机：idle → logging-in（loading）→ success（onLoggedIn 切 AppShell，面板即卸载）
//   → manual（CAPTCHA_MANUAL，手动验证码）/ error（红字提示）
type LoginPhase = "idle" | "logging-in" | "manual" | "error";

interface AuthOk {
  username: string;
  displayName: string;
}

interface CaptchaData {
  uid: string;
  pngBase64: string; // 裸 base64，前端拼 data URI
}

interface SavedAccount {
  username: string;
  lastLogin?: string;
}

export function LoginPanel({ onLoggedIn }: { onLoggedIn: () => void }) {
  const [phase, setPhase] = useState<LoginPhase>("idle");
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState(""); // 仅表单 state，绝不进 localStorage
  const [captchaCode, setCaptchaCode] = useState("");
  const [captcha, setCaptcha] = useState<CaptchaData | null>(null);
  const [errorMessage, setErrorMessage] = useState<string | null>(null);
  const [accounts, setAccounts] = useState<SavedAccount[]>([]);
  const setDisplayName = useUiStore((s) => s.setDisplayName);

  const logging = phase === "logging-in";

  // 已保存账号：mount 时拉取；空列表或命令失败 → 列表为空 → 整块隐藏（无占位）
  useEffect(() => {
    invokeCommand<{ accounts: SavedAccount[] }>("list_accounts").then((r) => {
      if (r.success && r.data) setAccounts(r.data.accounts);
    });
  }, []);

  // CAPTCHA_MANUAL 分支的 data 是验证码形态，其余为 AuthOk——按 "uid" in data 收窄
  const applyAuthResult = (r: CommandResult<AuthOk | CaptchaData>, fromManual: boolean) => {
    if (r.success && r.data) {
      const ok = r.data;
      setDisplayName(
        "displayName" in ok ? ok.displayName || ok.username || username : username,
      );
      setPassword("");
      setPhase("idle");
      onLoggedIn();
      return;
    }
    if (r.message === "CAPTCHA_MANUAL" && r.data && "uid" in r.data) {
      setCaptcha(r.data);
      setCaptchaCode("");
      setErrorMessage(fromManual ? "验证码不正确，请重新输入" : null);
      setPhase("manual");
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
    const r = await invokeCommand<AuthOk>("login", {
      account: { username: username.trim(), password },
    });
    applyAuthResult(r, false);
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
    const r = await invokeCommand<AuthOk>("login_manual", {
      account: {
        username: username.trim(),
        password,
        captchaUid: captcha.uid,
        captchaCode: captchaCode.trim(),
      },
    });
    applyAuthResult(r, true);
  };

  const handleSavedLogin = async (saved: string) => {
    if (logging) return;
    setErrorMessage(null);
    setPhase("logging-in");
    const r = await invokeCommand<AuthOk>("login_saved", {
      account: { username: saved },
    });
    applyAuthResult(r, false);
  };

  return (
    <div className="flex min-h-screen items-center justify-center px-4">
      <div className="w-full max-w-sm">
        <div className="mb-6 flex flex-col items-center gap-3">
          <div
            aria-hidden="true"
            className="flex size-12 items-center justify-center rounded-[10px] bg-brand text-lg font-bold text-white"
          >
            锡
          </div>
          <h1 className="text-xl font-semibold text-text">锡院助手</h1>
        </div>

        <section className="rounded-[10px] border border-line bg-surface p-6">
          <form
            className="space-y-3"
            onSubmit={(e) => {
              e.preventDefault();
              void handleLogin();
            }}
          >
            <label className="block">
              <span className="mb-1 block text-sm text-text-2">学号</span>
              <Input
                value={username}
                onChange={(e) => setUsername(e.target.value)}
                autoComplete="username"
                placeholder="请输入学号"
                disabled={logging}
              />
            </label>
            <label className="block">
              <span className="mb-1 block text-sm text-text-2">密码</span>
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
              <div className="space-y-2 rounded-[10px] border border-line bg-bg p-3">
                <span className="block text-sm text-text-2">验证码</span>
                <div className="flex items-center gap-3">
                  <img
                    src={`data:image/png;base64,${captcha.pngBase64}`}
                    alt="登录验证码"
                    className="rounded border border-line bg-white"
                  />
                  <Input
                    value={captchaCode}
                    onChange={(e) => setCaptchaCode(e.target.value)}
                    placeholder="算式答案"
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
                  onClick={() => void handleManualRetry()}
                >
                  重试
                </Button>
                <p className="text-xs text-text-2">
                  自动识别验证码未成功，请输入图片中算式的答案。
                </p>
              </div>
            )}

            {errorMessage && phase !== "logging-in" && (
              <p className="text-sm text-alert" role="alert">
                {errorMessage}
              </p>
            )}

            <Button
              type="submit"
              className="w-full bg-brand text-white"
              disabled={logging}
            >
              {logging ? "正在识别验证码…" : "登录"}
            </Button>
          </form>
        </section>

        {accounts.length > 0 && (
          <section className="mt-4 rounded-[10px] border border-line bg-surface p-4">
            <h2 className="text-xs font-medium text-text-2">
              已保存账号 · 点击免密重登
            </h2>
            <div className="mt-2 flex flex-wrap gap-2">
              {accounts.map((account) => (
                <button
                  key={account.username}
                  type="button"
                  disabled={logging}
                  onClick={() => void handleSavedLogin(account.username)}
                  title={account.lastLogin}
                  className="rounded-full border border-line bg-bg px-3 py-1.5 text-sm text-text transition-colors hover:border-brand hover:text-brand disabled:pointer-events-none disabled:opacity-50"
                >
                  {account.username}
                </button>
              ))}
            </div>
          </section>
        )}
      </div>
    </div>
  );
}
