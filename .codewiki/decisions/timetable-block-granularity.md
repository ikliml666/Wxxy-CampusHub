---
title: 课表网格保持大节行粒度（P0 样式对齐否决小节行方案）
type: decision
source_files:
  - tauri-app/frontend/src/panels/TimetablePanel.tsx
tags:
  - timetable
  - ui
---

上游 shiguangschedule 安卓端按小节（45min/节）渲染网格；本仓 P0 样式对齐（2026-09-19）评估了
「按小节行渲染」方案后仍保持大节行，只对齐其视觉要素（时间列起止两行、课程块课名/教师/@教室、
点空白格新建课程）。理由：

1. `TimetableView.slots` 行数不固定（自定义作息，见 [[timetable-editable-slots|作息时间表可编辑]]），
   「45+10+45 拆小节」的时间推导只在内置 100 分钟大节表下成立，自定义作息下会推导出错误时间，
   必须引入渲染分支；
2. 本仓全链路已有大节数据/小节存储双口径（`blockOf = ceil(小节/2)` 换算点集中），再引入第三套
   「视觉小节」坐标在 `isCustomTime`（半节课，P5）落地前无收益，届时再评估。

完整两案权衡与三项 UI 变更规格见 `docs/superpowers/plans/2026-09-18-m2.5-timetable.md` §6；
HANDOFF 原始建议（A 案）见 `docs/HANDOFF-timetable-parity.md` P0 节。
