import { ArrowLeft, MonitorSmartphone, RefreshCw, ScrollText, UserRound } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { Surface } from "@/components/Surface";
import { Button } from "@/components/ui/button";
import { cn } from "@/shared/cn";
import { invokeCommand } from "@/shared/tauriApi";
import type { PlatDevice, PlatLoginLogs, PlatProfile } from "@/shared/types";
/**
 * 个人中心（M4.5 批 14）：官方「我的-设置」plat 只读面的桌面化。
 *
 * 对齐官方 `/plat/wode`（资料卡）+ `/plat/user/deviceManage`（设备管理）+
 * `plat/user/logList`（登录日志）。数据全部来自 `get_plat_*` 只读命令（plat API
 * 与一卡通同 token，见 [[plat-api-same-token]] wiki learning——组件内不需要知道）。
 *
 * 一期只读：设备「下线」/ 校园卡解绑 / 改手机号 / 改密码等写操作属账号安全动作，
 * 各自需要确认流与专项验证，不在此顺路提供（官方语义也是逐项强确认）。
 */

/** plat 用户资料里我们展示的字段（`get_plat_profile` → data 的已知键）。 */
function profileRows(p: PlatProfile): { label: string; value: string }[] {
  return [
    { label: "学号", value: p.sno || p.account || "" },
    { label: "姓名", value: p.name || "" },
    { label: "身份", value: p.identityName || "" },
    { label: "班级 / 部门", value: p.departmentName || "" },
  ].filter((r) => r.value !== "");
}

