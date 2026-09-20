import { UserRound } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { Button } from "@/components/ui/button";
import { EmptyState } from "@/components/EmptyState";
import { Surface } from "@/components/Surface";
import { cn } from "@/shared/cn";
import { invokeCommand } from "@/shared/tauriApi";
import type { EcardFaceDetail } from "@/shared/types";

/**
 * 人脸采集子页（官方 overLightMobileH5「人脸采集」的复刻，M4.5 批 5）。
 *
 * 数据流：`ecard_face_detail`（读状态；H5 账号走官方 autoLogin 固定密码约定登录）→
 * 用户选照片（`<input type="file">` + FileReader，**照片不落盘、不进日志**）→
 * 二次确认 → `ecard_face_upload`（multipart 写入学校人脸库，用于食堂/门禁刷脸）。
 *
 * 红线：照片 base64 只在内存流转；不写 localStorage、不打日志。上传是**写操作**，
 * 必须显式确认后才发请求。
 */
export function EcardFaceView({ onChanged }: { onChanged: () => void }) {
  const [phase, setPhase] = useState<"loading" | "ready" | "error">("loading");
  const [detail, setDetail] = useState<EcardFaceDetail | null>(null);
  const [message, setMessage] = useState("");
  const [err, setErr] = useState("");
  const [busy, setBusy] = useState(false);
  const [previewUrl, setPreviewUrl] = useState("");
  const fileRef = useRef<HTMLInputElement>(null);
  const pendingRef = useRef<string>("");

  const load = async () => {
    setPhase("loading");
    setErr("");
    const r = await invokeCommand<EcardFaceDetail>("ecard_face_detail", {});
    if (r.success && r.data) {
      setDetail(r.data);
      setPhase("ready");
    } else {
      setErr(r.message ?? "人脸采集状态获取失败");
      setPhase("error");
    }
  };

  useEffect(() => {
    void load();
    // 组件卸载时释放预览 objectURL
    return () => {
      if (previewUrl) URL.revokeObjectURL(previewUrl);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const pickFile = (f: File | undefined) => {
    if (!f) return;
    if (!/^image\/(jpeg|png)$/.test(f.type)) {
      setErr("请选择 JPEG 或 PNG 照片");
      return;
    }
    if (f.size > 8 * 1024 * 1024) {
      setErr("照片不能超过 8 MB");
      return;
    }
    const reader = new FileReader();
    reader.onload = () => {
      const dataUrl = String(reader.result ?? "");
      const b64 = dataUrl.slice(dataUrl.indexOf(",") + 1);
      pendingRef.current = b64;
      setPreviewUrl(URL.createObjectURL(f));
      setErr("");
      setMessage("");
    };
    reader.readAsDataURL(f);
  };

  const upload = async () => {
    if (!pendingRef.current) return;
    if (
      !window.confirm(
        "确认上传这张照片作为校园人脸？\n照片将写入学校人脸库（食堂/门禁刷脸用）。",
      )
    ) {
      return;
    }
    setBusy(true);
    setErr("");
    setMessage("");
    const r = await invokeCommand("ecard_face_upload", {
      photoBase64: pendingRef.current,
    });
    setBusy(false);
    if (r.success) {
      setMessage("上传成功。人脸已提交学校系统，刷脸设备生效以学校为准。");
      pendingRef.current = "";
      if (previewUrl) URL.revokeObjectURL(previewUrl);
      setPreviewUrl("");
      onChanged();
      void load();
    } else {
      setErr(r.message ?? "上传失败");
    }
  };

  if (phase === "loading") {
    return (
      <Surface className="px-4 py-4">
        <div aria-hidden>
          <div className="h-4 w-32 animate-pulse rounded bg-line" />
          <div className="mt-3 h-24 animate-pulse rounded bg-line" />
        </div>
      </Surface>
    );
  }

  if (phase === "error") {
    return (
      <Surface accent="wallet" className="px-4 py-4">
        <div className="flex items-center justify-between gap-3">
          <p className="min-w-0 text-body text-text-2">获取失败：{err}</p>
          <Button
            variant="outline"
            size="sm"
            className="shrink-0"
            onClick={() => void load()}
          >
            重试
          </Button>
        </div>
      </Surface>
    );
  }

  return (
    <div className="space-y-3">
      <Surface accent="wallet" className="px-4 py-4">
        <div className="flex items-start justify-between gap-3">
          <div className="min-w-0">
            <p className="text-body font-medium text-text">
              {detail?.name || "—"}
            </p>
            <p className="mt-0.5 text-caption text-text-2">
              {detail?.schoolName || "—"} · 学工号 {detail?.number || "—"}
            </p>
            <p
              className={cn(
                "mt-1 text-caption font-medium",
                detail?.collected ? "text-[var(--color-wallet)]" : "text-text-2",
              )}
            >
              {detail?.collected ? "已采集人脸" : "未采集人脸"}
            </p>
          </div>
          <div
            aria-hidden
            className="grid size-16 shrink-0 place-items-center rounded-card border border-line bg-surface-2"
          >
            {previewUrl ? (
              <img
                src={previewUrl}
                alt=""
                className="size-full rounded-card object-cover"
              />
            ) : (
              <UserRound className="size-7 text-text-2" />
            )}
          </div>
        </div>
      </Surface>

      <Surface className="px-4 py-4">
        <p className="text-body font-medium text-text">
          {detail?.collected ? "更换人脸照片" : "上传人脸照片"}
        </p>
        <p className="mt-1 text-caption text-text-2">
          正脸、无遮挡、光线均匀的近期照片；仅支持 JPEG / PNG，≤ 8 MB。照片将
          <span className="text-text">写入学校人脸库</span>
          ，用于食堂与门禁刷脸。
        </p>
        <input
          ref={fileRef}
          type="file"
          accept="image/jpeg,image/png"
          className="hidden"
          onChange={(e) => pickFile(e.target.files?.[0])}
        />
        <div className="mt-3 flex flex-wrap items-center gap-2">
          <Button
            variant="outline"
            size="sm"
            disabled={busy}
            onClick={() => fileRef.current?.click()}
          >
            选择照片
          </Button>
          <Button
            size="sm"
            disabled={busy || !pendingRef.current}
            onClick={() => void upload()}
          >
            {busy ? "上传中…" : "确认上传"}
          </Button>
          {pendingRef.current && !busy && (
            <Button
              variant="ghost"
              size="sm"
              onClick={() => {
                pendingRef.current = "";
                if (previewUrl) URL.revokeObjectURL(previewUrl);
                setPreviewUrl("");
              }}
            >
              撤销选择
            </Button>
          )}
        </div>
        {message && (
          <p className="mt-2 text-caption font-medium text-[var(--color-wallet)]">
            {message}
          </p>
        )}
        {err && <p className="mt-2 text-caption text-alert">{err}</p>}
      </Surface>
    </div>
  );
}

/** 空态兜底（详情缺失时整页占位，不白屏）。 */
export function EcardFaceEmpty() {
  return (
    <Surface accent="wallet" className="px-4 py-6">
      <EmptyState
        icon={UserRound}
        domain="wallet"
        title="人脸信息不可用"
      />
    </Surface>
  );
}
