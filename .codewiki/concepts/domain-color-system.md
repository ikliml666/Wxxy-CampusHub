---
title: 域色编码系统（domain-color-system）
type: concept
source_files:
  - tauri-app/frontend/src/index.css
  - tauri-app/frontend/src/components/DockNav.tsx
  - tauri-app/frontend/src/panels/TodayPanel.tsx
  - tauri-app/frontend/src/panels/LoginPanel.tsx
  - tauri-app/frontend/src/components/ui/button.tsx
  - tauri-app/frontend/src/components/ui/card.tsx
tags:
  - design
  - color
  - tailwind
  - theme
---

# 域色编码系统（domain-color-system）

前端用「品牌色 + 语义域色」双层体系：每个功能域固定一种颜色，全站（Dock、卡片角标、错误提示、品牌元素）只从这套变量取色，不散落硬编码色值。定义于 Tailwind v4 `@theme` 块（`tauri-app/frontend/src/index.css:6-20`），Tailwind 自动为每个 `--color-*` 变量生成同名工具类（`bg-brand`、`text-alert`、`bg-wallet` 等）。

## 取值与用途（浅色主题，index.css:6-20）

| Token | 值 | 语义域 | 用途 |
|---|---|---|---|
| `--color-brand` | `#5b2e90` | 锡院紫（品牌） | 登录按钮、品牌 logo 块、focus ring、今日域 |
| `--color-bg` | `#f7f6f9` | 纸灰 | 页面底色 |
| `--color-surface` | `#ffffff` | 卡片面 | 卡片/面板背景 |
| `--color-text` / `--color-text-2` | `#1f1b29` / `#6e6878` | 墨紫黑 / 次要文字 | 文字两级 |
| `--color-line` | `#e8e5ee` | 1px 边线 | 卡片描边 |
| `--color-wallet` | `#0f9d77` | 域绿 | 一卡通/余额/电费（钱包域） |
| `--color-info` | `#7c5cbf` | 域紫 | 资讯/公告 |
| `--color-todo` | `#d9822b` | 域琥珀 | 待办 |
| `--color-sched` | `#2b7dbf` | 域湖蓝 | 日程/会议/课表（应用面板同用 sched） |
| `--color-alert` | `#d9455b` | 告警红 | 低余额/错误提示（兼 destructive） |

字体亦是 token：`--font-sans`（Segoe UI / Microsoft YaHei UI 系）与 `--font-num`（Outfit，数字/金额等宽显示，OFL 许可随包分发，`index.css:18-19,61-81`；`.tabular-num` 工具类 `index.css:58`）。

## 深色主题提亮档（`.dark`，index.css:44-56）

深色下**每个域色整体提亮一档**（注释标注源自设计文档 §6 自评）：brand `#8b5cf6`、wallet `#2ec49a`、info `#a48bd9`、todo `#eda45c`、sched `#5da3e0`、alert `#e56b7d`；bg/surface/text/text-2/line 同步换深色档。域色只在 `.dark` 作用域内被覆盖，工具类无需写深色变体——这是 token 化的核心收益。

## 映射到 shadcn 语义变量（:root，index.css:22-42）

`components/ui/*`（shadcn 源码入库）只消费 shadcn 语义名，`index.css:22-42` 把语义名指向域色 token，完成两套体系的桥接：

- `--primary/--ring` → `var(--color-brand)`；`--destructive` → `var(--color-alert)`；
- `--background` → `var(--color-bg)`、`--card` → `var(--color-surface)`、`--border/--input` → `var(--color-line)`、`--muted-foreground` → `var(--color-text-2)` 等；
- `--radius: 10px` 统一圆角；`@custom-variant dark`（`index.css:4`）使 `dark:` 变体基于 `.dark` class 生效（配合 AppShell 的 `classList.toggle("dark")`）。

因此改一处 `@theme` 值，shadcn 组件与自研组件同时变色。

## 应用规则

- **Dock 导航**（`DockNav.tsx:25-39`）：每项 `color` 直接引用域色 CSS 变量（`"var(--color-brand)"` 等）；激活胶囊背景用 `color-mix(in srgb, ${item.color} 14%, transparent)`、底点与图标着色用同一变量（`DockNav.tsx:163-166,183`）——域色变量在深色下自动切换，Dock 无需额外处理。
- **卡片域色角标**：内容卡左上角 2×2 圆角小方块用所属域色标记归属，如钱包三卡 `bg-wallet`（`TodayPanel.tsx:64-67`）、下一节课横幅 `bg-sched`（`TodayPanel.tsx:79-81`）。
- **告警/错误**：表单错误红字用 `text-alert`（`LoginPanel.tsx:191`）；经 `--destructive` 映射同样供 shadcn 危险态使用。
- **品牌元素**：登录页 logo 块 `bg-brand`、主按钮 `bg-brand text-white`（`LoginPanel.tsx:120-123,196-199`）。
- 设置域（settings）不设专属域色，Dock 上用中性 `text-2`（`DockNav.tsx:38`）。

## 设计文档

完整设计依据（含 §6 深色提亮自评）见 `docs/design/frontend-design.md`；域色与面板的对应关系同时冻结在 `docs/superpowers/plans/2026-09-17-m0-m1-foundation.md` 的 Task 4。
