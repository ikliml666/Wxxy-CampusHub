import { useEffect, useState } from "react";
import AppShell from "./components/AppShell";
import { LoginPanel } from "./panels/LoginPanel";
import { invokeCommand } from "@/shared/tauriApi";
import { useUiStore } from "@/stores/uiStore";

type AppPhase = "checking" | "in" | "out";

export default function App() {
  const [phase, setPhase] = useState<AppPhase>("checking");
  const setDisplayName = useUiStore((s) => s.setDisplayName);

  // 启动会话检测：jar 检查 + 门户轻探测（Rust 侧）；保持登录则直接进主界面
  useEffect(() => {
    invokeCommand<{ loggedIn: boolean }>("check_session").then((r) => {
      setPhase(r.success && r.data?.loggedIn === true ? "in" : "out");
    });
  }, []);

  if (phase === "checking") return null;

  if (phase === "out") {
    return <LoginPanel onLoggedIn={() => setPhase("in")} />;
  }

  // 退出：乐观切回登录页（logout 命令后台执行，UI 立即响应；重启时 check_session 兜底）
  const handleLogout = () => {
    void invokeCommand("logout");
    setDisplayName(null);
    setPhase("out");
  };

  return <AppShell onLogout={handleLogout} />;
}
