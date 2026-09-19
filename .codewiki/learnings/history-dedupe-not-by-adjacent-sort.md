---
title: 历史去重不能靠「排序后看相邻」（同 id 会被别的房间插在中间）
type: learning
source_files:
  - tauri-app/src-tauri/src/infra/electricity_history.rs
  - tauri-app/src-tauri/src/commands/electricity.rs
tags:
  - electricity
  - history
  - dedupe
  - rust
  - m4
---

# 历史去重不能靠「排序后看相邻」（同 id 会被别的房间插在中间）

M4 批 2 写 `infra/electricity_history.rs::normalize` 时先写了「按 `(date, collectedAt, id)` 排序 → 看相邻同 id 就去重」，单测当场抓住：**同 id 的两条会被同期别的房间的记录插在中间**而不相邻。

反例（真实测试数据）：房间 101 在 9-19 采了两次（09:00 与 21:00），房间 102 在 9-19 的 09:05 采了一次。按时间排序得到

```
101@9-18 09:00 | 101@9-19 09:00 | 102@9-19 09:05 | 101@9-19 21:00
                 ↑ 同 id 的两条不相邻 ↓
```

⇒ 只比较「前一条」的去重逻辑完全失效（该合并的两条都留下了）。修法：**先按 `id` 分组去重、再按时间排序**（两趟排序）：

```rust
entries.sort_by(|a, b| a.id.cmp(&b.id).then_with(|| cmp_keep_key(a, b)));
// 同 id 相邻 ⇒ 每组只留最后一条（取舍键最大者）
out.sort_by(|a, b| (date, collected_at, id).cmp(...));  // 时间升序给图表
```

## 顺带定下的取舍键纪律

「同 id 保留哪一条」必须**只依赖记录本身、与遍历顺序无关**，否则多端合并失去可交换性与幂等性。故取舍键取 `(collectedAt, source, 分制余额, 原文)` 全字段（同 `collectedAt` 的记录用后续字段定序），而不是只比 `collectedAt`。单测覆盖 `merge(a,b) == merge(b,a)` 与 `merge(x,x) == x`。

## 同一轮抓到的另一个坑：`SavedRoom.id` 毫秒会撞号

`commands/electricity.rs` 原来用 `now_ms()` 给房间生成 id，**同一毫秒内连续新增两个房间会得到同一个 id**。单测里批量建房间时必然命中（`assert_ne!(a, b)` 直接失败）；真实场景两秒内点两次「保存」也可能撞。危害不是重复条目（去重按 `roomKey`），而是 **`id` 是绑定（`set_bound`）/删除（`remove_room`）的定位键** ⇒ 撞号会让「解绑/删除隔壁」误伤另一个房间。修法 `next_room_id(rooms)`：撞号就顺延（`while rooms.iter().any(|r| r.id == n) { n += 1 }`）。

写法上的一点普适教训：**「按 X 去重」的实现必须先按 X 分组，而不是先按别的键排序后指望 X 相邻**；以及**任何被当作定位键的 id 生成都要对照现有集合查重**（毫秒/秒级时间戳都不保证唯一）。

## 相关

[[decisions/electricity-daily-snapshot-and-merge|电费日快照与多端合并决策]]、[[learnings/synjones-charge-yuan-vs-fen-and-pending-orders|元/分口径与待支付单旧结论修订]]。
