---
title: "Tailwind v4：shadcn 语义 token 不写 @theme 就没有工具类"
type: learning
source_files:
  - tauri-app/frontend/src/index.css
  - tauri-app/frontend/src/components/ui/button.tsx
tags:
  - tailwind
  - css
  - shadcn
  - debugging
---

# Tailwind v4：shadcn 语义 token 不写 `@theme` 就没有工具类

外壳重设计时发现：`<Button>`（shadcn 源码入库，`className` 含 `bg-primary text-primary-foreground`）**主按钮长期渲染成裸文字**——无底色、白字落在浅背景上。深浅主题切换、重启、清缓存均无改善。

## 根因

shadcn 语义变量（`--primary`/`--background`/`--card`/`--border`/`--accent`/`--muted`/`--ring`/`--destructive` 等）当初只写在 `:root {}` 里指向域色 token。**Tailwind v4 的工具类只从 `@theme` 块 / `--color-*` 命名空间生成**；`:root` 里的普通 CSS 自定义属性只是运行时变量，Tailwind 扫描时根本不知道 `bg-primary` 该生成什么——于是样式表里**从来没有过 `.bg-primary` 这条规则**。域色自研类（`bg-brand`/`bg-wallet`）一直正常，恰因它们定义在 `@theme` 块内，反过来掩盖了语义类全灭的事实。

## 症状与取证

1. **构建产物 grep**：对 dist 产物 CSS `grep "bg-primary"`——无命中即坐实「类从未生成」，与运行时覆盖/优先级问题无关（那是「规则存在但被打败」，grep 一定有命中）。
2. **浏览器计算样式**：DevTools 对按钮 `getComputedStyle(el).backgroundColor` 返回 `rgba(0, 0, 0, 0)`（透明），且 Elements 面板里 `bg-primary` 类名旁**不显示来源规则**——类挂在元素上但样式表没有对应规则，是「未生成」的直接证据。
3. 反向验证：`bg-brand` 命中且着色正常 → 问题精确隔离在「语义名 → 工具类」这一跳。

## 修法：`@theme inline` 映射

`index.css:60-79` 新增 `@theme inline {}`，把 shadcn 语义名映射到域色 token：

```css
@theme inline {
  --color-primary: var(--color-brand);
  --color-card: var(--color-surface);
  --color-destructive: var(--color-alert);
  /* … 其余语义名同理 */
}
```

两个关键点：

- **必须用 `inline`**：非 inline 时 Tailwind 会在 `:root` 级生成 `--color-primary: var(--color-brand)` 定义并按此解析，`.dark` 里重定义 `--color-brand` 时语义类**不会跟随切换**（深色主题失灵）；`inline` 把 `var()` 值直接内联进每条工具类，`.dark` 覆盖域色后语义类自动跟随。
- **`:root` 里的同名普通变量保留**（`index.css:81-101`）：供直接引用 `var(--primary)` 的内联样式场景，两套并存不冲突。

## 沉淀规则

1. Tailwind v4 里「定义了 CSS 变量」≠「有了工具类」——工具类只来自 `@theme`/`--color-*` 命名空间；引入任何 shadcn 组件前先确认其语义名已在 `@theme` 映射。
2. 「组件看起来完全没样式」先 grep 构建产物再查浏览器：规则不存在（未生成）与规则被打败（优先级）是两条不同的排查路。
3. 语义类映射到另一组会随主题切换的变量时，必须 `@theme inline`；这是深色主题能跟随的前提。

## 相关

- [[concepts/domain-color-system|域色编码系统]]
- [[modules/frontend-shell|前端外壳]]
