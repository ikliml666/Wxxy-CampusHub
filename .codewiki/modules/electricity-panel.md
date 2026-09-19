---
title: 电费页（前端）
type: module
source_files:
  - tauri-app/frontend/src/panels/PowerPanel.tsx
  - tauri-app/frontend/src/components/ElectricityTrendCard.tsx
  - tauri-app/frontend/src/components/ElectricityPaymentsCard.tsx
  - tauri-app/frontend/src/components/RechargeFlow.tsx
tags: [frontend, electricity, recharge, charts, cascade]
---

# 电费页（前端）

协议与数据来源见 [[modules/campus-synjones|慧新E校协议]]，自采与合并见 [[decisions/electricity-daily-snapshot-and-merge|电费日快照与合并]]，本轮重设计的取舍见 [[decisions/electricity-panel-redesign|电费页重设计决策]]。

## 布局（M4 批 3 重设计）

`PowerPanel.tsx:421`：容器 `mx-auto mt-8 max-w-5xl px-4`，下方 `grid items-start gap-3 lg:grid-cols-[minmax(0,1fr)_340px] lg:gap-4`。**左列**=缴费片区 / 查询房间余额（含结果）/ 充值，**右列**=电费变化 / 缴费记录 / 常用房间。窄屏栅格塌成单列、右列内容顺移到左列之后。栅格轨道必须 `minmax(0,1fr)`、卡内文本 `min-w-0`，否则长房间名会撑破布局。

底部留白由外壳统一提供（`AppShell.tsx:119` `<main className="pb-28">`），面板自身不加。

## 级联：校区自动选定与三道防死循环闸

`PowerPanel.tsx:166-182`（在 `runQuery` 内）：拿到响应后若 `!isFinal && options.length === 1` 且 **path 末段尚未等于该唯一选项** 且 `path.length < MAX_CASCADE_STEPS`，就自动追加该步并递归续查。

- 判据**只有「选项数 == 1」**，绝不写死层名（学校加校区后自动退回手动选）。
- 三道闸：① 自动选定的 path 严格增长一级；② 末段已是该选项时不再自动选（防服务端同层重发）；③ `MAX_CASCADE_STEPS = 10` 兜底。
- 自动选定的层记进 `autoLevels`，在面包屑渲染为**非交互弱文本**「校区 · 无锡国际校区（自动）」，不占交互位。

楼栋是**卡片网格**（单击即进，带「上次查过」弱角标），房间号是自动聚焦输入框 + 回车提交 + 「最近查过」chips。**刻意不做屏幕数字小键盘**（桌面端物理键盘更快）。

## 结果卡

- 主数字读 `ElectricityView.balanceYuan`（后端 `charge::balance_from_text` 严格提取；`null` ⇒ 显示「无数据」，**绝不用 0 顶替**）。**前端不解析 `fields` 自由文本**——实测原文 `当前余额837.47元,当前剩余电量1550.87度` 同时含金额与 kWh，两处各写一份解析必然漂移。
- 明细行走 `fieldLines`（按分隔符折行）/ `isNegativeLine`（含负数标红）两个导出函数，通用渲染、不解构。
- 行动按钮组：去官网充值 / 立即采集（`run_electricity_snapshot`）/ 绑定为我的宿舍（`bind_electricity_room`）。

## 三张数据卡

- **电费变化**（`ElectricityTrendCard.tsx`）：内联 `<svg viewBox="0 0 100 40" preserveAspectRatio="none">` + 每段连续有效点一条 `<polyline vectorEffect="non-scaling-stroke">`，零图表依赖。`balance === null` 的点只占 x 位、**断开线段**（不插值/不连缺口/不画 0）；有效点 < 2 个给矮提示条空态（`ElectricityTrendCard.tsx:78-107`），**不画假线**。日均消耗按首末差 ÷ 天数跨度（缺口日不参与但跨度保留），预估可用天数 = 余额 ÷ 日均。
- **缴费记录**（`ElectricityPaymentsCard.tsx`）：未支付订单（`get_electricity_orders({status:0})`）**置顶**并给「取消订单」（复用 `recharge_cancel`，无阻塞确认框）——这让批 C 遗留的未支付单可在应用内清掉，「进充值前检查遗留订单」防线由此生效。月度缴费默认只列**有缴费的月份 + 全年合计**，其余折叠进「展开全年 12 个月」（`ElectricityPaymentsCard.tsx:163-165,203-282`）；零值必须在文案上写明是「无缴费（0 元）」而非取数失败。`get_electricity_monthly` 一次调用发 12 个串行请求并持 token 全局锁，**必须给独立 loading**，不能拖住整页。
- **常用房间**：本地 JSON（`get/save/delete_electricity_rooms`），绑定项置顶标「我的宿舍」。

## 充值卡（RechargeFlow）

`RechargeFlow.tsx` 自 M3.1 起不变，仅两处文案/边距调整：密码提示为「请输入校园卡查询密码（6 位，**默认为身份证后六位**，点满自动提交）」（`RechargeFlow.tsx:495`）。风险声明 5 条常驻；客户端只转发键位下标序列、不接触真实卡密码（见 [[modules/campus-synjones|慧新E校协议]] 的充值一节）。

## 契约陷阱

- IPC 键名必须与 Rust `snake_case` 转出的 camelCase 完全一致（`feeitemId` / `roomId` / `orderId`）——写成 `feeItemId` 会被 Tauri 判为缺参、建单根本发不出去（M3.1 真机点验才抓到）。
- `types.ts` 里有两个 `balanceYuan`：`EcardOverview.balanceYuan`（一卡通）与 `ElectricityView.balanceYuan`（宿舍电费），语义不同别串。
- `SavedRoom.bound` 在 TS 里是**可选**字段（后端 `serde(default)` 兼容旧 JSON），保存房间**不改变绑定**，绑定只经 `bind_electricity_room`。
