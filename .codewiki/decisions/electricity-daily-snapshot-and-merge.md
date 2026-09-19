---
title: 电费日快照自采与多端合并（不做计划任务、不做网络层）
type: decision
source_files:
  - tauri-app/src-tauri/src/infra/electricity_history.rs
  - tauri-app/src-tauri/src/commands/electricity_history.rs
  - crates/campus-synjones/src/turnover.rs
tags:
  - electricity
  - history
  - merge
  - offline-first
  - m4
---

# 电费日快照自采与多端合并（M4 批 2，2026-09-19）

## 背景

用户要「每天定时查询采集 + 电费变化与统计」。探针实测（[[modules/campus-synjones|慧新E校协议核心]] §三）先把学校的供给钉死：`/charge/turnover/*` 只给「缴了多少」（账单/月度合计/累计额），`balance_amount` 恒 null、`threeExpen_account` 恒空 ⇒ **宿舍电费的每日余额时间序列在学校侧不存在**，只能客户端自采。同时用户明确要考虑「将来安卓端如何通过校园网内网同步」。

## 决策

1. **自采形态 = 应用运行时补采 + 手动采集，不注册 Windows 计划任务**。理由：慧新E校 token **单活**（同账号只有最新一次 SSO 签发的 token 有效），后台任务拿 token 会顶掉用户正在用的会话；计划任务还要绕 UAC 与凭据落盘。代价如实接受：**应用没开的日子就是空档**，图表留空（不插值、不补零——补零会把「没采到」画成「余额 0」）。
2. **采集对象恒为「绑定的宿舍房间」**（`SavedRoom.bound`，同一时刻最多一个）：未绑定就返回可读中文原因而不是偷偷采第一个常用房间；绑定关系只经 `bind_electricity_room` 改，保存房间（改名/刷新元信息）**沿用存量绑定**，不因前端没带 `bound` 字段而静默丢绑定。
3. **记录形状为「跨端可合并」而设计**（本轮只落数据结构，不做网络层）：
   - `id = roomKey@date`——**确定性推导**，跨端一致 ⇒ 合并只需按 id 去重，不需要时钟同步、向量时钟或版本号；
   - `roomKey` = 片区 id + 级联路径的 `level=value`（用 `value` 而非 `code`：`value` 自带服务端数字 id，参数名才是会漂移的那一层）；
   - `device` 只记「哪台机器观察到的」，**不参与 id**（同一天两台机器采同一房间是同一个观测点）；
   - 去重键 = `roomKey` + 日期，同日重复采集**覆盖**（保留取舍键最大者）。
4. **不落盘的：** token / 账号 / cookie / 户号。落盘只有 `map.showData` 那句余额原文与房间路径（`map.data` 的户号在 crate 层就不透出）。

## 合并语义与已知局限

`merge_history(a, b)` = 追加后 `normalize`：**按 id 去重（保留取舍键最大者）→ 时间升序 → 上限裁剪**。取舍键 = `(collectedAt, source, 分制余额, 原文)`，**全字段参与是刻意的**——只比 `collectedAt` 时，两条 `collectedAt` 完全相同的记录会随遍历顺序而变，破坏「可交换 + 幂等」这两条多端同步的硬性质（有单测钉住 `merge(a,b) == merge(b,a)` 与重复合并幂等）。

局限（如实记录，将来做同步层时要面对）：
- 只解决「同一天同房间有两条」，**不解决**两台机器在同一天不同时刻采到不同余额的**取舍**问题——由 `collectedAt` 决定（取更晚的），不平均、不差分；
- **没有设备级冲突检测**：两端同时改同一天、且各自 `collectedAt` 都晚于对方时只保留一条，另一条观测丢失。日快照是「每天一个点」的形态，丢一条不影响趋势；真需要逐次观测精度时应改成 append-only 事件流；
- 学校**改楼栋名**会改 `roomKey` 的文本部分 ⇒ 历史断成两段（无法从现有数据区分「改名」与「换楼」）。接受该风险，**不做模糊匹配**（正是「不做文本解构」的同一条纪律）。

## 其它落点

- **保留上限 2000 条**（单房间一天一条 ≈ 5.5 年；每条约 300 字节 ⇒ 文件上限约 600KB，整体读写仍廉价），超出按时间丢最旧。
- 读取宽容语义与 `infra/timetable.rs`、`electricity_rooms.json` 一致：文件缺失/损坏 → 空列表 + `log::warn`，**不删坏文件**（保留现场）。
- 启动补采三道静默护栏：今日已采过 / 未绑定宿舍 / **当前无内存会话**（无会话时不进 SSO——一次后台补采不该去签发 token 顶掉用户的会话，这与「不注册计划任务」是同一条理由）。失败只 `log::warn`，日志只含错误文案与余额数值，无凭据。

## 相关

[[modules/campus-synjones|慧新E校协议核心]]、[[learnings/synjones-charge-yuan-vs-fen-and-pending-orders|元/分口径与待支付单旧结论修订]]、[[learnings/history-dedupe-not-by-adjacent-sort|历史去重不能靠排序后看相邻]]。
