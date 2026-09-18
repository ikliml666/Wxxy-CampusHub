---
title: 域色编码系统（domain-color-system）
type: concept
source_files:
  - tauri-app/frontend/src/index.css
  - tauri-app/frontend/src/components/DockNav.tsx
  - tauri-app/frontend/src/components/PanelHeader.tsx
  - tauri-app/frontend/src/components/Surface.tsx
  - tauri-app/frontend/src/components/LoginDialog.tsx
  - tauri-app/frontend/src/panels/TodayPanel.tsx
  - tauri-app/frontend/src/components/ui/button.tsx
  - tauri-app/frontend/src/components/ui/card.tsx
tags:
  - design
  - color
  - tailwind
  - theme
---

# 域色编码系统（domain-color-system）

前端用「品牌色 + 语义域色」双层体系：每个功能域固定一种颜色，全站（Dock、卡片角标、错误提示、品牌元素）只从这套变量取色，不散落硬编码色值。定义于 Tailwind v4 `@theme` 块（`tauri-app/frontend/src/index.css:6-54`），Tailwind 自动为每个 `--color-*` 变量生成同名工具类（`bg-brand`、`text-alert`、`bg-wallet` 等）。**域色系统是签名、语义命名是别名**：shadcn 组件消费的 `bg-primary`/`bg-card` 等只是域色 token 的映射别名（见下文 `@theme inline` 一节）。

## 取值与用途（浅色主题，index.css:6-20）

| Token | 值 | 语义域 | 用途 |
|---|---|---|---|
| `--color-brand` | `#5b2e90` | 锡院紫（品牌） | 登录按钮、品牌 logo 块、focus ring、今日域 |
| `--color-bg` | `#f7f6f9` | 纸灰 | 页面底色 |
| `--color-surface` | `#ffffff` | 卡片面 | 卡片/面板背景 |
| `--color-surface-2` | `#fbfafc` | 卡片内嵌区/骨架底 | 菜单折叠区、验证码区、空态底 |
| `--color-text` / `--color-text-2` | `#1f1b29` / `#6e6878` | 墨紫黑 / 次要文字 | 文字两级 |
| `--color-line` | `#e8e5ee` | 1px 边线 | 卡片描边 |
| `--color-line-strong` | `rgb(31 27 41 / 0.10)` | hover 加深边线 | 胶囊/按钮 hover 态 |
| `--color-wallet` | `#0f9d77` | 域绿 | 一卡通/余额/电费（钱包域） |
| `--color-info` | `#7c5cbf` | 域紫 | 资讯/公告（兼品牌渐变副色） |
| `--color-todo` | `#d9822b` | 域琥珀 | 待办 |
| `--color-sched` | `#2b7dbf` | 域湖蓝 | 日程/会议/课表（应用面板同用 sched） |
| `--color-alert` | `#d9455b` | 告警红 | 低余额/错误提示（兼 destructive） |

品牌渐变 `linear-gradient(140deg, var(--color-brand), var(--color-info))` 是品牌方标与无头像占位的统一记号（`AppShell.tsx:60`、`LoginDialog.tsx:215`、`Avatar.tsx:45`）。

## 排版 / 圆角 / 阴影 / 动效 token（index.css:22-50）

2026-09-18 外壳重设计扩展的四组 token（与域色同住 `@theme` 块）：

- **字号阶** `--text-display`(28px) / `--text-display-s`(22px) / `--text-title`(18px) / `--text-body`(14px) / `--text-caption`(12px)，各带 `--line-height`（display 系另有负字距）（`index.css:22-35`）——生成 `text-display`、`text-title` 等排版工具类，组件不再写裸 px 字号。
- **圆角阶** `--radius-card`(14px) / `--radius-inner`(10px) / `--radius-control`(8px)（`index.css:37-40`），生成 `rounded-card/inner/control`；shadcn 用的 `--radius: 10px` 保留在 `:root`。
- **域色染色阴影** `--shadow-card` / `--shadow-lift` / `--shadow-pop`（`index.css:42-45`）：环境影用**品牌紫染**（`rgb(91 46 144 / …)`）而非纯黑，弹层影（pop）中性更深。生成 `shadow-card/lift/pop` 工具类。
- **动效 token** `--ease-out-soft`（cubic-bezier(0.22,1,0.36,1)）与 `--dur-fast`(120ms) / `--dur-base`(180ms)（`index.css:47-50`），供 `duration-[var(--dur-fast)] ease-out-soft` 形式引用；`@layer base` 里统一 `:focus-visible`（品牌色 2px outline，`index.css:146-149`）与 `prefers-reduced-motion` 降级（过渡压到 ≤150ms 只留 opacity，`index.css:151-157`）。另有可选噪点覆盖层工具类 `.grain`（纯内联 SVG 2% 透明度，`index.css:134-142`）与 body 顶部域色径向氛围洗（≤5%，`index.css:123-130`）。

