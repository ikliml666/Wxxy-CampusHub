# Wiki Index

## Concept

- [[concepts\domain-color-system|域色编码系统（domain-color-system）]]
## Decision

- [[decisions\ecard-panel-merge|一卡通面板合并决策（钱包 + 电费 → ecard）]]
- [[decisions\timetable-editable-slots|作息时间表可编辑（save_time_slots 与 effective_slots 单点取值）]]
- [[decisions\recording-jar-session|决策记录 - RecordingJar 自实现与会话持久化]]
- [[decisions\guest-mode-account-shell|决策记录 - 游客优先外壳、账号系统与头像三态]]
- [[decisions\electricity-daily-snapshot-and-merge|电费日快照自采与多端合并（不做计划任务、不做网络层）]]
- [[decisions\electricity-panel-redesign|电费页重设计：双列布局、校区自动选定与零依赖图表]]
- [[decisions\timetable-diff-manual-and-ics|课表 diff、手动课程与 ICS 导出决策]]
- [[decisions\timetable-occurrence-expansion|课表生效实例展开（expand_occurrences 与 ICS 按生效结果导出）]]
- [[decisions\timetable-block-granularity|课表网格保持大节行粒度（P0 样式对齐否决小节行方案）]]
- [[decisions\timetable-view-contract|课表视图契约（TimetableView 下发 slots 与 currentWeek）]]
- [[decisions\timetable-notice-l1l2|调课通知 L1/L2 分级口径与 noticeId 取舍]]
- [[decisions\ecard-transfer-removed|账户转账功能删除决策（官方全站无转账操作界面）]]
## Learning

- [[learnings\cas-sso-plaintext-redirect|CAS SSO 回跳的三处坑：重定向中断、明文落点、探测误判]]
- [[learnings\tailwind-v4-shadcn-token-mapping|Tailwind v4：shadcn 语义 token 不写 @theme 就没有工具类]]
- [[learnings\tauri-multiwindow-cdp-verification|Tauri 多窗口应用的 CDP 点验：必须按 URL 选 target（附非主窗口 IPC 权限验证法）]]
- [[learnings\tauri-webview-ui-verification|Tauri/WebView2 真机 UI 验收：vite 供旧模块与点击注入失效的可用替代路径]]
- [[learnings\ecard-write-protocol-json-body|一卡通写操作协议实测（JSON body / 双层判定 / 三套金额单位）与转账结论]]
- [[learnings\ecard-stats-params-and-secure-keyboard|一卡通统计参数实测与安全键盘机制]]
- [[learnings\meeting-proxy-week-title-and-observable-degradation|会议代理端点与静默降级可观测化]]
- [[learnings\history-dedupe-not-by-adjacent-sort|历史去重不能靠「排序后看相邻」（同 id 会被别的房间插在中间）]]
- [[learnings\ecard-keyboard-pseudochar-protocol|安全键盘伪字符映射协议（键面必须渲染图片）]]
- [[learnings\official-ecard-packet-capture-parity|官方一卡通动态抓包对照与对接修复（unlostCard 大小写 / flag 档位 / 账户 value 重复）]]
- [[learnings\cwxu-official-site-content-extraction|官网正文抓取与鉴权门降级设计（content.jsp 系不可抓）]]
- [[learnings\portal-app-catalog-and-icons|应用目录、图标代拉与 appLink 校验分工（受保护资源与两种白名单）]]
- [[learnings\synjones-charge-yuan-vs-fen-and-pending-orders|慧新E校 charge 侧金额是元、一卡通侧是分；待支付单「恒 500」旧结论已推翻]]
- [[learnings\subagent-batch-image-labeling|批量图片标注不可靠：改结构化拼图 + 客观特征交叉校验]]
- [[learnings\zhengfang-tiaoxiu-swap-entries|教务调休条目形态与公告置换冗余]]
- [[learnings\zhengfang-kblist-multi-section-per-jxb|正方 kbList 同一教学班按多时段拆多条——diff 匹配键必须含时段]]
- [[learnings\jwglxt-sso-chain|正方教务 SSO 链与课表接口取证（ST 绑定 service / TGT 不落 cookie / 901 会话特征）]]
- [[learnings\kaptcha-arithmetic-five-roots|算术验证码识别：从 2.9% 到 100% 的五个根因]]
- [[learnings\error-text-url-credentials|错误文案回显 URL 凭据——reqwest 错误串泄票据与 redact_secrets 脱敏]]
- [[learnings\portal-session-expiry-200-envelope|门户会话失效是 HTTP 200 信封而非错误码（误分类成「解析失败」的连锁）]]
- [[learnings\portal-block-periods-and-school-timetable|门户大节语义与校本作息（14:50 误报教训）]]
- [[learnings\portal-avatar-upload-protocol|门户头像上传协议与裁切器踩坑]]
## Module

- [[modules\campus-auth|CAS 协议核心（campus-auth）]]
- [[modules\ecard-panel|一卡通页（前端 + 命令面，M4.5）]]
- [[modules\frontend-shell|前端外壳（frontend-shell）]]
- [[modules\campus-synjones|慧新E校协议核心（campus-synjones）]]
- [[modules\campus-hub-tauri|接线层（campus-hub src-tauri）]]
- [[modules\electricity-panel|电费页（前端）]]
- [[modules\campus-schedule|课表核心（campus-schedule）]]
- [[modules\campus-portal|门户业务协议核心（campus-portal）]]
