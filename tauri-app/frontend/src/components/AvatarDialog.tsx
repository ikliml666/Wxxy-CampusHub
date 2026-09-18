import { useCallback, useEffect, useRef, useState } from "react";
import Cropper, { type Area } from "react-easy-crop";
import "react-easy-crop/react-easy-crop.css";
import { motion, useReducedMotion } from "framer-motion";
import { Circle, ImageUp, RotateCcw, Square, Trash2, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { useAuthStore } from "@/stores/authStore";
import { useUiStore } from "@/stores/uiStore";

// 头像管线：选图 → react-easy-crop 1:1 取景（拖拽/滚轮/滑杆缩放）→ Canvas 导出。
// 两条保存路径共用同一裁切结果：
//   本机   min(1024, 裁切边长) 不放大，透明出 PNG、否则 JPEG q0.92 → set_avatar（裸 base64，上限 2MB）
//   学校   尺寸×质量阶梯内搜索能塞进 200KB 的最大尺寸，JPEG 铺白底 → upload_official_avatar（完整 data URL）
const LOCAL_MAX_SIDE = 1024;
const LOCAL_MAX_BYTES = 2 * 1024 * 1024;
const SCHOOL_MAX_BYTES = 200 * 1024;
const SIZE_LADDER = [1024, 896, 768, 640, 512, 448, 384, 320];
const QUALITY_LADDER = [0.95, 0.9, 0.85, 0.8, 0.75, 0.7, 0.65, 0.6];

/** 把裁切区域画到 side×side 画布；white=true 先铺白底（JPEG 无透明通道，防透明区发黑） */
function drawCrop(
  img: HTMLImageElement,
  area: Area,
  side: number,
  white: boolean,
): HTMLCanvasElement {
  const canvas = document.createElement("canvas");
  canvas.width = side;
  canvas.height = side;
  const ctx = canvas.getContext("2d");
  if (!ctx) throw new Error("canvas 2d context unavailable");
  if (white) {
    ctx.fillStyle = "white";
    ctx.fillRect(0, 0, side, side);
  }
  ctx.drawImage(img, area.x, area.y, area.width, area.height, 0, 0, side, side);
  return canvas;
}

function canvasToBlob(canvas: HTMLCanvasElement, mime: string, quality: number): Promise<Blob> {
  return new Promise((resolve, reject) => {
    canvas.toBlob(
      (b) => (b ? resolve(b) : reject(new Error("图片编码失败"))),
      mime,
      quality,
    );
  });
}

function blobToDataUrl(blob: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const fr = new FileReader();
    fr.onload = () => resolve(fr.result as string);
    fr.onerror = () => reject(new Error("图片读取失败"));
    fr.readAsDataURL(blob);
  });
}

/** 任一像素 alpha < 255 即视为含透明通道 */
function hasAlpha(canvas: HTMLCanvasElement): boolean {
  const ctx = canvas.getContext("2d");
  if (!ctx) return false;
  const data = ctx.getImageData(0, 0, canvas.width, canvas.height).data;
  for (let i = 3; i < data.length; i += 4) {
    if (data[i] < 255) return true;
  }
  return false;
}

/** 本机版本导出：min(1024, 裁切边长) 不放大；透明出 PNG，否则 JPEG q0.92；PNG 超 2MB 回退白底 JPEG */
async function renderLocal(
  img: HTMLImageElement,
  area: Area,
): Promise<{ dataUrl: string; side: number; bytes: number }> {
  const side = Math.min(LOCAL_MAX_SIDE, Math.round(area.width));
  const canvas = drawCrop(img, area, side, false);
  let blob: Blob;
  if (hasAlpha(canvas)) {
    blob = await canvasToBlob(canvas, "image/png", 1);
    if (blob.size > LOCAL_MAX_BYTES) {
      // 透明 PNG 超 2MB 时回退白底 JPEG，罕见路径
      blob = await canvasToBlob(drawCrop(img, area, side, true), "image/jpeg", 0.92);
    }
  } else {
    blob = await canvasToBlob(canvas, "image/jpeg", 0.92);
  }
  return { dataUrl: await blobToDataUrl(blob), side, bytes: blob.size };
}

