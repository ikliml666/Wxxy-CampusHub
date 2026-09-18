import { useEffect } from "react";
import AppShell from "./components/AppShell";
import { LoginDialog } from "./components/LoginDialog";
import { AvatarDialog } from "./components/AvatarDialog";
import { useAuthStore } from "@/stores/authStore";

export default function App() {
  // 启动会话探测：非阻塞（不 await 渲染）。status "unknown" 期间壳与面板照常
  // 可交互，落定 guest/authed 后由各组件自行响应（游客空态 / 登录态内容）。
  useEffect(() => {
    void useAuthStore.getState().checkSession();
  }, []);

  return (
    <>
      <AppShell />
      <LoginDialog />
      <AvatarDialog />
    </>
  );
}
