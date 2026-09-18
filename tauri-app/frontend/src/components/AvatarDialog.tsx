import { useEffect, useRef, useState } from "react";
import { motion, useReducedMotion } from "framer-motion";
import { ImageUp, Trash2, X } from "lucide-react";
import { Avatar } from "@/components/Avatar";
import { Button } from "@/components/ui/button";
import { useAuthStore } from "@/stores/authStore";
import { useUiStore } from "@/stores/uiStore";

// 头像管线：选图/拖图 → Image 解码 → Canvas 中心裁方 → 缩放 256×256 →
// 有透明通道输出 PNG、否则 JPEG q0.9 → 裸 base64 交给 uploadAvatar。
// 体积守卫：转换后 base64 > 512KB（后端 clamp 上限）内联报错并禁用保存。
const TARGET = 256;
const MAX_BASE64 = 512 * 1024;

interface Preview {
  dataUrl: string;
  rawBase64: string;
}

/** 中心裁方 + 缩放导出；非图片/解码失败抛错由调用方内联展示 */
async function fileToPreview(file: File): Promise<Preview> {
  const bitmap = await createImageBitmap(file, { imageOrientation: "from-image" });
  try {
    const side = Math.min(bitmap.width, bitmap.height);
    const sx = (bitmap.width - side) / 2;
    const sy = (bitmap.height - side) / 2;
    const canvas = document.createElement("canvas");
    canvas.width = TARGET;
    canvas.height = TARGET;
    const ctx = canvas.getContext("2d");
    if (!ctx) throw new Error("canvas 2d context unavailable");
    ctx.drawImage(bitmap, sx, sy, side, side, 0, 0, TARGET, TARGET);
    const alphaChannel = ctx.getImageData(0, 0, TARGET, TARGET).data;
    let hasAlpha = false;
    for (let i = 3; i < alphaChannel.length; i += 4) {
      if (alphaChannel[i] < 255) {
        hasAlpha = true;
        break;
      }
    }
    const dataUrl = canvas.toDataURL(hasAlpha ? "image/png" : "image/jpeg", 0.9);
    return { dataUrl, rawBase64: dataUrl.slice(dataUrl.indexOf(",") + 1) };
  } finally {
    bitmap.close();
  }
}

function fmtSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(2)} MB`;
}

export function AvatarDialog() {
  const open = useUiStore((s) => s.avatarDialogOpen);
  const closeAvatarDialog = useUiStore((s) => s.closeAvatarDialog);
  const status = useAuthStore((s) => s.status);
  const uploadAvatar = useAuthStore((s) => s.uploadAvatar);
  const syncOfficialAvatar = useAuthStore((s) => s.syncOfficialAvatar);
  const clearAvatar = useAuthStore((s) => s.clearAvatar);

  const [preview, setPreview] = useState<Preview | null>(null);
  const [origSize, setOrigSize] = useState(0);
  const [tooBig, setTooBig] = useState(false);
  const [dragging, setDragging] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<"save" | "sync" | "clear" | null>(null);

  const fileInputRef = useRef<HTMLInputElement>(null);
  const reduceMotion = useReducedMotion();
  const authed = status === "authed";

  // 打开：重置上次残留
  useEffect(() => {
    if (!open) return;
    setPreview(null);
    setOrigSize(0);
    setTooBig(false);
    setDragging(false);
    setError(null);
    setBusy(null);
  }, [open]);

  // Esc 关闭 + 弹层期间锁背景滚动
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") closeAvatarDialog();
    };
    window.addEventListener("keydown", onKey);
    const prevOverflow = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    return () => {
      window.removeEventListener("keydown", onKey);
      document.body.style.overflow = prevOverflow;
    };
  }, [open, closeAvatarDialog]);

  if (!open) return null;

  const handleFile = async (file: File | null | undefined) => {
    if (!file) return;
    setError(null);
    setTooBig(false);
    try {
      const p = await fileToPreview(file);
      setOrigSize(file.size);
      setPreview(p);
      setTooBig(p.rawBase64.length > MAX_BASE64);
    } catch {
      setError("无法读取图片文件，请换一张试试");
    }
  };

  const handleSave = async () => {
    if (!preview || tooBig || busy) return;
    setError(null);
    setBusy("save");
    const r = await uploadAvatar(preview.rawBase64);
    setBusy(null);
    if (r.success) {
      closeAvatarDialog();
      return;
    }
    setError(r.message ?? "保存失败，请稍后重试");
  };

  const handleSync = async () => {
    if (busy) return;
    setError(null);
    setBusy("sync");
    const r = await syncOfficialAvatar();
    setBusy(null);
    if (r.success) {
      closeAvatarDialog();
      return;
    }
    setError(r.message ?? "同步失败，请稍后重试");
  };

  const handleClear = async () => {
    if (busy) return;
    setError(null);
    setBusy("clear");
    const r = await clearAvatar();
    setBusy(null);
    if (r.success) {
      closeAvatarDialog();
      return;
    }
    setError(r.message ?? "移除失败，请稍后重试");
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 px-4"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) closeAvatarDialog();
      }}
    >
      <motion.div
        role="dialog"
        aria-modal="true"
        aria-label="设置头像"
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
          aria-label="关闭头像弹窗"
          onClick={closeAvatarDialog}
          className="absolute right-3 top-3 flex size-7 items-center justify-center rounded-full text-text-2 transition-colors duration-[var(--dur-fast)] hover:bg-surface-2 hover:text-text"
        >
          <X className="size-4" aria-hidden="true" />
        </button>

        <div className="mb-4">
          <h2 className="text-title font-semibold text-text">设置头像</h2>
          <p className="text-caption text-text-2">
            支持 jpg / png，自动居中裁方并压缩到 256×256
          </p>
        </div>

        <input
          ref={fileInputRef}
          type="file"
          accept="image/*"
          hidden
          onChange={(e) => {
            void handleFile(e.target.files?.[0]);
            e.target.value = ""; // 允许重复选择同一文件
          }}
        />

        {preview ? (
          <div className="flex flex-col items-center gap-2.5 py-1">
            <Avatar src={preview.dataUrl} size="xl" name="头像预览" />
            <p className="text-caption text-text-2 tabular-num">
              原 {fmtSize(origSize)} → {fmtSize(preview.rawBase64.length)}
            </p>
            <button
              type="button"
              onClick={() => fileInputRef.current?.click()}
              className="rounded-control px-2 py-1 text-caption text-text-2 transition-colors duration-[var(--dur-fast)] hover:bg-surface-2 hover:text-text"
            >
              换一张
            </button>
          </div>
        ) : (
          <button
            type="button"
            onClick={() => fileInputRef.current?.click()}
            onDragOver={(e) => {
              e.preventDefault();
              setDragging(true);
            }}
            onDragLeave={() => setDragging(false)}
            onDrop={(e) => {
              e.preventDefault();
              setDragging(false);
              void handleFile(e.dataTransfer.files?.[0]);
            }}
            className={
              "flex w-full flex-col items-center gap-2 rounded-inner border border-dashed px-4 py-8 text-caption text-text-2 transition-colors duration-[var(--dur-fast)] ease-out-soft " +
              (dragging
                ? "border-brand bg-brand/5 text-text"
                : "border-line-strong bg-surface-2 hover:border-brand/50 hover:text-text")
            }
          >
            <ImageUp className="size-5" aria-hidden="true" />
            拖入图片或点击选择
          </button>
        )}

        {tooBig && (
          <p className="mt-2 text-caption text-alert" role="alert">
            图片过大，请换一张
          </p>
        )}
        {error && (
          <p className="mt-2 text-body text-alert" role="alert" aria-live="polite">
            {error}
          </p>
        )}

        <div className="mt-4">
          <Button
            className="w-full"
            disabled={!preview || tooBig || busy !== null}
            aria-busy={busy === "save"}
            onClick={() => void handleSave()}
          >
            保存
          </Button>
          {authed && (
            <div className="mt-2 grid grid-cols-2 gap-2">
              <Button
                variant="outline"
                disabled={busy !== null}
                aria-busy={busy === "sync"}
                onClick={() => void handleSync()}
              >
                同步学校头像
              </Button>
              <Button
                variant="outline"
                disabled={busy !== null}
                aria-busy={busy === "clear"}
                onClick={() => void handleClear()}
              >
                <Trash2 className="size-4" aria-hidden="true" />
                移除本地头像
              </Button>
            </div>
          )}
        </div>
      </motion.div>
    </div>
  );
}
