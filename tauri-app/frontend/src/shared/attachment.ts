import {
  File,
  FileArchive,
  FileSpreadsheet,
  FileText,
  FileType2,
  type LucideIcon,
} from "lucide-react";
import { downloadAttachment } from "./tauriApi";

/** 文件名后缀（不含点，小写；无后缀返回空串）。 */
export function attachmentExt(name: string): string {
  const i = name.lastIndexOf(".");
  return i > 0 ? name.slice(i + 1).toLowerCase() : "";
}

/** PDF 判定：.pdf 后缀不区分大小写。 */
export const isPdfAttachment = (name: string) => attachmentExt(name) === "pdf";

/** 附件类型图标（lucide 映射）。 */
export function attachmentIcon(name: string): LucideIcon {
  switch (attachmentExt(name)) {
    case "pdf":
      return FileText;
    case "xls":
    case "xlsx":
    case "csv":
      return FileSpreadsheet;
    case "doc":
    case "docx":
      return FileType2;
    case "zip":
    case "rar":
    case "7z":
      return FileArchive;
    default:
      return File;
  }
}

/** 裸 base64 → 字节（大附件分块解码，避免一次性栈压力）。 */
export function base64ToBytes(base64: string): Uint8Array<ArrayBuffer> {
  const bin = atob(base64);
  const bytes = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
  return bytes;
}

/** base64 → Blob → 触发系统保存对话框。 */
export function saveBase64File(fileName: string, base64: string): void {
  const blob = new Blob([base64ToBytes(base64)], {
    type: "application/octet-stream",
  });
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = fileName;
  document.body.appendChild(a);
  a.click();
  a.remove();
  URL.revokeObjectURL(url);
}

/**
 * 附件下载统一出口：download_attachment → base64 → 本地保存。
 * 成功返回文件名（调用方提示「已开始下载 fileName」）；
 * 失败抛出命令返回的中文文案（调用方展示错误态）。
 */
export async function downloadAndSaveAttachment(url: string): Promise<string> {
  const r = await downloadAttachment(url);
  if (!r.success || !r.data) throw new Error(r.message ?? "附件下载失败");
  saveBase64File(r.data.fileName, r.data.base64);
  return r.data.fileName;
}