export function EcardProfileView() {
  const [phase, setPhase] = useState<"loading" | "ready" | "error">("loading");
  const [msg, setMsg] = useState("");
  const [profile, setProfile] = useState<PlatProfile | null>(null);
  const [online, setOnline] = useState<PlatDevice[]>([]);
  const [authorized, setAuthorized] = useState<PlatDevice[]>([]);
  const [logs, setLogs] = useState<PlatLoginLogs | null>(null);

  const refetch = useCallback(async () => {
    setPhase("loading");
    setMsg("");
    // 四路只读并行；设备/日志失败不阻塞资料展示（逐路兜底为空态）
    const [p, onlineR, authR, logsR] = await Promise.allSettled([
      invokeCommand<PlatProfile>("get_plat_profile"),
      invokeCommand<PlatDevice[]>("get_plat_equipment", { status: "1" }),
      invokeCommand<PlatDevice[]>("get_plat_equipment", { status: "0" }),
      invokeCommand<PlatLoginLogs>("get_plat_login_logs", { page: 1, size: 10 }),
    ]);
    const errs: string[] = [];
    if (p.status === "fulfilled" && p.value.success && p.value.data) {
      setProfile(p.value.data);
    } else {
      errs.push(p.status === "rejected" ? "资料获取失败" : p.value.message ?? "资料获取失败");
    }
    if (onlineR.status === "fulfilled" && onlineR.value.success && onlineR.value.data) {
      setOnline(onlineR.value.data);
    } else {
      setOnline([]);
      if (onlineR.status === "rejected") errs.push("在线设备获取失败");
    }
    if (authR.status === "fulfilled" && authR.value.success && authR.value.data) {
      setAuthorized(authR.value.data);
    } else {
      setAuthorized([]);
      if (authR.status === "rejected") errs.push("授权设备获取失败");
    }
    if (logsR.status === "fulfilled" && logsR.value.success && logsR.value.data) {
      setLogs(logsR.value.data);
    } else {
      setLogs(null);
      if (logsR.status === "rejected") errs.push("登录日志获取失败");
    }
    setPhase(errs.length >= 4 ? "error" : "ready");
    setMsg(errs[0] ?? "");
  }, []);

  useEffect(() => {
    void refetch();
  }, [refetch]);

  const deviceRow = (d: PlatDevice, online: boolean) => (
    <div
      key={d.id}
      className="flex items-center justify-between gap-3 rounded-control border border-line bg-surface px-3 py-2.5"
    >
      <div className="min-w-0">
        <p className="text-body font-medium text-text">
          {d.name || d.type || "设备"}
          {online && (
            <span className="ml-2 text-caption font-normal text-ok">当前在线</span>
          )}
        </p>
        <p className="mt-0.5 text-caption text-text-2">
          最近登录：{d.updateTime || d.createTime || "—"}
        </p>
      </div>
      <MonitorSmartphone aria-hidden className="size-4 shrink-0 text-text-2" />
    </div>
  );

  return (
    <Surface className="px-4 py-4">
      <div className="flex items-center justify-between gap-3">
        <p className="text-body font-medium text-text">个人中心</p>
        <Button variant="ghost" size="xs" onClick={() => void refetch()}>
          <RefreshCw aria-hidden className="size-3" />
          刷新
        </Button>
      </div>
      <p className="mt-1 text-caption text-text-2">
        与官方 APP「我的 / 设置」同源；一期只读，设备下线等操作后续单独提供。
      </p>

      {phase === "loading" && (
        <p className="mt-3 text-caption text-text-2" aria-busy>
          正在获取个人资料与设备…
        </p>
      )}
      {phase === "error" && (
        <div className="mt-3">
          <p className="text-caption text-alert">{msg || "获取失败"}</p>
          <Button variant="outline" size="sm" className="mt-2" onClick={() => void refetch()}>
            重试
          </Button>
        </div>
      )}

      {phase === "ready" && profile && (
        <>
          {/* 资料卡：对齐官方电子卡面（头像 + 学号/姓名/院系） */}
          <div className="mt-3 flex items-center gap-3 rounded-inner border border-line bg-surface px-3 py-3">
            {profile.avatar ? (
              <img
                src={profile.avatar}
                alt=""
                aria-hidden
                className="size-12 rounded-full border border-line object-cover"
                draggable={false}
              />
            ) : (
              <span className="flex size-12 items-center justify-center rounded-full border border-line bg-surface-2">
                <UserRound aria-hidden className="size-6 text-text-2" />
              </span>
            )}
            <div className="min-w-0">
              <p className="text-body font-medium text-text">
                {profile.name || "—"}
                {profile.identityName && (
                  <span className="ml-2 text-caption font-normal text-text-2">
                    {profile.identityName}
                  </span>
                )}
              </p>
              <p className="mt-0.5 text-caption text-text-2">
                学号 {profile.sno || profile.account || "—"}
              </p>
            </div>
          </div>

          <div className="mt-3 grid grid-cols-2 gap-2">
            {profileRows(profile).map((r) => (
              <div key={r.label} className="rounded-control border border-line bg-surface px-3 py-2">
                <p className="text-caption text-text-2">{r.label}</p>
                <p className="mt-0.5 text-body text-text">{r.value}</p>
              </div>
            ))}
          </div>

          <p className="mt-4 text-caption font-medium text-text">已登录设备</p>
          <div className="mt-1.5 grid gap-1.5">
            {online.length === 0 ? (
              <p className="text-caption text-text-2">暂无在线设备</p>
            ) : (
              online.map((d) => deviceRow(d, true))
            )}
          </div>

          <p className="mt-4 text-caption font-medium text-text">已授权手机设备</p>
          <div className="mt-1.5 grid gap-1.5">
            {authorized.length === 0 ? (
              <p className="text-caption text-text-2">暂无已授权设备</p>
            ) : (
              authorized.map((d) => deviceRow(d, false))
            )}
          </div>

          <p className={cn("mt-4 flex items-center gap-1 text-caption font-medium text-text")}>
            <ScrollText aria-hidden className="size-3.5" />
            登录日志
          </p>
          <div className="mt-1.5">
            {!logs || logs.records.length === 0 ? (
              <p className="text-caption text-text-2">暂无登录日志</p>
            ) : (
              <div className="grid gap-1">
                {logs.records.map((r, i) => (
                  <div
                    key={r.id ?? i}
                    className="flex items-center justify-between gap-3 rounded-control border border-line bg-surface px-3 py-2"
                  >
                    <p className="text-caption text-text">{r.createTime || "—"}</p>
                    <p className="text-caption text-text-2">{r.ip || ""}</p>
                  </div>
                ))}
              </div>
            )}
          </div>
        </>
      )}

      {/* 兜底提示（部分子请求失败但资料可用时） */}
      {phase === "ready" && msg && (
        <p className="mt-3 flex items-center gap-1 text-caption text-text-2">
          <ArrowLeft aria-hidden className="size-3" />
          {msg}
        </p>
      )}
    </Surface>
  );
}