/** 学校版本搜索：从大到小遍历尺寸阶梯，尺寸内质量从高到低取第一个 ≤200KB；源分辨率不足的档位 clamp 不放大 */
async function findSchoolImage(
  img: HTMLImageElement,
  area: Area,
): Promise<{ side: number; blob: Blob } | null> {
  const srcSide = Math.round(area.width);
  let prev = -1;
  for (const target of SIZE_LADDER) {
    const side = Math.min(target, srcSide);
    if (side === prev) continue; // clamp 后与上一档重合，跳过重复编码
    prev = side;
    const canvas = drawCrop(img, area, side, true);
    for (const q of QUALITY_LADDER) {
      const blob = await canvasToBlob(canvas, "image/jpeg", q);
      if (blob.size <= SCHOOL_MAX_BYTES) return { side, blob };
    }
  }
  return null;
}

function fmtSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(2)} MB`;
}

type Busy = "local" | "school" | "sync" | "clear";

export function AvatarDialog() {
  const open = useUiStore((s) => s.avatarDialogOpen);
  const closeAvatarDialog = useUiStore((s) => s.closeAvatarDialog);
  const status = useAuthStore((s) => s.status);
  const uploadAvatar = useAuthStore((s) => s.uploadAvatar);
  const uploadOfficialAvatar = useAuthStore((s) => s.uploadOfficialAvatar);
  const refreshAvatar = useAuthStore((s) => s.refreshAvatar);
  const syncOfficialAvatar = useAuthStore((s) => s.syncOfficialAvatar);
  const clearAvatar = useAuthStore((s) => s.clearAvatar);

  const [imgUrl, setImgUrl] = useState<string | null>(null);
  const [imgEl, setImgEl] = useState<HTMLImageElement | null>(null);
  const [origBytes, setOrigBytes] = useState(0);
  const [area, setArea] = useState<Area | null>(null);
  const [crop, setCrop] = useState({ x: 0, y: 0 });
  const [zoom, setZoom] = useState(1);
  const [shape, setShape] = useState<"round" | "rect">("round");
  const [localInfo, setLocalInfo] = useState<{ side: number; kb: number } | null>(null);
  const [schoolInfo, setSchoolInfo] = useState<{ side: number; kb: number } | null>(null);
  const [dragging, setDragging] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [done, setDone] = useState<string | null>(null);
  const [busy, setBusy] = useState<Busy | null>(null);

  const fileInputRef = useRef<HTMLInputElement>(null);
  const reduceMotion = useReducedMotion();
  const authed = status === "authed";

  // 打开/关闭：重置残留并释放 objectURL（cleanup 按 imgUrl 变更逐个 revoke）
  useEffect(() => {
    if (open) return;
    setImgUrl(null);
    setImgEl(null);
    setOrigBytes(0);
    setArea(null);
    setCrop({ x: 0, y: 0 });
    setZoom(1);
    setShape("round");
    setLocalInfo(null);
    setSchoolInfo(null);
    setDragging(false);
    setError(null);
    setDone(null);
    setBusy(null);
  }, [open]);

  useEffect(() => {
    if (!imgUrl) return;
    return () => URL.revokeObjectURL(imgUrl);
  }, [imgUrl]);

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

  // 裁切变化后防抖 250ms 真实重编码，给出两条路径的体积预估（保存时以同函数重算为准）
  useEffect(() => {
    if (!imgEl || !area) return;
    let stale = false;
    const t = window.setTimeout(async () => {
      try {
        const local = await renderLocal(imgEl, area);
        if (stale) return;
        setLocalInfo({ side: local.side, kb: Math.ceil(local.bytes / 1024) });
        if (authed) {
          const school = await findSchoolImage(imgEl, area);
          if (stale) return;
          setSchoolInfo(
            school ? { side: school.side, kb: Math.ceil(school.blob.size / 1024) } : null,
          );
        }
      } catch {
        // 估算失败静默；保存路径会重试并把错误内联展示
      }
    }, 250);
    return () => {
      stale = true;
      window.clearTimeout(t);
    };
  }, [imgEl, area, authed]);

  // 裁切回调必须声明在提前 return 之前：Hook 顺序不能随 open 变化
  const onCropComplete = useCallback((_: Area, pixels: Area) => setArea(pixels), []);

  if (!open) return null;

  const handleFile = async (file: File | null | undefined) => {
    if (!file) return;
    setError(null);
    setDone(null);
    const url = URL.createObjectURL(file);
    const img = new Image();
    img.src = url;
    try {
      await img.decode();
    } catch {
      URL.revokeObjectURL(url);
      setError("无法读取图片文件，请换一张试试");
      return;
    }
    setImgUrl(url); // 旧的由 revoke effect 释放
    setImgEl(img);
    setOrigBytes(file.size);
    setCrop({ x: 0, y: 0 });
    setZoom(1);
    setArea(null);
    setLocalInfo(null);
    setSchoolInfo(null);
  };

  const handleSaveLocal = async () => {
    if (!imgEl || !area || busy || done) return;
    setError(null);
    setBusy("local");
    try {
      const local = await renderLocal(imgEl, area);
      const raw = local.dataUrl.slice(local.dataUrl.indexOf(",") + 1);
      const r = await uploadAvatar(raw);
      if (r.success) {
        void refreshAvatar();
        closeAvatarDialog();
        return;
      }
      setError(r.message ?? "保存失败，请稍后重试");
    } catch (e) {
      setError(e instanceof Error ? e.message : "保存失败，请稍后重试");
    } finally {
      setBusy(null);
    }
  };

  const handleSaveSchool = async () => {
    if (!imgEl || !area || busy || done || !authed) return;
    setError(null);
    setBusy("school");
    try {
      const found = await findSchoolImage(imgEl, area);
      if (!found) throw new Error("无法压缩到 200 KB 内，请换一张试试");
      const dataUrl = await blobToDataUrl(found.blob);
      const r = await uploadOfficialAvatar(dataUrl);
      if (!r.success) throw new Error(r.message ?? "上传失败，请稍后重试");
      // 上传成功同时把这版存本机（同一高分辨率图），随后提示完成并自动关闭
      const r2 = await uploadAvatar(dataUrl.slice(dataUrl.indexOf(",") + 1));
      if (!r2.success) throw new Error(r2.message ?? "本机保存失败，请稍后重试");
      setDone(`已上传学校 · ${found.side}×${found.side} · ${Math.ceil(found.blob.size / 1024)} KB`);
      window.setTimeout(() => closeAvatarDialog(), 1200);
    } catch (e) {
      setError(e instanceof Error ? e.message : "上传失败，请稍后重试");
    } finally {
      setBusy(null);
    }
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

  const pick = () => fileInputRef.current?.click();
  const resetCrop = () => {
    setZoom(1);
    setCrop({ x: 0, y: 0 });
  };

  const shapeBtn = (active: boolean) =>
    "flex size-7 items-center justify-center rounded-control transition-colors duration-[var(--dur-fast)] " +
    (active ? "bg-surface text-text shadow-card" : "text-text-2 hover:text-text");

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
        className="relative w-full max-w-md rounded-card border border-line bg-surface p-5 shadow-pop"
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
            拖动取景、滚轮或滑杆缩放，输出 1:1 头像
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

        {imgEl && imgUrl ? (
          <div className="flex flex-col items-stretch gap-3">
            {/* 工具行：形状切换 · 重置 · 换一张 */}
            <div className="flex items-center gap-1.5">
              <div
                className="flex items-center gap-0.5 rounded-control bg-surface-2 p-0.5"
                role="group"
                aria-label="裁切形状"
              >
                <button
                  type="button"
                  aria-label="圆形"
                  aria-pressed={shape === "round"}
                  onClick={() => setShape("round")}
                  className={shapeBtn(shape === "round")}
                >
                  <Circle className="size-3.5" aria-hidden="true" />
                </button>
                <button
                  type="button"
                  aria-label="方形"
                  aria-pressed={shape === "rect"}
                  onClick={() => setShape("rect")}
                  className={shapeBtn(shape === "rect")}
                >
                  <Square className="size-3.5" aria-hidden="true" />
                </button>
              </div>
              <button
                type="button"
                onClick={resetCrop}
                className="flex items-center gap-1 rounded-control px-1.5 py-1 text-caption text-text-2 transition-colors duration-[var(--dur-fast)] hover:bg-surface-2 hover:text-text"
              >
                <RotateCcw className="size-3.5" aria-hidden="true" />
                重置
              </button>
              <span className="flex-1" />
              <button
                type="button"
                onClick={pick}
                className="rounded-control px-2 py-1 text-caption text-text-2 transition-colors duration-[var(--dur-fast)] hover:bg-surface-2 hover:text-text"
              >
                换一张
              </button>
            </div>

            {/* 280×280 取景舞台 */}
            <div className="relative mx-auto size-[280px] overflow-hidden rounded-inner bg-black/80">
              <Cropper
                image={imgUrl}
                aspect={1}
                crop={crop}
                onCropChange={setCrop}
                zoom={zoom}
                onZoomChange={setZoom}
                minZoom={1}
                maxZoom={3}
                cropShape={shape}
                showGrid={shape === "rect"}
                objectFit="contain"
                onCropComplete={onCropComplete}
                classes={{ containerClassName: "rounded-inner overflow-hidden" }}
              />
            </div>

            {/* 缩放滑杆 */}
            <div className="flex items-center gap-2">
              <span className="shrink-0 text-caption text-text-2" id="avatar-zoom-label">
                缩放
              </span>
              <input
                type="range"
                min={1}
                max={3}
                step={0.01}
                value={zoom}
                onChange={(e) => setZoom(Number(e.target.value))}
                aria-labelledby="avatar-zoom-label"
                className="h-1 flex-1 accent-brand"
              />
            </div>

            {/* 体积与尺寸信息（防抖后真实编码结果） */}
            <div className="flex flex-col gap-0.5 text-caption text-text-2 tabular-num">
              <p>
                原图 {fmtSize(origBytes)} · 本机{" "}
                {localInfo
                  ? `${localInfo.side}×${localInfo.side} · 约 ${localInfo.kb} KB`
                  : "计算中…"}
              </p>
              <p>
                {authed
                  ? `学校 ${
                      schoolInfo
                        ? `${schoolInfo.side}×${schoolInfo.side} · 约 ${schoolInfo.kb} KB`
                        : "计算中…"
                    } / 上限 200 KB`
                  : "登录后可上传到学校"}
              </p>
            </div>

            {done && (
              <p className="text-caption text-wallet" role="status">
                {done}
              </p>
            )}
            {error && (
              <p className="text-caption text-alert" role="alert" aria-live="polite">
                {error}
              </p>
            )}

            {/* 两条主操作 */}
            <div className="grid grid-cols-2 gap-2">
              <Button
                variant="secondary"
                disabled={!area || busy !== null || done !== null}
                aria-busy={busy === "local"}
                onClick={() => void handleSaveLocal()}
              >
                仅保存到本机
              </Button>
              <TooltipProvider>
                <Tooltip>
                  <TooltipTrigger asChild>
                    {/* disabled 按钮不派发 hover，包一层 span 承接 Tooltip */}
                    <span tabIndex={-1} className="inline-flex">
                      <Button
                        disabled={!authed || !area || busy !== null || done !== null}
                        aria-busy={busy === "school"}
                        onClick={() => void handleSaveSchool()}
                      >
                        保存并上传学校
                      </Button>
                    </span>
                  </TooltipTrigger>
                  {!authed && <TooltipContent>登录后可上传到学校</TooltipContent>}
                </Tooltip>
              </TooltipProvider>
            </div>
          </div>
        ) : (
          <button
            type="button"
            onClick={pick}
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

        {error && !imgEl && (
          <p className="mt-2 text-caption text-alert" role="alert" aria-live="polite">
            {error}
          </p>
        )}

        <div className="mt-4">
          {authed && (
            <div className="grid grid-cols-2 gap-2">
              <Button
                variant="outline"
                size="sm"
                disabled={busy !== null || done !== null}
                aria-busy={busy === "sync"}
                onClick={() => void handleSync()}
              >
                同步学校头像
              </Button>
              <Button
                variant="outline"
                size="sm"
                disabled={busy !== null || done !== null}
                aria-busy={busy === "clear"}
                onClick={() => void handleClear()}
              >
                <Trash2 className="size-4" aria-hidden="true" />
                移除本机头像
              </Button>
            </div>
          )}
        </div>
      </motion.div>
    </div>
  );
}