字体亦是 token：`--font-sans`（Segoe UI / Microsoft YaHei UI 系）与 `--font-num`（Outfit，数字/金额等宽显示，OFL 许可随包分发，`index.css:52-53,161-181`；`.tabular-num` 工具类 `index.css:131`）。

## 深色主题提亮档（`.dark`，index.css:103-121）

深色下**每个域色整体提亮一档**（注释标注源自设计文档 §6 自评）：brand `#8b5cf6`、wallet `#2ec49a`、info `#a48bd9`、todo `#eda45c`、sched `#5da3e0`、alert `#e56b7d`；bg/surface/text/text-2/line/surface-2/line-strong 同步换深色档。阴影换冷色系深档（黑基 + 深紫染减淡，避免灰底上发脏，`index.css:117-120`）。域色只在 `.dark` 作用域内被覆盖，工具类无需写深色变体——这是 token 化的核心收益。

## 映射到 shadcn 语义变量：`@theme inline` 才是工具类来源（index.css:56-79）

`components/ui/*`（shadcn 源码入库）只消费 shadcn 语义工具类名（`bg-primary`/`bg-card`/`border-border` 等）。**Tailwind v4 只认 `@theme`/`--color-*` 命名空间**——只在 `:root` 写 `--primary: …` 普通变量不会生成任何工具类（曾导致主按钮长期无底色，根因与取证见 [[learnings/tailwind-v4-shadcn-token-mapping|Tailwind v4 语义 token 映射]]）。修法是 `@theme inline {}`（`index.css:60-79`）把语义名映射到域色 token：

- `--color-primary` → `var(--color-brand)`、`--color-destructive` → `var(--color-alert)`、`--color-border/--color-input` → `var(--color-line)`、`--color-muted-foreground` → `var(--color-text-2)` 等；
- **必须 `inline`**：值以 `var()` 内联进工具类，`.dark` 里重定义 `--color-brand` 等域色时语义类才能跟随切换（非 inline 会在 `:root` 处定值、深色失灵）；
- `:root` 里的同名普通变量（`--primary`/`--card`/`--radius` 等，`index.css:81-101`）**保留**，供直接引用 `var(--primary)` 的场景（如 shadcn 部分组件内联样式）。

因此改一处 `@theme` 域色值，shadcn 组件与自研组件同时变色。

## 应用规则

- **Dock 导航**（`DockNav.tsx:25-39`）：每项 `color` 直接引用域色 CSS 变量（`"var(--color-brand)"` 等）；激活胶囊背景用 `color-mix(in srgb, ${item.color} 14%, transparent)`、底点与图标着色用同一变量——域色变量在深色下自动切换，Dock 无需额外处理。
- **共享组件域色参数**：`PanelHeader.tsx:4-20` 定义 `PanelDomain` 类型与 `domainVar`（域色 → CSS 变量）映射，`EmptyState` 图标圆底与 `Surface` 左上角 8px 角块据此内联取色（`EmptyState.tsx:35-42`、`Surface.tsx:26-32`）——深浅主题自动跟随。
- **告警/错误**：表单错误红字用 `text-alert`（`LoginDialog.tsx:304`）；经 `--destructive` 映射同样供 shadcn 危险态使用。
- **品牌元素**：登录弹层 logo 块与主按钮、顶栏品牌块均为品牌渐变方标 + 品牌主按钮（`LoginDialog.tsx:210-218`、`AppShell.tsx:54-64`）。
- 设置域（settings）不设专属域色，Dock 上用中性 `text-2`（`DockNav.tsx:38`）；`PanelDomain` 的 `neutral` 档同义（`PanelHeader.tsx:19`）。

## 设计文档

完整设计依据（含 §6 深色提亮自评）见 `docs/design/frontend-design.md`；域色与面板的对应关系同时冻结在 `docs/superpowers/plans/2026-09-17-m0-m1-foundation.md` 的 Task 4。
