# 更新日志

## 2026-09-21 · 应用内浏览器第一批：三入口改应用内 WebView 打开 + 注入优化（免密直达留第二批）

- **模块**：`tauri-app/src-tauri/src/{commands/browser.rs(新),browser_inject.js(新),commands/{electricity,mod}.rs,lib.rs,Cargo.toml}`、`tauri-app/frontend/src/{components/browser/{BrowserOverlay,BrowserToolbar,BrowserStatusView,BrowserEventsBridge}(新),components/{AppShell,ecard/EcardPowerView}.tsx,panels/{AppsPanel,InfoPanel}.tsx,stores/browserStore.ts(新),shared/{types.ts,tauriApi.ts,constants.ts(新)}}`、`crates/campus-portal/src/article.rs`（仅 pub(crate) 可见性，白名单零改动）
- **改动**：
  1. **三入口分流**：通知原文/门户应用目录/电费充值兜底改走 `decide_open` 纯函数三态——校园域（*.cwxu.edu.cn ∪ 10.3.100.110）进应用内 webview；域外（知网/万方/超星等 external 类）自动降级旧 `open_app` 系统浏览器；非 http/https 拒绝。白名单口径零扩大（`is_allowed_info_url` 红线未动，新增域后缀攻击单测）
  2. **主窗口内嵌第二 webview**（Tauri 2 multiwebview + `unstable` feature）：主 webview 缩顶 48px（React 工具栏渲染在此条内）、副 webview 占剩余区加载校方页，`WindowEvent::Resized` relayout；关闭即 `Webview::close()` 复原。spike 六问真机实锤（`.codewiki/learnings/inapp-webview-spike.md`）
  3. **注入优化**（browser_inject.js 按 host 分桶）：表格/图片防溢出、焦点环、正文居中、`target=_blank`/`window.open` 改应用内导航；alert 只收集不拦截（弹窗名单灰度制）；三禁（不自动填表/hook 官方校验/伪造 cookie）
  4. **前端**：browserStore（弹层态 + history persist 上限 20）+ BrowserOverlay 工具栏/进度条/blocked 提示条 + BrowserEventsBridge 常驻桥（`browser://opened` 事件 → `storeSyncOpen`，防"Rust 直建 webview 而前端无感"的无 chrome 死路——final review 抓出的 C1）+ 30s loading watchdog（含页内导航/刷新 started 回写场景，scoped re-review 补全）
  5. **操作简便化**：Esc/Alt+方向键、在外部浏览器逃生口、打开历史（persist）、CAS 登录页提示条（"登录一次后本应用将记住状态"——WebView2 profile 持久化）
- **验证**：`cargo test -p campus-hub --lib` **160 passed / 0 failed**（含 decide_open 四态 + 域后缀攻击）；`npx tsc --noEmit` + `npm run build` 零错误；CDP 真机实锤——InApp 打开（cas/webvpn 域）、**WebVPN 网关重定向链**（tsgcnki → webvpn 网关加密路径 → 网关登录页）、blocked 拦截 UI（"已拦截非校园网链接"）、Esc 关闭复原、resize bounds 贴合、电费入口工具栏完整（C1 修复后复验）。留用户验收：webview 内登录一次后的跨重启自持、通知 needsBrowser 真页、电费充值真页（有效片区 id）
- **已知问题/下批**：免密直达（CAS TGT 换 ST，本批最大收益项）、Esc 多层双关、导航栈事件启用、弹窗灰度名单、errorMsg 面板内 toast；详见 `.codewiki/modules/inapp-browser.md`

## 2026-09-21 · UI 改造轮：资讯附件内嵌 PDF 查看器 + 日程页重设计去课程重复 + 电费通知设置迁移

- **模块**：`crates/campus-portal/{src/{lib.rs,article.rs,client.rs}}`、`tauri-app/src-tauri/{src/{commands/portal.rs,lib.rs},tauri.conf.json}`；前端 `tauri-app/frontend/src/{panels/{SchedulePanel,InfoPanel,SettingsPanel}.tsx,components/{PdfViewerDialog(新),ecard/{EcardPowerView,EcardPowerNotifCard(新)}}.tsx,shared/{types.ts,tauriApi.ts,attachment.ts(新)},vite.config.ts,package.json}`
- **资讯附件 + 内嵌 PDF 查看器（GitHub 调研定案 react-pdf）**：
  - 后端：`get_info_detail` 新增 `attachments: { name, url }[]`——协议层 `extract_attachments`（`article.rs`）从正文容器抽取常见文件后缀 `<a>` 链接（相对 URL 补全、去重保序、链接文本为名），新增 `is_allowed_attachment_url`（`*.cwxu.edu.cn` + 精确 `10.3.100.110`，原 `is_allowed_info_url` 白名单不放宽、混淆域拒绝有单测）；新命令 `download_attachment`（命令数 105→106）：带 RecordingJar 会话代理下载（门户 cookie 自动携带，解决「cookie 在 Rust 侧前端 fetch 拿不到」）、双重 15MB 限流（content-length 预检 + 流式累计兜底）、返回 `{ fileName, base64 }`
  - 前端：详情页标题下新增附件区（空列表不渲染），类型图标按后缀映射；**PDF** 打开 `PdfViewerDialog` 弹层（`react-pdf@11` + `React.lazy` 按需 704KB 独立 chunk；大面积 `min(1100px,94vw)×90vh` 充分利用空间；工具条 = 文件名 + scroll-spy 页码 + 缩放 0.6–2.0 + 下载 + 关闭；Esc/遮罩关闭、焦点管理、锁背景滚动；全页顺序渲染 <50 页口径）；**非 PDF** 经 `downloadAndSaveAttachment` 统一出口 → Blob + `a[download]` 系统保存，按钮 idle/loading/done/error 四态完整
  - CSP 最小放行（tauri.conf.json）：`script-src` + `'wasm-unsafe-eval'`（pdf.js v4 扫描件 JPEG2000 WASM 解码）、新增 `worker-src 'self' blob:`、`img-src` + `blob:`；`connect-src` 未动（下载全走 Rust）；worker 用 `new URL(..., import.meta.url)` 产物化同源加载；CJK 中文 PDF 必配 cmaps（`vite-plugin-static-copy` v4 `rename: { stripBase: true }` 扁平拷 168 个 bcmap，旧选项 `structured` 已废弃）
- **日程页重设计（消除与课表页重复）**：
  - 移除课程：`isCourseEvent`（`classifyName === "课程" || classifyCode 含 "course"` 双判据）在周分桶/月计数/最近日程三处入桶前统一剔除，页头描述同步去掉「课程」；月视图角标不再调 `get_schedule_day_counts`，改由过滤后的 `get_schedule_month` 月明细前端自算（同源保证角标与列表口径一致；后端命令保留未删）
  - 布局修复「挤在一起」：容器 `max-w-3xl` → `w-full max-w-6xl`，周历 `overflow-x-auto` + 列最小 136px，事件块 `truncate` → `line-clamp-2`；详情卡星期文案改按事件自身日期，周历块与最近日程行共用
  - 下方自动显示「最近日程」：独立 fetch 今天起 30 天（同接口同过滤），按自然日分组升序（今天/明天/周X + 日期），行 = 域色竖条 + 标题 + 时间 + 地点 + 分类 chip，点击联动详情卡；今天无事件/30 天全空各有空态，loading 骨架/错误重试四态完整
- **电费通知设置迁移（设置页 → 一卡通 · 电费子页）**：开关/阈值/频率三行整体迁入新卡 `EcardPowerNotifCard`（电费子页底部，wallet 域色），阈值非数字/负数校验与整包保存契约原样保留；设置页 `NotifSettingsCard` 只剩公告/待办，save 构造 `{...settings}` 电费字段原样回传——两处编辑互不覆盖
- **验证**：`cargo check` 通过；`cargo test -p campus-portal` **55 passed / 0 failed**（含附件抽取新增单测）、`cargo test --lib`（src-tauri）**158 passed / 0 failed**（含白名单/文件名/percent-decode 新增 3 个）；`npm run build`（tsc -b && vite build）通过，cmaps 168 项就位、pdf.worker 与 PdfViewerDialog 均为独立 chunk；主包 940KB 与改造前相当
- **未验证（留真机）**：PDF 实际渲染（CSP wasm/worker 在 WebView2 生效、中文 CMap 效果）需登录后点开真实带附件通知；`a[download]` 在 WebView2 的保存行为；日程页与附件区视觉走查

## 2026-09-21 · M4/M5 真机验证轮：校外全链路探针实锤 + 通知闭环真触发 + 三处真实缺陷修复

- **模块**：`tauri-app/src-tauri/{src/{lib.rs,commands/{notification,auth,timetable}.rs},Cargo.toml,tests/webvpn_offcampus_live.rs(新)}`；无前端改动
- **验证结果（全部实测证据）**：
  - **M4 校外完整链路探针 PASS**（`webvpn_offcampus_live.rs`，校园网内即可跑）：CAS 登录 → `WebVpnSession::login` 深澜会话 → **`sso_token_via` 经网关跑 SSO 桥换到慧新E校 token（307 字节 bearer）** → `wrapped_client` GET `queryCurrentCard` 经网关 HTTP 200 → 业务信封 code=200 且 **retcode="0"**（网关换的 token 被一卡通业务层完整接受）→ 真实余额 70.46 元与批 14 真机记录吻合。此前「berserker API 带 synjones-auth 头经网关透传未验证」缺口就此补实锤；M4 仅剩 net_zone 判 OffCampus 后的自动触发待用户校外真机
  - **M5 通知闭环全链路 PASS**（CDP + 真实数据）：免密登录 → POLL_KICK 唤醒轮询 → **3 条未读**（2 条门户真实新公告，其中「干部选聘公告」为当晨 01:30 新发 + 1 条电费提醒「当前余额 829.63 元低于阈值」）→ Bell 徽标「3」→ 通知面板渲染完整 → 「全部标为已读」→ unread 落盘清零 + 切面板徽标消失
  - **托盘常驻 PASS**：点标题栏关闭 → 窗口隐藏（UIA 报无可见顶层窗口）→ **进程存活**（CloseRequested 被拦 = TRAY_READY = 托盘创建成功的功能性证明）；托盘图标肉眼确认与菜单唤回留用户侧
  - **系统通知**：dev 模式 WinRT 调用无失败日志（弹窗视觉验收留 NSIS 打包版）
- **验证中发现并修复的三处真实缺陷**：
  1. **登录后首批通知延迟**：隔夜死会话使轮询失败退避（×2 起），用户重新登录后还要背最多 20 分钟退避 → `notification::POLL_KICK`（tokio Notify const）+ poll_tick 休眠改 `select!` 可中断 + `finish_login` 成功点 `notify_one`（新会话同时重置退避）——登录后立即做一轮完整检查
  2. **日志盲区**：全仓无 logger 初始化，`log::warn!` 全部静默丢弃——轮询/补采失败原因完全不可见（M2 `[meeting-diag]` 教训重演）→ setup 里初始化 `env_logger`（默认 info，RUST_LOG 可覆盖，dev 进 stderr）
  3. **日期敏感测试**：`ics_skips_events_on_skipped_dates` 用宽泛 `!contains("20260921")`，DTSTAMP（生成当天）在 9-21 当天跑测试误报 → 收窄为 `!contains("DTSTART:20260921")`（断言意图=该日无课程实例）
- **顺带**：主目录 `npm install` 补装批 14 依赖（jsbarcode/react-qr-code 此前只在 worktree 装过，主目录白屏）；探针过程中实锤 queryCurrentCard 响应形态 `data={account,card,errmsg,retcode,sno}`（双层判定第二层在业务层）
- **验证**：`cargo test -p campus-hub --lib` **155 passed / 0 failed**；探针 `test result: ok`（`CAMPUS_HUB_CREDS=… cargo test -p campus-hub --test webvpn_offcampus_live -- --ignored --nocapture`，读凭据文件、不打印 token/cookie）

## 2026-09-20 · M5 通知中心：通知基建 + 三源后台轮询 + 应用内通知页 + 托盘常驻

- **模块**：`tauri-app/src-tauri/src/{infra/notification.rs(新),commands/notification.rs(新),app_tray.rs(新),lib.rs,Cargo.toml}`、`tauri-app/frontend/src/{panels/{NotificationsPanel(新),SettingsPanel}.tsx,components/{AppShell,DockNav}.tsx,stores/uiStore.ts,shared/{types.ts,tauriApi.ts}}`
- **通知基建**：`notification_state.json`（未读上限 200 + 每源已见 id 队列上限 100 + `baselined_sources` 显式基线 + 电费 24h 节流）与 `notification_settings.json`（8 字段全 serde default：默认 7 栏订阅、门户/待办 10 分钟、电费 30 分钟、阈值 10 元），损坏回退默认不删坏文件；去重/节流/校验全纯函数 + 单测
- **poll_tick 单循环**：门户 7 栏 / 待办 / 电费三源合一（启动即检查，间隔热读设置）；无会话静默跳过（绝不拉起登录）、单源失败不影响其他源、失败指数退避 ×2 封顶 2h；系统通知 Rust 侧 `NotificationExt`（标题「锡院助手」，内容只含 kind+标题——token/票据/户号不进通知）；命令 4 条 `get_notifications` / `mark_notifications_read` / `get_notification_settings` / `save_notification_settings`
- **前端**：PanelId 第 9 项「通知」（persist v3→v4 迁移、旧值原样保留）、`NotificationsPanel` 四态 + 分类徽标 + 点击跳对应面板 + 全部标为已读；顶栏 Bell 去占位（挂载/切面板刷新未读数、不额外轮询）；设置面板新增「通知」分区（三源开关 + 电费阈值 clamp + 频率展示）
- **托盘常驻**：tauri features `tray-icon/image-ico` + `app_tray.rs`（菜单「显示主窗口/退出」、左键唤起）；**关窗 = 隐藏到托盘**，托盘未建成（TRAY_READY AtomicBool）不拦关窗走默认退出
- **交叉复核修复（deepseek-flash，全部属实）**：P0「已读复活」——轮询快照在 await 前取、落盘整份回写，窗口期用户已读被覆盖 ⇒ 落盘前重读 `load_state` 只追加本轮增量；P1「首条待办被吞」——游标为空≠未基线 ⇒ `baselined_sources` 显式标志（serde default 迁移旧文件）；P1 托盘失败锁死；P2 阈值 clamp（负值按默认）+ 网关登录失败 60s 负缓存
- **验证**：`cargo test -p campus-hub --lib` **155 passed / 0 failed**（含新增 19）、`tsc --noEmit` 零错、`npm run build` 通过；CDP 真机点验——Dock 第 9 项与空态、设置分区完整、阈值 8 保存后 `notification_settings.json` 落盘再还原、**poll_tick 启动即建 7 栏真实公告基线**（state.json 游标为真实文章 id）、Bell 无未读徽标正确
- **未验证**：WinRT 系统通知弹窗（dev 模式不可靠，需 NSIS 打包版验收）、托盘图标肉眼确认（失败已有不锁死兜底）

## 2026-09-20 · M4 网络智能路由：深澜 WebVPN 全链路打通（加密破解 + 会话 + 路由接入）

- **模块**：`crates/campus-webvpn/`（新：`crypto.rs/wrap.rs/session.rs/route.rs` + live 探针）、`crates/campus-synjones/src/{client,sso,charge,recharge,ecard_face}.rs`、`tauri-app/src-tauri/src/{infra/net_zone.rs(新),commands/{synjones,electricity}.rs}`；`docs/cas-recon/REPORT.md` 样本复用
- **深澜（Srun）URL 加密破解（deepseek-flash 调研 + golden 向量实证）**：AES-128-CFB128，key=IV=`wrdvpnisthebest!`（16 字节 ASCII）、流式无 padding（密文长度=host 字节）、输出 `hex(IV)+hex(密文)`；**本校 key/IV 均为默认值**（REPORT.md 两条活样本逐字节吻合）；实现由 KEY 派生 IV 前缀、禁止硬编码 `7772…`；6 条 golden 向量全过（含预测 `10.3.100.110` → `a1a70fcf…`，后被 live 证实）
- **live 实锤（最大风险排除）**：WebVPN 登录链真实走通（CAS TGT 换 ST → 深澜 5 cookie 种下 → is_alive=true），`http://10.3.100.110/charge/feeitem` 经包装 URL 通过网关返回 **HTTP 200 + 真实电费片区数据（16101 字节）**——「网关是否代理内网 IP」此前不可验证，本轮实锤可用
- **校园网检测 `net_zone.rs`**：主判据 UDP `connect("10.3.100.110:80")` 取 `local_addr` 源 IP（不发包、微秒级）∈10/8 → Campus；辅判据 netsh 枚举接口 10/8 + SSID 佐证；60s TTL 缓存；纯函数 `classify` 单测。**本机实测坑**：代理 TUN 网卡（198.18.0.0/15）抢占默认路由致源 IP 误判 OffCampus——netsh 辅判据救回（详见 wiki `learnings/srun-webvpn-crypto.md`）
- **会话层 `session.rs`**：三步登录 + `is_alive`（GET / 不落 /login）+ `wrapped_client`；取舍=不落盘内存持有、失效凭 TGT 静默重登（多域 cookie 恢复牵动 DPAPI/session.json 破坏面大，注释写明）
- **路由接入**：`route.rs` 纯函数决策表（Campus/Unknown→Direct；OffCampus×内网×有会话→Wrapped 幂等；×无会话→NeedLogin；**公网域名永不包装**）；`SynjonesClient` 加 `base_override/vpn` 字段、**全部 base 拼接收口**（grep `10.3.100.110` 非测试代码无遗漏）；SSO 桥换票不包装、回跳 302 链逐跳先判后包（token 提取同源）；命令层 `WEBVPN_SESSION` static（TTL 30 分钟 + 失败 60s 负缓存）+ `synjones_session_routed` 接线
- **红线**：充值/电费**写路径校外失败绝不自动重试**（直接中文报错提示走官网，杜绝重复扣款面）；CAS `targetUrl`/service 保留内网原值（CAS 注册语义）；token/cookie 不进日志
- **验证**：`cargo test -p campus-webvpn` 29、`-p campus-synjones` 97、workspace 全绿；live 探针如上。**未验证（需用户配合）**：校外真机（手机热点断校园网）端到端查电费、带 `synjones-auth` 头的 berserker API 与 POST JSON 经网关透传、深澜对 10.3.100.110 的代理授权长期稳定性

## 2026-09-20 · 一卡通遗留小项：plat 设备写操作（下线/移除授权）+ 付款码条码格式取证定案

- **模块**：`crates/campus-synjones/src/plat.rs`、`tauri-app/src-tauri/src/commands/ecard.rs`、`tauri-app/frontend/src/components/ecard/EcardProfileView.tsx`、`lib.rs`
- **设备管理写操作（官方 bundle 取证背书）**：官方 `searcher.89d412b9.js` 实证下线/移除授权端点 `POST /berserker-base/equipment/{offlineEquipmentByUser,removeEquipmentByUser}`、body `{"equipmentUserBh":"<设备条目id>"}`、成功判定仅 `code===200` ⇒ `plat_offline_device` / `plat_remove_device` 两命令（复用 plat 鉴权链与 `Envelope::Berserker`）+ `clean_equipment_bh` 信任边界（只放行纯数字，单测钉死）+ 设备行「下线/移除」按钮（`window.confirm` 二次确认、busy 防重复、就地中文报错）。**按红线未真发**（会真踢用户设备），首次真发由用户在应用内触发
- **付款码条码格式定案（不改）**：官方 plat bundle 四层拆链（app → paycode chunk → PayCodeComponent → BarcodeComponent）拿到决定性证据 `JsBarcode(..., {format:"CODE128", margin:0, height:80})`——官方显式 CODE128，本仓同款 ⇒ 批 14「POS 不识别改 ITF」的猜测作废，证据不足不改（取证链见 wiki `learnings/paycode-barcode-format-evidence.md`）
- **证据不足未做**：改手机号/改密码（涉短信验证码流程）、脱机开关切换、付款顺序调整、`barcodeDel`（bundle 参数形态未完整取证）
- **验证**：`cargo test -p campus-synjones -p campus-hub` 全绿、`tsc --noEmit` 零错；live 探针 `plat_device_write_probe_live.rs` 标 `#[ignore]`（只枚举+打印请求构造，绝不真发）

## 2026-09-20 · M4.5 批 14：付款码 + 个人中心（plat 只读面）+ 官方对齐三小项——为安卓版铺路

- **模块**：`crates/campus-synjones/src/{plat.rs(新),ecard.rs}`、`tauri-app/src-tauri/src/{commands/ecard.rs,lib.rs}`、`tauri-app/frontend/src/components/ecard/{EcardPaycodeView(新),EcardProfileView(新),EcardHome,EcardBalanceView,EcardBillView,EcardStatsView}.tsx`、`tauri-app/frontend/src/{shared/types.ts,stores/uiStore.ts,panels/EcardsPanel.tsx}`
- **plat 体系鉴权突破（关键前提）**：探针实证 plat API（`/plat/*` 背后的 `/berserker-base|app` 接口）用 **`synjones-auth` 头**——与一卡通现有 token **同源直调**，无需第二套会话（官方「统一身份认证」的 plat JWT 获取链已逆向记录但不需要）。取证见 wiki `learnings/plat-api-same-token.md` 与探针 `tests/plat_sso_probe_live.rs`
- **付款码（新增，官方 `/plat/pay` 同款）**：`codebarPayinfo` 选电子账户支付方式 → `batchGetBarCodeGet` 取动态码（双层判定 retcode=="0"）；前端 jsbarcode CODE128 条码 + react-qr-code 二维码 +「查看数字」+ 有效期倒计时 + 手动刷新 + 脱机开关只读。**PII 红线**：绑定银行卡全号（bandacc）在 DTO 结构面上不存在；动态码不落日志/localStorage。待确认：条码编码格式（POS 不识别改 ITF 一行）；官方二维码 websocket 实时推流为二期
- **个人中心（新增，官方「我的/设置」只读面）**：`get_plat_profile`（资料卡）/`get_plat_equipment`（已登录/已授权设备）/`get_plat_login_logs`（日志）/`get_plat_offline_switch`；`Promise.allSettled` 逐路兜底。一期只读，设备下线/解绑/改手机等写操作待后续单独提供
- **官方对齐三小项**：账单搜索改 300ms 防抖（「搜索消费明细」）；统计页新增**支出排行榜**（月维度、官方 chart 页同款，当月数据与官方一致）；账户详情补「开户时间/当日已消费」（Option 字段命中才展示，不臆造）
- **验证**：cargo test 120 通过、tsc 零错误、npm build 通过；CDP 真机全流程点验——付款码页真实出码（余额 ¥70.46 / 倒计时 / 脱机开关已开启 / 查看数字 20 位 4 组）、个人中心资料设备日志全渲染、排行榜与官方同序、账单搜索过滤生效

## 2026-09-20 · M4.5 批 13：官方一卡通全站逐界面扫描——删除「账户转账」（官方无此功能）+ 功能对照清单

- **模块**：`crates/campus-synjones/src/ecard_ops.rs`、`tauri-app/src-tauri/src/{commands/ecard.rs,lib.rs}`、`tauri-app/frontend/src/components/ecard/EcardHome.tsx`、`tauri-app/frontend/src/panels/EcardsPanel.tsx`、`tauri-app/frontend/src/{shared/types.ts,stores/uiStore.ts}`（删除 `EcardTransferView.tsx`）
- **转账功能删除（用户确认官方找不到卡转系统，扫描实证后删除）**：官方 h5 全站遍历（首页宫格 / 大厅应用全集 / 卡包 / 卡片设置 / 支付设置 / 我的 / 设置 / 账单 / 统计 / 消费记录 / 挂失解挂 / 卡片充值 / 修改银行卡 / 校园卡解绑 / 付款码，逐按钮逐界面）——**没有任何转账操作界面**；卡设置里的「转账标识」只是圈存档位弹层（只允许自助转账 / 自助及自动转账）。结合此前 cardTransfer API 四变体全 400 + bundle 零调用，判定该功能对校内用户不存在，删除整个「账户转账」：前端 tile/子页/路由值/类型，后端 `transfer`/`EP_CARD_TRANSFER`/`parse_transfer_accounts`/`get_ecard_transfer_accounts`/`ecard_transfer` 及单测；live 探针历史取证保留。**圈存（自动转账标识/金额/限额）保留**——官方有对应设置界面且可用
- **功能对照结论（写入 wiki `decisions/ecard-transfer-removed.md`）**：官方功能我们已全覆盖（卡包/卡片信息/消费记录/卡片设置/挂失解挂/账单/统计/银行卡/卡片充值/人脸采集/电费）；未做项的取舍——付款码体系（条码/二维码/脱机开关/付款顺序）桌面端无场景不做，plat 账号体系设置（个人资料/安全/设备管理/通用/校园卡解绑）不适用；小差异留档：账单搜索框、统计支出排行榜、卡片信息开户时间/有效期
- **验证**：cargo test 120 通过（删转账单测后）、tsc 零错误；应用内回归——一卡通宫格「账户转账」消失、其余功能完整

## 2026-09-20 · 课表节假日轮：每日自动导入教务课表（教务为准）+ 公告置换辅助化 + 法定节假日跳过

- **模块**：`crates/campus-schedule/src/{holiday.rs(新),diff.rs,model.rs,lib.rs}`、`tauri-app/src-tauri/src/{commands/timetable.rs,infra/timetable.rs,lib.rs}`；前端零改动
- **背景与取证**：用户报教务系统引入调休功能后与公告解析冲突、「同一门课挤在同一格」。复用 restore_session 拉当前 kbList 取证（临时 example 用完即删）：**教务以「新增同教学班条目」表达调休**（信息隐藏同 jxb_id 一条周一原课 + 一条 `xqj=7, zcd=2周` 调休条目），教务已把补课排进周日列；kbList 的 `date` 字段是查询日时间戳、非逐条上课日期。公告置换（swap_days）与教务调休是同一事实的两份表达，并存即冲突。取证结论见 wiki `learnings/zhengfang-tiaoxiu-swap-entries`
- **每日自动导入（教务为准）**：lib.rs setup spawn 后台循环，启动即检查 + 每小时检查（覆盖长开跨天）。`auto_sync_tick`：① 法定节假日刷新（timor.tech，无需会话，闸 `last_holiday_fetch`）② 教务课表导入（闸 `last_auto_import`，手动导入也写闸→当天不重复）。护栏：无会话静默跳过（绝不后台拉起登录顶掉用户会话）、失败只 warn 下小时重试、不弹窗不阻塞启动
- **导入内核抽取 + 教务为准清理**：`import_timetable` 命令体抽为 `run_timetable_import(client, tgt, portal)`（命令与后台共用）；落库前清理——① `weekday_covered` 判定「置换日所在教学周该星期已有导入课」的 swap_days 条目删除（教务已表达调休，置换冗余）② `redundant_extra_override_ids` 删除与教务条目完全重合（同名+jxb 一致+同星期同节次+周次全覆盖）的旧逐课 extra override（同格重复渲染的根源）③ 置换日不在学期内不自动清、原课已删的 override 保守保留
- **公告辅助化**：`apply_swap_day` 采纳前同判定拦截——教务已表达则 err「教务课表已包含 X（周Y）的调休安排，以教务为准，无需采纳该置换」（前端 noticeMsg 展示，文案路径已有）
- **法定节假日跳过**：timor.tech `holiday/year/{当年+次年}`（跨年学期；需浏览器 UA 否则被 Cloudflare 拦）→ 新 `campus_schedule::holiday` 模块 `parse_timor_year`（只取 `holiday=true` 放假日，补班日忽略）+ `merge_holidays`（先剔旧自动假日再并入新集，官方修订自动退场，手动停课日不动）→ 并入 `config.skipped_dates` + 节日名写 `holiday_names`。渲染「休+节日名」列、ICS 剔除、今日页 skipped 全部复用既有消费方（`holidayNames` 字段与渲染上轮已预留），前端零改动。`fetch_holidays` 命令改调共享内核（原内联解析删除，顺修「未返回 {year} 年放假数据」字面量不插值 bug）；`save_skipped_dates` 手动编辑不再冲掉自动假日
- **新增 config 字段**：`last_auto_import` / `last_holiday_fetch`（Option\<NaiveDate\>，serde default，旧文件兼容）
- **验证**：`cargo test --workspace` 22 目标全过、0 失败（新增 holiday 4 测 + diff `weekday_covered`/`redundant_extra_requires_exact_overlap` 2 测，campus-hub 121 过）；`npm run build`（tsc+vite）通过；timor API 真实形态与 kbList 调休条目形态均实拉验证
- **未验证**：后台自动导入的真机实际触发（需重启应用观察日志 `[auto-sync]`）；用户库里是否真有旧 extra override 残留待导入后清理
## 2026-09-20 · M4.5 批 12：安全键盘「伪字符映射协议」逆转——密码输错根因修复 + 查看卡号官方三步弹窗

- **模块**：`crates/campus-synjones/src/{ecard_ops.rs,ecard.rs}`、`tauri-app/src-tauri/src/{commands/ecard.rs,lib.rs}`、`tauri-app/frontend/src/components/ecard/{SecureKeypad,EcardBankView,EcardCardOpsView}.tsx`
- **根因（推翻批 6 与旧取证两轮结论，用户坚持的密码是对的）**：`keyboard` 接口的 `numberKeyboard` 是**伪字符数组**（与想输的数字无关），`numberKeyboardImage` 才是每键一张「画着数字」的图片；服务端按 uuid 批次做数字↔伪字符映射还原。旧实现把伪字符当键面文字渲染，用户面对一堆字母符号无从输入真实密码——「输入密码就是错误」的根因。应用内端到端实锤：Number 键盘 + positions 链提交证件后六位**校验通过（retcode=0）**；旧「该校密码非证件后六位」结论作废（当时把明文直拼 pwd，协议上必然 60005）。详见 wiki `learnings/ecard-keyboard-pseudochar-protocol.md`
- **系统键盘明文输入模式删除**：明文脱离键盘批次映射，服务端还原必失败（协议上不可行）——删除 UI 入口与 `ecard_check_pwd_plain`/`ecard_unlost_plain` 命令、`plain_pwd`/`fresh_keyboard_uuid`
- **SecureKeypad 重写**：键面改为渲染官方图片九宫格（`images`，与官方 H5 一致），提交逻辑不变（padId + 位置下标）；全部调用点从 standard 91 键伪字符盘改为 Number 键盘；删 altMode/明文分支
- **查看卡号官方同款三步**：密码校验通过 → **底部弹窗**（只显前 4 位 + 通栏「查看卡号」按钮）→ 点按钮才显完整卡号（4 位一组 + 保管提示）。后端新增 `fetch_bank_number`：校验通过后回读本人卡列表 `bankacc`（官方同款数据源——`checkPwd` 只是显示闸门，全号随卡列表早已下发），`ecard_check_pwd` 填充 `bankCardNo`；学校未下发时前端如实提示不伪造
- **验证**：cargo test 121 通过、tsc 零错误；CDP 真机全流程——键盘输证件后六位 → 校验通过 → 弹窗显前 4 位 → 点按钮显 19 位完整卡号（尾 4 位与已绑定卡吻合）

## 2026-09-20 · M4.5 批 6：密码支持系统键盘明文输入（用户要求）+ 转账失败说明文案

- **模块**：`crates/campus-synjones/src/ecard_ops.rs`、`tauri-app/src-tauri/src/{commands/ecard.rs,lib.rs}`、`tauri-app/frontend/src/components/ecard/{SecureKeypad,EcardCardOpsView,EcardBankView,EcardTransferView}.tsx`
- **功能（用户点名）**：所有查询密码弹层新增「用系统键盘」切换——不再强制官方乱序键盘，用户用本机键盘直接输密码（Enter 或「提交密码」按钮）。后端新增 `fresh_keyboard_uuid`（取键盘只为 uuid，乱序键位一律丢弃、不进缓存）与 `plain_pwd`（`"1$1$"+明文+"$1$"+uuid`），命令 `ecard_check_pwd_plain` / `ecard_unlost_plain`（解挂）。明文只在本机前端内存与 IPC 流转，不落盘不进日志（错误构造点仍过 redact_secrets）。安全键盘模式保留可随时切回
- **真机验证**：`ecard_check_pwd_plain` 用错误密码探针返回业务错误 `60005 账户密码错误`——「取 uuid→明文拼串→checkPwd」全链通、格式被业务层接受，正确密码即可通过。**两次独立实证（官方上下文+本应用）都证明 60005 是密码值本身不对**：该校查询密码不是证件后六位/已被修改，请用「卡设置 → 忘记旧密码？用短信验证码找回」重设，重设后用明文输入一定能过
- **转账失败文案**：`code=400 操作失败/业务异常` 时追加说明「可能是学校未开通卡间转账（官方手机版也无此入口），请改用卡片充值或到校服务终端办理」——该校服务端四变体穷举均 400，官方移动壳无转账入口
- **限额「不显示」说明**：学校接口不回显限额（官方也只本地回写），981b5cd 起保存成功后显示「（本地记录）」；此前构建无此功能，更新后重新提交一次即见
- **验证**：`tsc --noEmit` 零错误、真机 CDP 明文链路探针通过
## 2026-09-20 · M4.5 批 5：人脸采集接入（官方 fapi 链复刻 + 真机打通）+ 设置项取舍结论

- **模块**：`crates/campus-synjones/src/{ecard_face.rs(新),client.rs,lib.rs}`、`Cargo.toml`（rsa/md-5/base64/multipart）、`tauri-app/src-tauri/src/{commands/ecard.rs,lib.rs}` + `Cargo.toml`、`tauri-app/frontend/src/{components/ecard/EcardFaceView.tsx(新),components/ecard/EcardHome.tsx,panels/EcardsPanel.tsx,shared/types.ts,stores/uiStore.ts}`
- **功能**：一卡通宫格新增「人脸采集」子页（`EcardView` 十值）——显示姓名/学工号/学校/采集状态，选择照片（JPEG/PNG ≤8MB，预览）→ 二次确认 → 上传学校人脸库（食堂/门禁刷脸）
- **协议复刻（deepseek-flash 对官方 overLightMobileH5 bundle 全量取证 + live 实测）**：`GET /fapi/img/code/public/key`（RSA 公钥**每次不同**，即取即用）→ JSEncrypt 等价 **RSA PKCS#1 v1.5** 加密密码 → `POST oauth/token`（form；`client_id=client_core`、**client_secret=123456 官方 autoLogin 写死**、`isThird=true` 自动开设账号）→ `GET oauth/detail?userId=`（实测仅 userId 必需）→ `POST meeting/largeScreen/replaceFace/{userId}`（multipart part 名 `avatar`，formData 仅 userId）。sign 头 `md5("/path-@-ts")` 照源码实现（服务端当前不校验，防收紧）。学分母：官方 `faceAcquisition`（分数检测）在本 build 是死代码，主流程只需 replaceFace
- **真机验证**：`ecard_face_detail` 返回真实数据 `{name:林博乐, number:24385214, schoolName:无锡学院, collected:false}`；子页 UI 渲染完整。**上传未自动验证**（写人脸库，留用户在应用内显式确认后操作）
- **红线**：照片 base64 只在内存流转（FileReader→IPC→multipart），不落盘、不进日志；上传前 window.confirm 二次确认
- **设置项取舍（YAGNI 记录）**：官方「我的-设置」其余选项不搬——脱机二维码开关/付款码支付顺序服务于本应用没有的付款码功能；安全设置（手机号/登录密码）属 plat 账号体系另一条鉴权链；换账号/退出/关于已由应用外壳提供
- **验证**：`cargo build` 通过、`tsc --noEmit` 零错误、真机 CDP 读链路与 UI 点验通过
## 2026-09-20 · M4.5 批 3 补验四：转账「未开通」复核（app 来源四变体全 400）、查询密码 60005 实证、限额本地持久化

- **模块**：`tauri-app/frontend/src/components/ecard/EcardCardOpsView.tsx`（唯一代码改动）+ wiki/CHANGELOG
- **用户反馈三点**：① 免密与限额没有同步数据 ② 查看卡号报 `60005 账户密码错误` ③ 官方浏览器里把 pc 字样改为 app 就能正常访问（项目有记录——HANDOFF.md「`synAccessSource=pc` 被服务端拒」结论）
- **转账复核（推翻上轮「官方页面也失败」的表述，维持「学校未开通」结论）**：上轮官方 PC 转账页的「服务大厅未授权/没有可转入的卡」实为**官方 PC 前端自己用 `synAccessSource=pc` 被服务端拒**（4030 旧结论）——不是转账功能未开通的证据。本轮在官方页面上下文用 **`app` 来源 + 官方 token** 直接调 `/ykt/tsm/cardTransfer`（官方字段逐字、0.01 元）：**四变体（正向/反向/整数 1 元/旧端点 acctypeTransfer）全部 `code=400`**（前三个「操作失败」、旧端点「业务异常」），且官方 plat 移动壳的宫格里**根本没有转账入口**——服务端业务层拒绝一切形态，维持**该校未开通卡间转账**结论，代码保留官方形态
- **查询密码 60005 实证（应用无 bug）**：在官方页面上下文用官方协议（`pwd="1$1$"+明文+"$1$"+官方 uuid`、`account=42940`、GET checkPwd）提交**身份证后六位 190453**，官方同样返回 `retcode=60005 账户密码错误`——证明：① 我们的请求格式/键盘翻译链路与官方逐字一致（同一格式官方也 60005）② **用户的查询密码不是证件后六位**（改过，或学校初始密码另有规则）。解法：应用内「卡设置 → 忘记旧密码？用短信验证码找回」重设后重试（找回走 `sendfindPwdVer`/`findPwd`，同样已实现）
- **限额「没有同步数据」→ 本地持久化**：学校侧卡信息接口不回显限额（官方前端同样只做 sessionStorage 本地回写，服务端读回恒为旧值），上一轮只做了组件内存态记住。现改为**localStorage 持久化**（键 `campushub-ecard-limits-local`，按子账户 acctype 存最近提交值），跨会话显示「当前：…（本地记录）」；提交成功文案同步说明。展示用途，清缓存即回到学校侧视图
- **验证**：`tsc --noEmit` 零错误；官方上下文协议对照两次（转账四变体 + 默认密码一次提交，均无副作用、无资金变动）

## 2026-09-20 · M4.5 批 3 补验三：官方系统动态抓包对照——四处真 bug 修复 + 转账「学校未开通」实锤 + 人脸采集与设置页考察

- **模块**：`crates/campus-synjones/src/{ecard_ops,ecard}.rs`、`tauri-app/frontend/src/components/ecard/{SecureKeypad,EcardTransferView,EcardCardOpsView}.tsx`、`tauri-app/frontend/src/shared/types.ts`
- **背景**：用户报「官方一卡通系统里都能正常操作，是对接没做好」，并截图指出：查询键盘排版不便且经常「获取键盘失败」、输密码提交 404、转账选择框选中换不了、圈存无法开启、限额无法设置。用浏览器（browser-use + 请求记录器）走完整官方链路（融合门户 → 服务大厅 → 一卡通 → 支付设置/银行卡/转账页）**动态抓取官方真实报文**逐项对照
- **修复 1（解挂 404）**：官方端点是**小写 `unlostCard`**，旧实现写成 `unLostCard`，该校路径大小写敏感 → 用户在解挂流程输完密码提交得到 `HTTP 404`。改回官方原文（`ecard_ops.rs` EP_UNLOST）
- **修复 2（圈存「无法开启」的真相——写成功、读回解析错）**：官方 `queryCard` 卡级 `autotrans_flag` 真机实测为 **2**（自助及自动转账），旧解析 `== Some(1)` 把它读成 false → 用户提交圈存**其实已写入学校**，界面却显示「关闭」。改为**非 0 即开启**并新增 `autotrans_flag_kind` 原值透出（0/1/2），前端「当前」按官方档位文案显示（禁止/只允许自助/自助及自动）。`set_autotrans` 报文同时对齐官方抓包：`autotransFlag` 字符串、金额 **JSON number 分**、档位 1 不带 `autotransLimite`
- **修复 3（转账选择框选中换不了）**：`queryCardByTransfer` 返回的 CARD/ACCOUNT 两账户 `account` 是**同一个卡号**（42940）→ 两 `<option>` 的 value 重复 → 选中后再也切不动。选项 value/key 改用唯一 `code`（CARD/ACCOUNT），提交仍按 code 找回完整账户对象
- **修复 4（键盘排版 + 稳定性）**：91 键从四组摊平改为**分区 tab**（数字/大写/小写/符号，默认数字区、每键仍携全局下标、提交语义不变），真机点验切换正常；取键盘请求补上官方恒带的 `order=1` 参数，前端失败**自动重试一次**。另实测官方 Number 键盘真相：**10 个随机字符**、无刷新按钮、输错换批——官方原生体验本就如此，我们保留 Standard 全键盘是合理替代
- **限额报文 number 化**：官方抓包 `{"account":"42940","acctype":"000","daycostlimit":50000,...}` 金额为 JSON number 分；`set_limits` 从字符串切换为 number（`yuan_to_fen_str` 随之删除，浮点陷阱口径由单测钉住）。`check_pwd` 确认本就是 GET+query 与官方一致（实测错误密码返回业务错误 `60005 账户密码错误`，非 404）；`modify_pwd` 实测业务报错 `1008 两次输入不一致`，端点亦通
- **转账「学校侧未开通」实锤**：官方 PC 前端 `/transfer` 页在本校当前环境下同样报「服务大厅未授权(1)」+「该卡没有可转入的卡」+ JS 异常——官方自己也用不了；维持「端点协议正确、该校未开通」判定，待学校开通后无需改代码
- **新功能考察（结论已写入 wiki，实现留待下一轮）**：① **人脸采集**=独立 H5（`/overLightMobileH5`，uni-app），表单形态：姓名/手机号/学工号只读 + 点击上传照片 + 确认；接口 `POST /fapi/meeting/largeScreen/faceAcquisition/{userId}`（采集，multipart 字段 `avatar`）与 `replaceFace/{userId}`（替换），前置 `checkFaceScore` 校验人脸分数，token 为 H5 应用自有体系（对接需验证鉴权链）② **「我的-设置」页选项**（`/plat/user/setup`）：个人资料、安全设置（手机号+登录密码）、支付设置（脱机二维码开关 `getUserOfflienSwitch`、付款码支付顺序、校园卡支付设置=转账标识+限额——我们已有）、设备管理、通用、关于、换账号、清缓存、退出
- **用户账户状态**：官方页验证操作已全部还原——限额 500→0（官方报文 retcode=0）、圈存 flag 2→1→2、金额 20/20 未动；本校 dev 应用仅发过错误密码协议探针（无风险）
- **验证**：`cargo test --workspace` 全绿（22 个套件 0 failed）、`tsc --noEmit` 零错误；真机 CDP 点验——转账选择框 `CARD→ACCOUNT` 可自由切换、键盘四分区 tab 渲染与切换正常、圈存档位读回 `flag:true, kind:2`

## 2026-09-20 · M4.5 批 3 补验二：写操作协议实测修正（form→JSON body）+ 限额真机打通 + 限额不回显的界面修复 + 转账结论

- **模块**：`crates/campus-synjones/src/{client,ecard_ops}.rs`、`tauri-app/frontend/src/components/ecard/EcardCardOpsView.tsx`（本轮 Rust 改动仅前两个文件）
- **背景**：校园网恢复后补验批 3 的写路径。用户以 AskUserQuestion 明确授权三项真机验证：**限额设置**、**0.01 元转账（含转回）**、**查看卡号（用户自己点，密码不经过我）**
- **根因修复（本批最关键）**：批 3 的 15 个写端点全部返回 `code=400 业务异常`，根因是**请求体编码错了**——实现用了 form-urlencoded，而官方 axios 实例默认 **JSON body**。新增 `SynjonesClient::post_json`（值全字符串）与 `post_json_vals`（值可为 number），14 处写端点从 `post_form` 切到 JSON（`synAccessSource` 合并进 JSON body 并保留同名头）。切到 JSON 后错误码随之为业务级（`retcode`/`code` 有具体语义），不再是无信息的「业务异常」
- **限额设置真机打通（首个 live 通过的写操作）**：`set500 ok=true` + `reset0 ok=true`。同时修掉第二个协议错：`acctype` 需按 `-` 拆分（官方 `acctype.split("-")[0]/[1]`），整串传会报 `code=60006 电子账户信息不存在`。这一条同时证明了 JSON body、acctype 拆分、`retcode` 双层判定三条链路正确
- **新发现：学校侧不回显限额（⇒ 界面修复）**：写入成功后重取卡信息，`dayCostLimitYuan` 等**仍是旧值**——与官方前端靠 `sessionStorage` 本地回写的行为一致，不是我们的 bug。但旧界面保存成功后 `onChanged()` 重取概览会把输入框与「当前」刷回旧值，**看着像没生效**。`EcardCardOpsView` 的限额区改为保存成功后记住本次提交值（`saved` 状态优先于学校侧值展示）并标注「（本次提交值）」，文案同时说明「学校系统不回显限额」。这是本轮唯一的前端改动
- **金额单位三套口径（本批易错点，已写入 wiki）**：一卡通读类（`ecard.rs`）一律**分**；`/charge/*`（充值/电费）一律**元**；写操作内部也不统一——限额与圈存的三个字段是**分**（官方 `100*x`），而**卡间转账的 `tranamt` 是元且不乘 100**（官方 `tranamt: amountValue.number` 原值直传）。转账若按分处理会把金额放大 100 倍
- **转账仍未通过（如实结论）**：三次真机尝试（0.01 元字符串 / 0.01 元 JSON number / 1 元 JSON number）均返回 `慧新E校接口错误（code=400）：操作失败`，**账户余额与流水零变化（无副作用）**。已逐项核对与官方 bundle **逐字一致**：端点 `/berserker-app/ykt/tsm/cardTransfer`、5 个字段名与取值来源（`dstCardAccount`/`srcCardAccount` 取账户的 `account`、`src/dst_acctype` 取 `payacc`）、`tranamt` 单位与类型、以及**官方该页面不含任何 `pwd`/`password`/`checkPwd` 字段**（正则确认）。据此判定为**服务端侧拒绝**（该校未开通此功能，或存在 bundle 不可见的前置条件），非客户端实现缺陷；代码保留官方形态并在函数注释标注该结论
- **用户账户状态**：所有验证均在授权范围内且**已还原**——限额最终为 `0/0/0`（`ecard_set_limits` 返回 `success:true`）、余额与流水无任何变动；「查看卡号」留待用户本人在应用内操作（查询密码不经过我）
- **验证**：`cargo test --lib` **91 passed / 0 failed**、`tsc --noEmit` 零错误；真机 CDP 点验限额保存成功且新文案「（本次提交值）」「已提交。学校系统不回显限额…」渲染正常（该次提交值为幂等的 0）。**未点验部分如实说明**：非零限额提交后的显示保持、以及其它密码类写操作（改密/挂失/解挂/绑卡）的端到端行为未做真机点验——前者受 CDP 输入注入在本环境不稳定所限，后者需用户本人操作密码键盘

## 2026-09-19 · M4.5 批 3 补验：写操作子页真机验证 + 两处修复（查询密码键盘选型、进页自动弹键盘）

- **模块**：`tauri-app/frontend/src/components/ecard/{SecureKeypad,EcardCardOpsView,EcardBankView}.tsx`（仅前端）
- **背景**：批 3 交付时校园网断开，写操作子页只验证到「能进 + 错误态正确」。本次校园网恢复（本机 10.2.65.46）后补验
- **真机补验结果（逐页截图）**：登录后首页显示真实余额 **￥79.96**（电子账户）+ 卡账户 ￥0.00 · 正常 · `4****40` · 本科生卡；**银行卡**页「已绑定 · 尾号 5158」+ 解绑（告警色）；**账户转账**页真实两账户与余额（卡账户 ￥0.00 / 电子账户 ￥79.96）；**卡设置**页真实限额（单日/免密/单笔均「未设置」）与圈存（开启 · 每次圈 ￥50.00 · 余额下限 ￥20.00，与探针实测的 `autotrans_amt=5000`/`limite=2000` 分吻合）；**安全键盘真实取到并渲染**（91 键）。写路径仍按红线未提交（挂失/改密/转账/绑卡一个都没发）
- **修复 1（可用性缺陷，实证驱动）**：原先查询密码用的是 `kind="number"` 键盘，实测该请求返回的是 **10 个随机字符**（某次 10 键里只有 2 个数字），**输不进「身份证后六位」这类固定数字密码**；且我校查询密码规则 `passwordRule = A/a/Num/#/leng_6` 本身含字母与符号。改为 `kind="standard"`（实测 91 键 = 数字 10 + 大写 26 + 小写 26 + 符号 32，其中**数字区是完整的 0-9 乱序**）⇒ 任意密码可输入。`SecureKeypad` 的全键盘同步改为**按字符类型分四区渲染**（数字/大写/小写/符号，每键仍携带**全局下标**——后端只按 `positions` 反查 `keys[i]`，分区纯属呈现层，不改提交语义）
- **修复 2（交互缺陷）**：进入「卡设置」页会**立即弹出**密码键盘（旧实现由 `step = inputs.findIndex(null)` 推出，初始必为 0），用户只是想看限额/圈存时也被要求输密码。改为显式「修改密码」按钮启动（新增 `started` 状态，取消/重置时归位）；文案同步改为「6 位（默认身份证后六位，可含字母与符号）」
- **验证**：`tsc --noEmit` 零错误；真机 CDP 点验两处修复（进页无键盘 → 点「修改密码」弹 91 键分区键盘、数字区完整）

## 2026-09-19 · M4.5 批 3：一卡通写操作（挂失·解挂 / 改密 / 限额 / 圈存 / 转账 / 银行卡）+ 错误文案凭据脱敏（安全修复）

- **模块**：`crates/campus-synjones/src/{ecard_ops,ecard,lib,client,charge,recharge,sso}.rs`、`tauri-app/src-tauri/src/commands/{ecard,synjones}.rs` + `src/lib.rs`、`tauri-app/frontend/src/{components/ecard/{SecureKeypad,EcardCardOpsView,EcardTransferView,EcardBankView}.tsx, components/ecard/{EcardHome}.tsx, panels/EcardsPanel.tsx, shared/types.ts, stores/uiStore.ts}`
- **后端 15 条写命令**（挂失 / 解挂 / 校验查询密码 / 改密 / 短信找回密码 / 免密与限额 / 圈存转账标识 / 卡账户↔电子账户转账 / 绑卡发码与提交 / 解绑银行卡 / 绑校园卡与解绑）；`ecard_ops.rs` 补齐协议实现，含辅助 `PasswordInput { padId, positions }`、`assemble_pwd`、纯函数 `build_pwd`、`yuan_to_fen_str`、`require_retcode_ok`
- **双层成功判定**（本批新引入的必要修正）：`/ykt/tsm/*` 与 `/accountuser/*` 的成败不只看 axios 层 `code==200`，还要看业务层 `data.retcode=="0"`，失败取 `data.errmsg` 回落顶层 `msg`——学校侧的可读原因不再被吞掉
- **密码明文只在后端**：`pwd` 协议为 `"1$1$"+明文+"$1$"+keyboardUuid` + `pwdType:"1"`；前端只提交 **`padId` + 用户点击的位置下标序列**，后端按 padId 从一次性键盘缓存取映射翻译成字符、拼串、用完即弃（与电费充值 `passwordMap` 同构的红线：明文不进前端、不进日志、不落盘、不回填输入框）
- **卡号原号不出后端**（设计取舍）：`CardDetail` 只有脱敏号 ⇒ 写命令的 `account` 改为可选，缺省时由后端 `ecard::current_account`（走 `getCampusCards` 取本人当前卡）解析——与「电费房间上下文串由后端合成」同一取舍；只有 `ecard_transfer` 需要前端回传账户原号（转账必须选账户，来自 `get_ecard_transfer_accounts`）
- **前端**：`SecureKeypad.tsx`（通用安全键盘：只记位置下标、点满 6 位自动提交、占位符显示位数、aria-label 只写「第 N 键」防读屏泄露）、`EcardCardOpsView.tsx`（挂失·解挂 / 改密三段式 / 短信找回 / 免密与限额 / 圈存）、`EcardTransferView.tsx`（转账，金额不得超转出余额）、`EcardBankView.tsx`（绑卡/解绑/查看卡号）；宫格新增三项（银行卡项按 `enabledApps` 门控、挂失分区按 `showLost` 门控）；**多卡绑定不做**（本校 `getAllApps` 无 `bind-campus-card`）；改密的「新密码一致性」本地比对依赖两把键盘的布局指纹，布局不同则交服务端判定（明文不进前端的必然代价，已在代码注释与 wiki 写明）
- **安全修复（本批意外发现）**：真机点验时 UI 错误文案里**回显了完整 URL 中的 CAS 票据**（`ticket=ST-…`）——根因是 `reqwest` 把完整 URL 拼进错误串，而本系统 URL 的 query 带票据。新增 `campus_synjones::redact_secrets`（按参数名抹值：`ticket`/`synjones-auth`/`token`/`access_token`/`password`/`pwd`/`vercode`），在**错误构造点**统一调用（`client`/`charge`/`recharge` 的 HTTP 错误与 `sso` 的两处换票错误），命令层 `err_text` 再兜一层——UI 文案、诊断日志、live 探针三条路径同时受益。实现踩了两个坑（均是既有测试当场抓到的）：① 回看键名用 `rfind(...)+1` 遇多字节分隔符（中文括号）**panic**（`byte index N is not a char boundary`），改用 `char_indices().rev()...map(|(p,c)| p + c.len_utf8())`；② 值边界用「终止符列表」会把全角右括号一起吞掉，改为按**允许字符集**界定
- **验证**：`cargo test --workspace` **363 passed / 0 failed**（含脱敏与 pwd 拼装的单测）、`tsc --noEmit` 零错误；真机点验三个新子页可正常进入、标题与门控正确、取数失败时显示错误条 + 重试（不白屏）
- **⚠️ 写路径未 live 验证（红线）+ 真机点验环境异常**：挂失/改密/限额/转账/绑卡等会真实改变卡状态或余额，按红线**未发任何真实请求**，实现依据是官方 bundle 反查的协议形态；点验当天校园网断开（`10.3.100.110` 不可达，HTTP 000），写操作子页的**数据渲染与键盘弹出未能实测**——需恢复校园网后由用户真机操作确认（尤其 `retcode` 双层判定的真实响应形态、`checkPwd` 的 GET query 是否被服务端接受）

## 2026-09-19 · M4.5 批 1+2：一卡通页（宫格首页 + 子页）——钱包与电费合并、官方读类能力补齐

- **模块**：`crates/campus-synjones/src/{ecard,ecard_stats,ecard_ops,recharge,lib}.rs`、`tauri-app/src-tauri/src/commands/{ecard,synjones,electricity,mod}.rs` + `src/lib.rs`、`tauri-app/frontend/src/{panels/EcardsPanel.tsx, components/ecard/*, components/{AppShell,DockNav,RechargeFlow}.tsx, panels/TodayPanel.tsx, shared/types.ts, stores/uiStore.ts}`
- **用户裁决（AskUserQuestion）**：① 功能范围**全选**——读类补齐 + 一卡通充值 + 安全类写操作 + 银行卡与多卡管理；② 布局**宫格首页 + 子页**；③ 导航**合并为一个入口**（Dock 9 → 8）
- **缘由（用户）**：「钱包页面还未完善，请打开融合门户的一卡通系统查看，把一卡通系统功能搬下来单独做一个页面，将钱包和电费都融合至一卡通系统页面」。抓官方 `campus-card-pc` 前端 bundle 反查后确认：**官方一卡通系统本身就含电费**（服务大厅清单里 `电费桃1-李8`/`李9-李11`/`梅1-梅3` 即我们的 448/449/450 三片区），而**一卡通充值就是 `/charge` 体系的 401 片区**——三者本就是同一套缴费引擎，合并成一个页面是还原官方形态而非新造
- **删除**：`panels/WalletPanel.tsx`、`panels/PowerPanel.tsx`（后者内容整体迁入 `components/ecard/EcardPowerView.tsx`，交互与四态零回归）
- **live 实测推翻两条旧结论（重要）**：新建只读探针 `crates/campus-synjones/tests/ecard_features_probe_live.rs` 实测，`/berserker-search/statistics/turnover/count` **吃** `timeFrom/timeTo`（无参 533160 分 / 2026 全年 241540 / 2020-01 为 0，三组互不相同），`statistics/turnover/sum/user` 的正确参数是 `dateStr + dateType(month|year) + statisticsDateStr(day|month) + type(1 收入/2 支出)` 且**有数据**（回 `{"2026-09-03": 1800.0, …}` 日期→金额 map，值为分）⇒ 此前 wiki 与代码注释里写的「统计接口无解、UI 暂不可用」**是参数形态猜错导致的误判**，本次改为真实统计能力（支出/收入/结余 + 按日按月双折线 + 分类聚合 + 分类字典）
- **后端新增（全部只读）**：`ecard.rs` 扩 `CardDetail`/`AccInfo` 脱敏卡视图（`getCampusCards` 全字段，金额分→元）与流水 `TurnoverFilter`（`type/typeId/info/orderId/sortFields`，**未传的参数不进 query**）、记录补 `orderId/typeId/turnoverType/labelName/labelRemark/cardBalanceYuan`；`ecard_stats.rs`（统计三件套 + 分类字典，信封 `Search`，series map 在后端转**升序数组且零值保留**）；`ecard_ops.rs`（安全键盘 `GET /berserker-secure/keyboard` + 进程级缓存：TTL 300s、至多 8 把、随机 `padId`、`take_pad` 取走即删）；命令层 `commands/ecard.rs` 7 条新命令 + `get_ecard_transactions` 原地扩参
- **前端结构**：容器 `panels/EcardsPanel.tsx`（宫格首页 ⇄ 子页，返回栏）+ `components/ecard/{EcardHome,EcardBalanceView,EcardBillView,EcardStatsView,EcardPowerView,EcardRechargeView,MiniLine}.tsx`；`PanelId` 9→8（`wallet`+`power` → `ecard`），`uiStore` persist **v3** 把旧值显式迁移到 `ecard`（否则用户升级后被兜底踢回今日页）、新增持久化 `ecardView`；今日页「查电费」「卡片充值」两个快捷动作改指 `ecard` 并**直达子页**（合并成一个入口后不必多点一次）
- **脱敏与安全**：卡号默认只透出脱敏形态（够长前 5+`****`+后 2；**本校 5 位短卡号**改首位+`****`+末 2——旧实现一律给 `****`，界面等于什么都没显示）；持卡人姓名/手机号/证件/户号一律不解析不进 DTO；`frontInfo` 的 `getFrontConfig` 串里**含学校侧下发的 `privateKey`**，实现只取白名单键、整串不落盘不透传（该观察已写入 wiki learnings）
- **充值片区 401**：`singleFeeitem?feeitemid=401` 实测 `layout="1,10,50,100"`、金额上下限均为 null；`getThirdData(401, level=0)` 回 **code=500** ⇒ 401 **没有级联上下文**，建单**不带 `third_party`**——为此 `recharge::create_order` 的 `path` 改 `Option`，命令层加**显式开关** `no_context`（不做「缺 path 即静默无上下文」，避免电费片区忘传房间时从报错退化成难排查的静默失败）
- **真机点验（CDP + WebView2 调试端口，逐页截图）**：宫格首页显示真实余额 **￥79.96**（电子账户）+ 卡账户 ￥0.00 · 正常 · `4****40` · 本科生卡；流水子页取到真实 **1046 条**记录并可按方向/分类/关键词筛选（分类 chips 为服务端真实分类名）；统计子页 2026-09 支出 ￥195.55 / 收入 ￥201.00 / 结余 ￥5.45 + 双折线（含极值标注）+ 分类占比；电费子页三个片区、常用房间、缴费记录（含「已取消 · 本机记录」）、今年每月缴费全部正常；充值子页按 401 显示 1/10/50/100 元快捷金额与风险说明
- **验证**：`cargo test --workspace` **353 passed / 0 failed**、`tsc --noEmit` 零错误、真机点验如上（写路径按红线**只到提交前一步，未提交任何支付/挂失/改密**）
- **未做（批 3）**：安全类写操作子页（挂失·解挂、修改/找回查询密码、免密与支付限额、转账标识、卡账户↔电子账户转账、银行卡绑定/解绑）与多卡绑定（该校 `getAllApps` 无 `bind-campus-card`，按官方开关不显示入口）。后端的安全键盘缓存与写操作协议形态已就位（bundle 反查 + 401 实测），写路径按红线**未经 live 验证**，需接好后由用户真机操作确认

## 2026-09-19 · M4 布局微调：「常用房间」并入「查询房间余额」卡（用户要求融合）

- **模块**：`frontend/src/panels/PowerPanel.tsx`（仅前端）
- **缘由（用户）**：「常用房间卡片和查询房间余额卡片融合」。常用房间本质就是查询入口——点「查余额」走的是**同一条级联查询**，原先却在右列独立成卡，与左栏的查询动作隔开。
- **改法**：整块移入「查询房间余额」卡的**卡内区段**（`mt-4 border-t border-line pt-3` 分隔，标题右侧带「共 N 个」计数），空态 / 绑定标记 / 登录提示 / 错误提示原样保留；右列由三张卡减为两张（电费变化 + 缴费记录），左右两列高度更接近。
- **验证**：`tsc --noEmit` 零错、`npm run build` 成功；真机点验（CDP 截图）：左列查询卡内含「常用房间 · 共 1 个」区段（绑定项带「我的宿舍」标记与查余额/解绑/删除），右列只剩电费变化与缴费记录，且上一轮修复的「已取消 · 本机记录」留痕仍正常显示。

## 2026-09-19 · M4 修复：电费页「最近查过」与「已取消订单」两处用户反馈

- **模块**：`frontend/src/panels/PowerPanel.tsx`、`frontend/src/components/ElectricityPaymentsCard.tsx`（仅前端，无后端改动）
- **问题 1「最近查过只显示一个 `1`，我查的是 125」**：根因是这块的**数据源与显示都错了**——数据源是「已保存房间」（要用户先保存才出现），渲染的又是 `SavedRoom.label`（用户自起的房间名，本例就是「1」），所以既只有一个 chips、内容也不是房间号。改为：每次查到**末级房间**即记入本机 `localStorage`（片区 + 完整路径，同房间去重后置顶、上限 8 条），chips 显示**路径末级的房间号**（`value`），点击直接重查——不要求用户先保存为常用房间，这才对得上「最近查过」的字面语义。
- **问题 2「缴费记录中被取消的没有显示取消的标记」**：真机实测（`get_electricity_orders` 分别按不带 status / `0` / `1` / `2` 查询）确认**学校侧的取消是物理删除**——取消后该订单在四种查询里**都不返回**，学校侧没有「已取消」状态可供渲染，且取消时它已从列表消失、我们也没留存摘要。改为：取消成功时**先抓下该单摘要再删**（订单号 / 金额 / 提交时间 / 摘要），写入本机 `localStorage`（上限 10 条），在缴费记录卡显示为「已取消」灰条（金额划线），元信息标注「**本机记录**」——明确区分「学校侧状态」与「本机留痕」，不伪装成学校侧数据。
- **边界**：两条记录都是本机 `localStorage`（属便利性数据，不进 `%APPDATA%` 的业务文件，也不参与跨端合并）；存储被禁用或内容损坏时静默回空，不影响查询与缴费记录主流程。
- **验证**：`tsc --noEmit` 零错、`npm run build` 成功；真机点验：查到 125 房间后「最近查过」显示 `125`（不再是「1」）、点击可一键重查；取消订单后该单以「已取消 · ¥1.00 · 本机记录」留痕。

## 2026-09-19 · M4 批 3+4：电费页重设计（双列布局·校区自动·楼栋卡片·自采趋势与缴费记录）+ 真机点验

- **模块**：`frontend/src/panels/PowerPanel.tsx`（重写 873 行）、`components/ElectricityTrendCard.tsx`（新建）、`components/ElectricityPaymentsCard.tsx`（新建）、`components/RechargeFlow.tsx`（仅文案）
- **用户裁决落地**：① 每片区校区唯一 ⇒ **唯一选项自动选定**（判据只有「选项数 == 1」、不写死层名；三道防死循环闸——路径严格增长 / 末段已是该选项则不再自动选 / `MAX_CASCADE_STEPS=10` 兜底），自动层收成弱提示「校区 · 无锡国际校区（自动）」而非占一个交互位；② 楼栋改**卡片网格**（单击即进，带「上次查过」角标），房间号用自动聚焦输入框 + 回车提交 + 「最近查过」chips；③ 充值密码提示补「**默认为身份证后六位**」；④ 容器从 `max-w-3xl` 居中单列改 `max-w-5xl` **双列**（左列片区/查询/结果/充值，右列电费变化/缴费记录/常用房间），消除宽屏两侧空白
- **取舍（主智能体裁决）**：**不做屏幕数字小键盘**——桌面端物理键盘输入快于鼠标点数字，「快捷输入」由自动聚焦 + 回车 + 最近房间 chips 兑现
- **零新依赖**：项目无图表库，趋势折线与月度柱均**手写 SVG / CSS 块**；`balance === null` 的点只占位、断开线段（**不插值、不连缺口、不画 0**），有效点 < 2 个时给空态文案而不画假线
- **遗留订单清理（实测闭环）**：「缴费记录」卡置顶未支付订单并给「取消订单」（复用 `recharge_cancel`）——批 C 首轮验证留下的那笔 **1 元未支付订单（09-19 16:40、450 片区）已在应用内取消成功**（提示「订单已取消」、待支付区消失），此前给用户的「需自行去官方缴费页取消或等过期」负担**解除**，「进充值前检查遗留订单」防线正式生效
- **真机点验（CDP + WebView2 调试端口）**：片区「桃园1号-李园8号」→ **校区自动选定** → 楼栋卡片（14 个，「李园捌号A12」带「上次查过」）→ 房间 125 → 查询成功：**主数字 ￥837.47**，而服务端原文为「当前余额837.47元,当前剩余电量1550.87度」⇒ 结构化余额 `balanceYuan` 提取正确，**没把 1550.87 度（kWh）当作钱**（这对「严格相邻提取、刻意排除电量词」的取舍是决定性的实证）；「绑定为我的宿舍」+「立即采集」→ `%APPDATA%/campushub/electricity_history.json` 落盘 `{id:"448::…@2026-09-19", device, roomKey, balance:837.47, raw, source:"manual"}`（**无 token、无 PII**），趋势卡如实显示「最近 30 天只有 1 个有效余额记录，再过几天就能画出变化」；月度缴费（09 月 1.00）与账单 2 条均取数正常
- **顺带印证（非本任务）**：账单与一卡通流水显示 09-19 17:08 有一笔 1 元缴费成功入账 ⇒ M3.1 的**支付成功路径已由用户真机试充验证通过**
- **未验证 / 已知边界**：趋势折线的多日形态需再有 2 天真实数据才能目视确认（当前按设计走空态）；单价未展示（`price` 未暴露到命令层，且实测 448 的 `price` 为 0 不可靠）；启动补采的实跑日志待观察
- **验证**：`cargo test --workspace` **329 passed / 0 failed / 13 ignored**、`tsc --noEmit` 零错、`npm run build` 成功，真机点验如上

## 2026-09-19 · M4 批 1+2：缴费历史只读取数层 + 自采日余额快照 / 绑定宿舍 / 启动补采

- **模块**：`crates/campus-synjones/`（`turnover.rs` 新建、`charge.rs` 加余额严格提取、`ecard.rs` 加 `cardBalance`、`tests/m4_history_probe_live.rs` live 探针）、`tauri-app/src-tauri/src/infra/electricity_history.rs`（新建）、`commands/electricity_history.rs`（新建，6 命令）、`commands/electricity.rs`（`SavedRoom.bound` + `set_bound`）、`lib.rs`（注册 + `.setup()` 启动补采）、`frontend/src/shared/types.ts`（类型）
- **实测修订（重要）**：① **单位红线**——`/charge/*` 侧 `TRANAMT`/`tranamt`/`accountTotal`/`pieAccountList[].tranamt` 是**元**（官方 PC 页对 `TRANAMT` 做 `/100` 是错的，不要照抄），一卡通侧才是**分**；② **推翻旧结论**：`/charge/order/personal_data?status=0` 并非「任何形态恒 500」——真实门槛是 **App 口径头组**（`synAccessSource=app` 必须同时进 query 与同名头），补齐即回 `code=200` + `orderList`，**「进充值前检查遗留订单」这条防线恢复**（含当时遗留的 1 元待支付单）；③ 学校侧**没有**电费日余额序列（`balance_amount` 恒 null、`mouthAccount` 忽略参数、`threeExpen_account` 恒空）⇒ 日序列只能客户端自采
- **批 1（crate 取数层，全部只读 GET）**：`turnover.rs`——账单 `app_account`（`count` 是全量条数）/ 月度 `pie_account`（参数生效，空月 = 0）/ 累计 `app_totalAccount`（可能 null）/ 订单 `order/personal_data`（含待支付）/ 片区配置 `showFeeitem`；`charge::balance_from_text` 按「关键词 + 分隔符 + 数字」的**相邻形态**提取余额（提不到返回 None，**刻意排除「剩余电量」**——448 原文同一句里既有金额也有 kWh）；`Transaction.card_balance_yuan`（分→元的事件级余额快照）
- **批 2（自采与统计）**：`electricity_history.json`（`id = roomKey@date` 确定性主键、去重键 = 房间 + 日期、同日覆盖、上限 2000 丢最旧、读失败回空且不删坏文件）；`merge_history(a,b)` 纯函数（按 id 去重取 `collectedAt` 新者 → 时间升序 → 裁剪），**可交换 + 幂等**（安卓端内网同步的基础）；绑定宿舍（`SavedRoom.bound`，`serde(default)` 兼容旧文件，最多一个，绑新自动解绑旧的，**保存房间不会静默丢绑定**）；新命令 6 条 `get_electricity_bills` / `get_electricity_monthly` / `get_electricity_orders` / `get_electricity_history` / `bind_electricity_room` / `run_electricity_snapshot`；启动补采（`.setup()` + `spawn`，今日未采 + 有内存会话才采一次，不弹窗/不阻塞/失败只记日志，未登录静默跳过）
- **红线遵守**：学校侧**只读**（禁 `/blade-pay/pay`、`deleteOrder`、`addRefundOrder`、`sceneBind/add`、`updateReceivable`；取消遗留单复用已有 `recharge_cancel`）；token/账号/密码/cookie/户号**不落盘、不进日志、不进样本**；token 单活 + `MutexGuard` 串行化沿用同一实例；单测全离线，**未跑 `#[ignore]` live 测试**
- **修复（交接断口 + 真缺陷）**：`ecard.rs` 被中断截断的 `#[test]` + 函数签名两行、`lib.rs` 缺 `pub mod turnover`、`turnover.rs::parse_bills` 的 `records` 类型推断（E0282）；另修两个真缺陷——`infra/electricity_history.rs::normalize` 的**同 id 去重不能靠「排序后看相邻」**（会被别的房间的同期记录插在中间而漏合并，见 `learnings/history-dedupe-not-by-adjacent-sort`）、`SavedRoom.id` **毫秒撞号**（同毫秒连续新增两个房间会得到同一 id，而 id 是绑定/删除的定位键）
- **批 2 补口（同轮收尾，后端收口）**：`ElectricityView` 加 **`balance_yuan`**（serde ⇒ 前端 `balanceYuan`）——`final_query` 构造末级视图时用 `balance_from_text` 提取一次，前端结果卡主数字与趋势/统计**读它、不解析 `fields` 自由文本**（提取实现只有 crate 一处，复制到前端会随校方文案漂移）；`None` = 未提取到（显示「无数据」），**绝不用 0 代替**。桌面端自采快照 `build_entry` 改为**直接透传**该字段（不再自行提取）＋新增 `charge::raw_text_of` 供取原文；`types.ts` 加 `balanceYuan?: number | null`（不动组件）
- **验证**：`cargo test --workspace` **323 passed / 0 failed / 13 ignored**（本轮新增单测 35：crate 侧 19 = `turnover.rs` 13 + `charge.rs` 5 + `ecard.rs` 1，tauri 侧 16 = `infra/electricity_history.rs` 10 + `commands/electricity_history.rs` 4 + `commands/electricity.rs` 2）；`tsc -p tsconfig.json` 前端类型检查零错（`SavedRoom.bound` 声明为可选，不破坏既有组件）；**真机点验与启动补采实测属批 4**，本轮未跑

## 2026-09-19 · 仓库卫生续：PLAN.md 移出 git 跟踪 + 个人脚本防误提交

- **模块**：`.gitignore`、git 索引（PLAN.md `git rm --cached`，本地文件保留）
- **要点**：`PLAN.md`（项目计划，含内网地址与侦察结论，与 docs/ 同性质的本地工作文档）移出跟踪；根目录个人油猴脚本 `一键填分4.user.js` 加入 ignore 防误提交（此前仅未跟踪）；`m4_history_probe_live.rs` 属于其他会话（sess-36e5d8e0）开发物，不动
- **验证**：`git rm --cached` 后本地文件在、`check-ignore` 两条规则命中、主目录 ff 后已 restore PLAN.md 工作区文件

## 2026-09-19 · 仓库卫生：docs/ 移出 git 跟踪（本地工作文档不进远端）

- **模块**：`.gitignore`、git 索引（docs/ 下 30 个文件 `git rm -r --cached`，本地文件全保留）
- **要点**：docs/（HANDOFF 交接、cas-recon 侦察脚本与报告、superpowers 计划、verify 验证截图、upstream 清单，约 1.7MB）均为本地工作产物，按用户裁决不再推送远端；`.gitignore` 新增 `docs/` 整目录规则，删除被其覆盖的旧条目 `docs/cas-recon/captcha.png`；Tauri 应用图标、`.codewiki/` 等其余跟踪文件不动；`账号与密码.txt` 经查本就未跟踪
- **注意**：git 历史中的 docs/ 旧版本不受影响（远端历史仍可见）；如需彻底抹除需重写历史（未执行，涉 force push）

## 2026-09-19 · M3.1：充值改客户端直调官方接口（移除内嵌官方页面）

- **背景（用户裁决）**：「充值还是不要使用官方界面，我们直接使用对应的验证以及接口，同时声明风险」；配套裁决：内嵌页移除并保留「去官网充值」兜底、**真实试充由用户自己完成**（不代做不可逆的资金操作）
- **模块**：`crates/campus-synjones/src/recharge.rs`（新建）、`commands/electricity.rs`（+6 命令）、`frontend/src/components/RechargeFlow.tsx`（新建）、`PowerPanel.tsx`、`shared/types.ts`；**移除** `RECHARGE_4030_HOOK` 与 `open_recharge_page`（-462 行，全仓 grep 零命中）
- **要点**：对比官方两条链路后选 **App 版口径**（`/charge-app`：纯 JSON、**无签名**、下单回 `orderid`、可轮询 `order.status`、可 `deleteOrder`）——PC 版需 SHA256 签名且 `target="_self"` **整页跳走**、客户端拿不到扣款结果；`payList` 来源是 `GET /charge/pay/getpayinfo`（**不是 paystep**）；免密判据 `payList[i].nopassword === 1`；`third_party` 由**后端**按房间路径重放末级查询合成（`map.data` 含户号等 PII，不下发前端）
- **安全红线**：安全键盘下发的 `passwordMap[uuid]` 是 10 字符**显示**序列，提交的是**用户点击的键位下标序列**而非真实数字 ⇒ **客户端不接触真实卡密码**；但客户端持解码表，故**只转发、绝不还原/落盘/打日志/回填输入框**
- **实测**：450 片区唯一账户渠道 `ACCOUNTTSM`（电子账户）且 `nopassword=false`（需密码）；live 跑到「键盘就绪」并当场取消订单；两个学校侧的坑已固化——`passwordMap[uuid]` 是 10 字符**字符串**（非数组）、`deleteOrder` **只有 JSON body 才回 200**
- **风险披露与遗留（如实记录）**：界面常驻风险声明（真实扣款不可撤销/不接触密码/失败可取消/接口变更改用官网）；首轮 live 验证用 form 方式调 `deleteOrder` 失败（该端点只认 JSON body），留下 **1 笔 1 元未支付订单**且订单号未落盘，学校侧**没有可用的待支付订单列表接口**（`personal_data?status=0` 恒 500）⇒ 无法程序化取消，**需用户在官方缴费页手动取消或等过期**（未支付=无扣款）。自批 C 起所有运行均正确清理；计划里「进充值前检查遗留订单」这条防线被证实**无法实现**
- **未验证**：`submit_pay` 的**成功路径**只能由用户真机试充验证（开发与点验阶段一律不提交支付）

## 2026-09-19 · 节假日轮：置换日模型 + 法定节假日一键拉取 + 公告重复通知去重

- **模块**：`campus-schedule/model.rs`（`swap_days`/`holiday_names`）、`notice.rs`（`detect_date_swap` 替代逐课置换分支）、`commands/timetable.rs`（`apply_swap_day`/`save_swap_days`/`fetch_holidays` 新命令、`revoke_by_notice` 扩展、ICS 置换实例、今日页 `swap_weekday`、扫描去重）、`TimetablePanel.tsx`（网格「休/班」角标+节日名横幅+置换课列、置换候选卡、设置弹层置换日编辑+节假日按钮）、`TodayPanel.tsx`（调休横幅）、契约 §22
- **双轨数据**：timor.tech API 一键拉当年法定放假日（并入跳过日期+节日名；补班日忽略——API 无补课映射，由公告置换解析承担）；公告「9月20日（周日）补9月28日（周一）课」自动产出置换候选，一次采纳=该日整列按被补日课表上课（替代此前逐门课一条补课 override）
- **显示**（用户拍板：横幅遮罩+隐藏课程）：假期列「休·国庆节」红标+节日名横幅+课程隐藏；置换列「班·补周一」蓝标+显示被补日课程；今日页调休横幅；ICS 含置换实例
- **重复通知去重**：两栏目同文（title+日期）只解析一条
- **验证**：`cargo test --workspace` 18 组全绿（新增 swap 撤销/今日置换/置换探测测试）、tsc 零错、vite build 成功
## 2026-09-19 · 自动解析轮：公告一键自动发现+解析（auto_parse_notices）+ 旧通知按学期过滤

- **模块**：`commands/timetable.rs`（新命令 `auto_parse_notices`，NoticeAutoParse 聚合结构）、`lib.rs`（注册）、`TimetablePanel.tsx`（一键按钮+状态徽标列表）、`shared/types.ts`（NoticeAutoParse）、契约 §21
- **改动**：原「检查→列表→逐条解析」两步流程收拢为一键：扫描命中 → **旧通知过滤**（发布日期早于本学期开学日丢弃，无开学日回落当年元旦）→ 逐条自动拉正文解析（置换型/单课型统一走 parse_notice_with_semester）→ 全部候选合并进确认流逐条采纳（不自动 apply 红线不变）；列表项改「N 条候选/未能解析/无调整」状态徽标
- **验证**：tsc 零错、vite build 成功、cargo test --workspace 18 组全绿
## 2026-09-19 · 同步覆盖修复轮：diff 匹配键加时段（修课程块压缩）+ 公告「全校日期置换」解析（修解析错位）

- **模块**：`campus-schedule/src/diff.rs`（match_key 五元组 + 消费式一对一配对）、`campus-schedule/src/notice.rs`（新增 `parse_notice_with_semester` 置换识别分支）、`commands/timetable.rs`（两处解析调用点接学期锚点）、契约 §20
- **同步压缩根因**：正方 kbList 同一教学班（同 jxb_id）按多时段拆多条（马原 3 条同 id、信安 2 条同 id），旧匹配键 `(name, class_id)` 二次同步时一对多命中 → 本地多条被覆盖成同一条时段 → 同格重叠分列渲染，块被压成 1/3 宽。修复：匹配键扩为 `(name, class_id, day, start_section, end_section)`，diff 改消费式配对，id 保持稳定；受污染用户数据已按教务真实值重排（id 不变），下次同步应零变化。回归测试 ×2
- **公告解析根因**（取证：应用确实抓到了正文）：「9月20日补课通知」正文第二行为「9月20日（星期日）补9月28日（星期一）课程」——全校日期置换型，旧解析器把书名号《关于…放假安排的通知》误提为课程名、摘录锚到「全体师生：」。修复：置换格式识别（相邻日期对 + 连接词，括号星期提示/学期锚点推算星期与周次），对被补日每门课产出 Extra 候选（节次沿用原课），缺锚点降级 Low；未命中回落原解析。回归测试 ×4
- **验证**：`cargo test --workspace` 全绿（diff 14 + notice 16 + 全仓 226+ 无失败）
## 2026-09-19 · 重设计轮（批 A+B）：作息小节化、公告自动发现调课通知、课表页弹窗化与引导性重设计

- **模块**：`campus-portal`（`section_time_slots` 内置默认、公告关键词发现 `collect_schedule_notices`、`html_text` 剥标签）、`commands/timetable.rs`（`effective_slots_at` 小节统一、删大节分叉、旧 5 行大节 slots 一次性迁移丢弃、新命令 `list_schedule_notices`/`parse_notice_from_url`）、`TimetablePanel.tsx`（CourseForm 模态化+上下文徽标+自动聚焦、删「全部课程」列表与粘贴解析卡片、公告发现区、作息弹层 11 小节、设置弹层快删改周次文本框）、`PanelHeader` 不变、契约 §18/§19
- **批 A（后端口径）**：config.slots/slot_rules 语义改为 11 小节表（与网格渲染、ICS、今日页全链路统一，消除「自定义走大节/默认走小节」分叉）；`block_time_slots` 大节表保留仅供门户聚合兜底；公告发现 = 栏目「通知公告(9)」+「教务处」各拉 50 条标题做关键词检测（强：调课/停课/补课；弱：教学调整/课程调整/上课时间/课程变更），命中进候选列表，URL 解析剥 HTML 正文复用 `parse_notice_text`，**只进候选确认流不自动改课表**
- **批 B（前端重设计）**：用户 5 点反馈落地——① 弹层/表单排版重排（快删 19 chips 改周次文本框）；② 删「全部课程」列表（编辑/删除保留在详情浮层，浮层补删除按钮）；③ 粘贴解析卡片 → 公告发现区（检查→列表→解析→候选确认）；④ 作息弹层按小节编辑；⑤ 点空白格改**居中模态弹窗**（上下文徽标「周五 · 9-10 小节 · 第 2 周」引导 + 自动聚焦课程名），不再页面下方出现；空课表引导文案指向顶栏按钮
- **验证**：`cargo test --workspace` 全绿（226+）、`tsc --noEmit` 零错、`npm run build` 成功；真机回归见后续条目
## 2026-09-19 · 修复：内嵌充值页「服务大厅未授权」弹窗（4030 参数改写 hook）

- **模块**：`commands/electricity.rs`（新增 `RECHARGE_4030_HOOK` + 离线单测，仅此一文件）
- **根因**：官方 `charge-pc` 页面部分请求**硬编码 `synAccessSource=pc`**，撞上学校服务端来源授权策略（`pc` 被拒、`app` 放行，即 4030）→ 页面弹「服务大厅未授权」；我们注入的 `agentType=app` 管不到写死该值的请求
- **要点**：注入脚本**第一段**安装 fetch/XHR hook，把 `synAccessSource=pc` 改写为 `app`，覆盖 URL query / 请求头 / 请求体三种携带位置（含 `Request` 实例与 `Headers` 三种形态；`new Request` 重建失败退回原对象）；纪律 = **只改不增**（官方没带的请求不加参数）、只改该键（authorization 等不动）、幂等、try/catch 兜底不破坏 token 注入；属**临时措施**，学校修复 PC 授权后可整段移除

## 2026-09-19 · M3 批 3：电费三级查询 + 常用房间 + 应用内嵌充值页

- **模块**：`campus-synjones/src/charge.rs`（新建）、`commands/electricity.rs`（新建，7 命令）、`PowerPanel.tsx`（重写）、契约 types +85 行
- **要点**：片区口径 = `status==1 && impl_interface` 非空（**恰好 3 条**；`status==1` 实有 6 条，另 3 条是补卡/充值/扫码类；同名停用项靠 `status` 排除）；末级是**手输房间号**（`flag[4]=='3'` 的「先选择再输入」）而非下拉，房间号格式严格（`101` 命中，`1-101`/`101室` 会得到「缴费系统返回数据错误 child==NULL！」）；结果 `map.showData` 键名**恒为「信息」**、值是三片区**格式各异**的自由文本（`map.money`/`iectranamt` 实测不存在）→ 通用字典渲染 + 逗号折行 + 负数标红，**不做文本解构**（否则随文案漂移静默失效）；常用房间**本地落盘** `electricity_rooms.json`（同片区同路径 upsert、上限 20），不写平台侧 `sceneBind/add`
- **真机点验（CDP）**：三片区 → 校区(1) → 楼栋(1/3/2号楼) → 手输 `101` → 「房间号：101 / 剩余金额：-545.70 / 单价：0.5400」（负数行标红）；保存房间确认落盘且列表可查余额/删除；内嵌充值窗口为**登录态**（渲染官方缴费表单，非登录页），且该窗口调 IPC 被 ACL 拒绝（安全边界验证通过）

## 2026-09-19 · M3 批 2：一卡通取数与钱包页接线（慧新E校实时 + 门户降级）

- **模块**：`campus-synjones/src/ecard.rs`（新建 356 行）、`commands/synjones.rs`（新建 270 行：`get_ecard`/`get_ecard_transactions`/`get_wallet_cards`）、`WalletPanel.tsx`（重写）、`TodayPanel.tsx`（仅钱包卡）、契约 types +61 行
- **要点**：**余额口径**以电子账户 `elec_accamt`（分）为准、卡账户 `(db_balance+unsettle_amount)` 次级显示；流水不带 `type` 即全量（实测 1042 = 支出 1006 + 收入 36）；`cardname` 实测为空串 → 回落 `card_name`→`cardtype`；**单 token 缓存**（进程级 `static SYNJONES` + `MutexGuard` 串行化，杜绝并发 SSO——token 实测为「单活」）；首页钱包卡**实时优先、失败静默回落门户快照并标注来源**，不阻塞首屏
- **已知降级**：今日/本月消费无解——`berserker-search/statistics/turnover/sum/user` 实测需五参、17 种组合全部返回空 `data`，`count` 给的是**全时段**总额且无视日期参数，故 UI 显示「暂不可用」（如后续需要，只能拉全量流水本地按日期求和，收益不匹配暂不做）

## 2026-09-19 · M2 遗留补做：顶栏命令面板（Cmd/Ctrl+K）

- **模块**：`components/CommandPalette.tsx`（新建 332 行）、`AppShell.tsx`（触发条 + 挂载）、`uiStore.ts`（开关状态，不入持久化）
- **要点**：三类数据源——DockNav `DOCK_ITEMS` 的 9 个页面项、现有 store 动作（主题/登录/头像同步/退出/切换已存账号）、设置跳转；模糊匹配 + `↑/↓/Enter/Esc` 全程键盘可达 + `role=dialog/combobox/listbox` 语义 + 焦点回归；清掉 M2 遗留的 `aria-disabled` 搜索胶囊与三处「M2 接入」注释；零新依赖

## 2026-09-19 · M3 批 1：慧新E校协议 crate（lyCas SSO 桥 + 三套信封）

- **模块**：`crates/campus-synjones`（新建：`lib.rs`/`sso.rs`/`client.rs` + live 测试）、根 `Cargo.toml` members
- **要点**：CAS TGT 走 lyCas 桥换 token（**2 跳**，token 在落点 URL query `?synjones-auth=`、无 Set-Cookie 参与）；**`targetUrl` 是硬性前提**——`/campus-card-pc/`、`/charge-pc/pays/450` 带 token，而 `/plat/shouyeUser` 与裸调用**不带**（产品默认值据此定为前者）；`synAccessSource=app` **双份携带**（GET 走 query + 同名头、POST form 走 body + 同名头），4030 首次带 token 实证（`app`→200、`pc`→HTTP 401 `code=4030`）；berserker/charge/search 三套信封分离解析，`Envelope` 显式传参防混用；**token 单活**、CAS TGT 隔夜过期（换票 500，正文含票据不得回显）。live 全链断言通过（换到 token 后 `queryCurrentCard` 200）

## 2026-09-19 · 修复：门户会话失效被误报为「解析失败」（各界面资讯拉取失败）

- **模块**：`campus-auth/src/{error.rs,cas.rs}`、`campus-portal/src/{client.rs,Cargo.toml}`、`campus-portal/tests/portal_diag_live.rs`（新建 live 回归）
- **根因**（真实账号实测复现）：门户用 **HTTP 200 + `data:null` + `meta.statusCode=302`** 表达会话失效，与匿名响应**逐字段相同**（`len=116`）；而 `extract_user_profile` 只认 `data.userName`、`profile_err` 又把所有非 HTTP 错误归 `Parse` → 用户看到「响应解析失败: tryLoginUserInfo 缺少 userName」；同时 `portal_probe` 仅凭「有 `customsid` + 首页未弹回 CAS」判定，**死会话被误报 Alive**，`check_session` 不清会话，落入「显示已登录、点什么都报错」的死状态
- **要点**：新增 `CampusAuthError::PortalNotLogin`；解析前**先判信封**（`meta.success==false` 或 `data` 空）；`profile_err` 映射为 `NotLogin`（「请先登录」文案首次可达）；`portal_probe` 追加鉴权信封判定，失效即 `Expired` → 启动清会话引导重新登录。live 复跑：新登录资讯 7 栏目正常、旧会话判 Expired；全仓 244 passed / 0 failed

## 2026-09-19 · M3 规划与侦察取证（含两条旧事实证伪）

- **模块**：`PLAN.md`（§3.2 慧新E校事实纠错 + §M3 细化）、`docs/superpowers/plans/2026-09-19-m3-ecard-electricity.md`（任务级计划，4 批）
- **要点**：**证伪**「电费 `charge-pc/pays/450` 匿名可用、返回剩余金额/单价」——该 URL 是 Vue SPA 壳，真链路为 `GET /charge/feeitem`（**唯一匿名可读**）→ `singleFeeitem` → `getThirdData` 三级级联；**一卡通流水在独立的 `berserker-search` 服务**（非 `berserker-app`）；`appScheme/info` 需 `?type=user&serviceType=<agentType>`（匿名可读）；同步纠正 `PLAN.md`、`docs/HANDOFF.md`、项目记忆库三处同源错误

## 2026-09-19 · 全量补齐收口：批 10 多课表、批 11 多方案/全局课程管理经用户裁决砍掉（单校定位，无代码改动）

- **模块**：`docs/HANDOFF-timetable-parity.md`（§七终态补记）、`CHANGELOG.md`；批 10 已实现代码**未提交即丢弃**（worktree reset），契约 §16 未合入
- **裁决**：本仓只绑定无锡学院（单校直连），多课表无使用场景——批 10 存储 v2 已由 glm5-3-flash 实现并通过三件套与回归测试，用户拍板放弃后整批丢弃，未进 master；批 11（多方案编辑 + 全局课程管理）评估为单校场景性价比低（同名多方案概率低、课表仅约 8 门课），一并砍掉
- **终态**：P1-P5 补齐以 P0 + 批 1-9 为准全部完成；明确不做清单新增「多课表管理」「多方案编辑」「全局课程管理」三项（将来多校/安卓版再议）

## 2026-09-19 · 全量补齐批 9（P5-e）：今日页接本地课表（本地优先、教务兜底）

- **模块**：`commands/timetable.rs`（`get_today_courses` 命令 + `today_courses` 纯函数 7 个单测）、`TodayPanel.tsx`（与门户 overview 并行取数、互不阻塞）、契约 §15
- **要点**：本地课表（含 override/停开/手动/自定义时间/跳过日）成为「下一节课」与「今日课程」列表的数据源；`has_local=false` 回落门户现状零变化；state 四态（normal/no_semester/vacation/skipped）；ongoing/next 后端按本机时钟算好下发，前端不自算时钟；已结束置灰按「位于 next 之前」推导

## 2026-09-19 · 全量补齐批 8（P5-d）：调课搬迁 + 快速删除

- **模块**：`commands/timetable.rs`（`move_day_courses`、`quick_delete` 两命令 + 6 单测）、设置弹层「批量调整」区块、契约 §14
- **要点**：搬迁单模式统一走 override（手动+导入，`move:<from>:<to>` 一批、整批可撤销），周次判定走 `week_index_at_date` 对齐口径（R8 禁 epoch 直除）；快删按周次×星期组合移除、weeks 删空删整条（级联清 override）

## 2026-09-19 · 全量补齐批 7（P5-c）：周次选择弹窗 + 顶栏态机 + 非本周降级 + 色板自定义

- **模块**：`commands/timetable.rs`（`week_state` 四态）、`model.rs`（`show_non_current_week`）、`uiStore`（persist v2 + `customCourseColors`）、`TimetablePanel.tsx`（周次弹窗/三态文案/降级渲染/`coursePalette` 合成）、契约 §13
- **要点**：「第 N 周」点开 1..M 网格跳转；unset（可点开设置）/before（距开学 N 天）/vacation（假期）三态；非本周课开关式降级显示（40% 透明、可点不可拖）；色板 = 8 固定 + 原生取色器自定义段，导入课哈希取色 `% 合成长度`（旧数据 0..7 行为不变）

## 2026-09-19 · 全量补齐批 6（P5-b）：课表 JSON 导入导出 + ICS VALARM

- **模块**：`commands/timetable.rs`（`export_timetable_json`/`import_timetable_json`/`build_ics_with_reminder` + 5 单测）、导出区 UI、契约 §12
- **要点**：导出含 courses+overrides+config 全量（roundtrip 单测）；导入一次落盘（禁止先清后写两次 IO）、config 非空才覆盖、非法中文报错不落库；VALARM 0-60 分钟可选（`TRIGGER:-PT{n}M`，缺省无、golden 保）

## 2026-09-19 · 全量补齐批 5（P5-a）：手动课程自定义时间等表单五件套

- **模块**：`commands/timetable.rs`（ManualCourseInput 扩 custom 字段 + 备注 300 截断）、`diff.rs`（format_weeks 单双周后缀）、CourseForm/SlotsEditor（按时刻开关、别名输入、dirty 拦截、N/300 计数）、`customBlockRange` 相交落块、契约 §11
- **要点**：custom 课网格按「与各大节相交的 min..max 大节」渲染（无相交仅列表可见）、不可拖；dirty 时关闭弹 confirm；`1,3,5,7` → `(单周)`、全偶 → `(双周)`（前后端同语义）

## 2026-09-19 · 全量补齐批 4（P4）：网格拖拽改课（交叉复核修复后合入）

- **模块**：`TimetablePanel.tsx`（拖拽状态机 +213 行）、契约 §10；deepseek-flash 复核 + glm5-3-flash 修复 7 项
- **要点**：4px 阈值区分点击/拖拽、setPointerCapture、suppressClick、落点纯前端几何（跳过日列无效回弹——复核修复 P0：原实现会落 day=0 脏数据并白屏）；落库分叉：多周/导入课 → 单周 Rescheduled override（`drag:<courseId>:<week>` 幂等）、单周手动课直改；跨度换算按**小节差**守恒（复核订正实现误读的大节差）；pointerId 防多指、blur/lostpointercapture/周切换复位

## 2026-09-19 · 全量补齐批 3（P3）：slot_rules 组合作息（冬/夏按日期区间自动切换）

- **模块**：`model.rs`（SlotRule）、`commands/timetable.rs`（`effective_slots_at` 区间命中、`save_slot_rules` + 8 单测）、SlotsEditor 规则区块、契约 §9
- **要点**：命中含端点、区间重叠取先声明、三段回落链 rules→config.slots→内置；空 slots 规则不算命中（防御手改 JSON）；TimetableView.slots 取「今天」生效作息（跨区间周的已知取舍，注释写明）

## 2026-09-19 · 批 2 交叉复核修复：extra 覆盖面两处数据丢失 + 前端补调块单位 bug + UID 防撞（deepseek-flash 复核、glm5-3-flash 修复）

- **模块**：`campus-schedule/occurrence.rs`（extra 循环独立化 + P3-c 登记 + 5 个新单测）、`commands/timetable.rs`（ICS 周次并集 + UID 后缀 + 溯源映射 + 4 个新单测）、`TimetablePanel.tsx`（P2 补调块单位修复 + 互锚注释）、契约 §8.4/§8.5 增量、CodeWiki 三篇
- **必修**：① **P1-a** cancel/resched 分支提前 `return` 吞掉 extra 循环（停课+补课 → 网格有补课块、展开结果丢失）→ extra 移出短路路径恒执行；② **P1-b** `build_ics` 只迭代 `course.weeks` 漏补课周 → 改 `course.weeks ∪ 各 override.weeks` 去重排序；③ **P2（M2.5 既有 bug）** `buildWeekBlocks` 调课新位 `newEnd` 缺省误用 raw 小节号当大节下标（块虚高数倍挤压同列分列）→ 改用已折算 `newStart`
- **轻量采纳**：④ **P3-a** 调课新位/补课 UID 追加 `-o{override 前 8 位}`（原位含仅换教室不变，golden 保）；⑤ **P3-b** DESCRIPTION 溯源改预建 id→类型映射
- **登记**：⑥ **P3-c** custom 课 override 语义（不进网格；被 resched 走大节表）写入 occurrence.rs 模块注释与契约 §8.4，两处互锚
- **验证**：`cargo test --workspace` 全绿（campus-schedule 54 + campus-hub lib 52 + portal 49，新增 9 测试含复核 7 条）；`tsc --noEmit` 零错；`npm run build` ✓

## 2026-09-19 · 全量补齐批 2（P2）：skippedDates + ICS 按生效结果展开（override/跳过日/自定义时间）

- **模块**：`campus-schedule/model.rs`（`CourseTableConfig.skipped_dates` 字段）、`campus-schedule/occurrence.rs`（**新建**：`expand_occurrences` 生效实例展开纯函数 + `OccurrenceKind`/`CourseOccurrence` + 11 个单测）、`campus-schedule/lib.rs`（导出）、`commands/timetable.rs`（`effective_slots` → `effective_slots_at(config, date)` 签名迁移、`build_ics` 重写为按生效结果展开、新命令 `save_skipped_dates` + 7 个单测）、`infra/timetable.rs`/`weeks.rs`（fixture 补新字段）、`lib.rs`（注册）、`TimetablePanel.tsx`（跳过日列「休」徽标/日期置灰/课程与空位不渲染 + 设置弹层「跳过日期」区块）、`types.ts`、契约文档 §8
- **蓝图**：批 2（P2），glm5-3-flash 执行；ICS 消费 override（复核并入项 1）+ custom 时间课接入（附注 5）
- **要点**：停课不生成 VEVENT（决策 5：否决 STATUS:CANCELLED）；调课原时段消失、新时段新 UID（`{id}-w{week}d{newDay}s{newStart}@campushub`，与原 UID 不同防日历端去重错乱）；补课新增 VEVENT；DESCRIPTION 追加「调课/补课」；跳过日 VEVENT 剔除；custom 课 DTSTART/DTEND 直取 `custom_*_time`（替掉旧版对无节次课程的 continue）；`expand_occurrences` 逐分支对照前端 `buildWeekBlocks`（停课两档/仅换教室原位/逆序取最后/停课优先调课，受控双写 + 注释互锚）；`effective_slots_at` 的 date 参数本批只迁移签名不消费（P3 扩展点，决策 2）
- **偏离**：`CourseOccurrence` 在决策 5 结构规格外加 `source_override_id: Option<String>`（Solid 实例无法仅凭几何字段区分原位/调课新位/补课，ICS 的 DESCRIPTION 标注需要 override 溯源；已冻结进契约 §8.4）
- **验证**：`cargo test --workspace` 全绿（campus-schedule 48 + campus-hub 49 等 0 失败；新增：expand 六分支/ICS 停课消失/调课新 UID/补课新增/跳过日剔除/custom 时刻/旧 JSON 无 skippedDates 键无损）、`tsc --noEmit` 零错、`npm run build` ✓（chunk 警告既有）

## 2026-09-19 · 全量补齐批 1（P1）：课表设置弹层 + showWeekends/firstDayOfWeek 接线 + ICS 周首日对齐

- **模块**：`campus-schedule/weeks.rs`（`previous_or_same_day_of_week` 转正导出）、`commands/timetable.rs`（新命令 `save_semester_config` + `apply_display_constraints` 后端单点联动 + ICS 对齐式日期 + 5 个单测）、`TimetablePanel.tsx`（列头旋转/5 列裁剪/weekDates 重写/SettingsEditor 弹层）、`types.ts`、契约文档 §7
- **蓝图**：`docs/superpowers/plans/2026-09-19-timetable-p1-p5.md`（glm5.3 规划轮，11 批）· 本批 = 批 1，glm5-3-flash 执行、主智能体复跑验证
- **要点**：开学日/总周数/当前周（hint 反推后端单点）手动设置整块补齐（复核并入项 2，`semester_start_from_week` 死代码激活）；firstDay=7 ⇒ showWeekends 强制开、关周末 ⇒ firstDay 回周一（双向联动收口后端）；ICS 日期改 `previous_or_same` 对齐式（firstDay=1 且周一开学输出不变，golden 保；新增非周一开学/firstDay=7 测试）；前端网格按 firstDay 旋转、隐藏周末列、`todayCol` 按显示列口径（规避 R1 陷阱）
- **偏离**：hint 校验加严 `1..=totalWeeks`（防反推出学期外日期）；`DAY_HEADERS` 常量删除改 `DAY_NAMES[displayDayOf(i)]` 等价映射
- **验证**：`cargo test --workspace` 全绿（41+49+37）、`tsc --noEmit` 零错、`npm run build` ✓；深色模式与真机视觉待用户验收

## 2026-09-19 · 课表对账复核轮：glm5-3-flash 双向交叉复核，P1-P5 补 3 实质漏网 + 行号修正（无代码改动）

- **模块**：`docs/HANDOFF-timetable-parity.md`（新增 §六复核轮补记）、`CHANGELOG.md`；**无代码改动**
- **背景**：用户要求「检查与原项目是否有没有对齐的功能」，点名 glm5-3-flash 分析。两个 glm5-3-flash 并行：上游侧（逐 ui 功能域/repository/tool 对照清单找漏记）+ 本仓侧（逐条 grep 验证 P1-P5 + 零消费字段盘点），主智能体抽查核实关键证据
- **核实属实的新发现（并入路线）**：① **ICS 导出不消费 override**（`build_ics` 只遍历 courses，调课/停课/补课通知在导出日历中不生效——数据正确性，并入 P2）；② 开学日/总周数手动设置整块缺失、`semester_start_from_week` 为零调用死代码（并入 P1 设置入口）；③ 今日页与本地课表是**数据源分叉**（走教务接口，override/手动课全不反映，P5 条目表述升级）；④ ICS 日期未按周首日对齐（并入 P1）；⑤ 自定义时间课被 ICS 静默跳过（P5 落地时同步接）
- **文档修正**：重叠分列行号应为 `grid.rs:153` 起（:16-140 是拖拽预留件）；`add_course_manual` 后端也硬编码 `is_custom_time: false`（不止前端）
- **上游侧结论**：清单 17 节覆盖面完备，无清单外桌面端适用实质缺口；4 条细节并入相应条目（详情弹窗编辑按钮本仓已有等价覆盖，其余 3 条并入 P5/P4 记录）
- **P1-P5 原有条目**：逐条 grep 全部属实，无一虚报、无需删除

## 2026-09-19 · P0 课表样式对齐上游安卓端：时间列起止时间、课程块教师名、点空白格新建课程

- **模块**：`tauri-app/frontend/src/panels/TimetablePanel.tsx`（渲染层）；契约修订 `docs/superpowers/plans/2026-09-18-m2.5-timetable.md` §6；CodeWiki decision `timetable-block-granularity`
- **内容**：① 时间列由「节号 + 开始时间」改为「节号 + 起止时间两行」（上游截图规格 §P0-1）；② 课程块三要素对齐截图：课名 → 教师（空不渲染）→ `@教室`（§P0-2）；③ 天列每大节行铺空位按钮（渲染在课程块之下、不遮挡块点击），点击预填星期/大节（小节 `2k-1..2k`）/展示周新建课程，未设开学日（`currentWeek===null`）时提示「请在学期内添加课程」，对齐上游「学期外不可添加」语义（§P0-6）
- **粒度决策**：否决 HANDOFF 建议的 A 案（按小节行渲染）——`slots` 行数不固定（自定义作息契约）下「45+10+45」小节时间推导会出错，且引入第三套坐标在 `isCustomTime`（P5）前无收益；采纳 B 案（大节行 + 视觉要素对齐），理由落盘契约 §6 与 CodeWiki decision
- **不做**：网格左上角「年份/周数」角标（顶栏已有「第 N 周 / 共 M 周」等价信息，HANDOFF 亦标注锦上添花）
- **验证**：`tsc --noEmit` ✓；`npm run build` ✓（chunk 体积警告为既有现象）；`cargo test --workspace` ✓（36+49+37 全绿，0 失败）；真机视觉对照留待用户验收（依赖 tauri 后端与真实登录态）

## 2026-09-19 · 交互级对账补充：上游「点空白新建」等 9 项交互缺口并入交接计划（无代码改动）

- **模块**：`docs/HANDOFF-timetable-parity.md`（P0 与 P5 两节）、`CHANGELOG.md`；**无代码改动**
- **背景**：用户以「课表界面点击空白处可以添加课程」为例追问交互级缺口——上轮 P0–P5 是功能域粒度，交互细节有漏网。主智能体 grep 核实本仓 `TimetablePanel.tsx` 全部 22 处交互入口后逐条对照上游清单
- **核实结论**：天列空白格无任何 onClick（`:1161-1163` 只挂课程块 button），点空白新建确缺；周次切换只有 ◀/本周/▶ 无弹窗；表单无未保存拦截；`TimeSlot.alias` 已迁、SlotsEditor state 保留但无输入框
- **并入计划**：P0 补「点击空白格新建课程（预填星期/节次/当前周）」——预填机制 `openForm(initial, …)` 现成，成本极低；P5 补 8 行：周次选择弹窗、开学前/假期标题态、手动课程自定义时间（isCustomTime 已迁表单未暴露）、节次别名编辑、表单未保存拦截、备注 300 字计数、ICS VALARM 提醒、多课表管理（`DEFAULT_TABLE_ID` 已预留）、多方案编辑+全局课程管理（大件，动模型）
- **已覆盖确认**（不缺口）：详情浮层「编辑/手动添加同款」、作息新增行时间自动推算（大节 100min 语义）、周次单双周快捷键、导入颜色同名同色（课名哈希）

## 2026-09-19 · 课表对账交接计划：shiguangschedule 全量对账 + P0–P5 补齐路线 + 安卓端储备

- **模块**：`docs/`（新增 2 份文档、`HANDOFF.md` 3 处接续）；**无代码改动**
- **背景**：用户核对「上游课程表项目（shiguangschedule，KMP 课表 App，201 个 .kt）的完整功能是否都迁移、不要漏掉完整设计」。deepseek-flash 全量通读上游源码出全量清单，glm5-3-flash 盘点本仓已实现面，主智能体抽查核实关键字段（config 字段、`format_weeks`、reminder/节假日/调课搬迁在本仓零命中等）
- **对账结论**：课表内核齐（数据模型/周次计算/重叠分列/默认节次/正方解析，另有自建 override/diff/notice 优于上游）；缺口集中在**交互层与周边功能**——组合作息（冬/夏自动切换）、拖拽改课（grid.rs 坐标互转已预留未接线）、节假日 skippedDates 全链路、手动调课搬迁与快速删除、JSON 导入与备份恢复、`showWeekends`/`firstDayOfWeek` 已迁字段前端零消费、时间列无结束时间、课程块缺教师名
- **用户指示落档**：①安卓组件/通知/勿扰/日历同步等平台强绑定设计**不算放弃**，归入「安卓端储备」待出安卓版时按清单实现；②课表样式对齐上游安卓端截图（时间列节次号+起止时间、课程块课名+教师+@教室、年周角标），列为 **P0 下一任务**；③上游全量清单落盘存档防漏
- **交付物**：
  - `docs/HANDOFF-timetable-parity.md` —— 对账结论 + P0–P5 路线（每项含上游证据/现状/验收）；P0 粒度两案（按小节行渲染推荐 / 大节行最小改，小节时间可由大节 100min 按 45+10+45 推导）；P4 拖拽导入课程建议落 `Rescheduled` override 而非直接改字段（与本仓模型自洽，优于上游直接改库）
  - `docs/upstream-shiguangschedule-inventory.md` —— 上游 17 节全量功能与算法清单（含 `文件:行号` 证据、上游自身三处周次口径不一致等坑位标注），兼作未来安卓端设计参考
  - `docs/HANDOFF.md` —— 「〇」补记追加、第八节「下一步」存档改写（M2/M2.5 已完成，指向新计划）、文档索引补 2 行

## 2026-09-18 · M2.5 收尾轮真机点验（CDP 打通 UI 交互，上一轮「点击未点验」缺口补齐）

- **模块**：验收与文档（产品代码改动仅来自下条点验驱动的 ICS 修复）；`.codewiki/learnings/tauri-webview-ui-verification.md` 大幅补充
- **突破口**：给 WebView2 开远程调试端口（启动 dev 时带 `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9222`，**不改仓库代码**），用 CDP `Runtime.evaluate` 触发真实 DOM click、必要时用 `Input.dispatchMouseEvent` 派发带用户手势的鼠标事件——绕开「坐标点击被别窗接收 + WebView2 不派发 UIA Invoke」的死局
- **逐项点验结果（真实账号 + 真机窗口）**：

| 交互 | 结果 |
|---|---|
| 导入 / 同步 | ✅ 新增 8 · 更新 0 · 停开 0 · 共 8 门；第 2 周 / 共 19 周 |
| 课程详情浮层 | ✅ 教师 / 教学班 / 周一 3-4 小节（第 1-12 周）/ 性质·考核「必修·考试」+「手动添加同款」「编辑」 |
| 手动添加（受控表单填值 + 提交） | ✅ 8 → 9 门、标「手动」，随后二次导入仍保留（零变动复验） |
| 调课通知（高置信自动应用） | ✅ 解析 → conf=high → 采纳 → 第 5 周出现【调】角标新时段块 + 原时段「已调出」虚线 |
| 停课两档（契约 §2.5.1） | ✅「第5周周五1-2节 …停课」只停该次；「第6周 …停课」（未提星期）该课该周全停 |
| 撤销此通知调整 | ✅ 3 条 override 全部撤销，落库 `sourceNoticeId` 计数归 0 |
| 删除课程（含 confirm 确认） | ✅ 假课程移除，回到 8 门 |
| 作息弹层（改时间 → 保存 / 恢复默认） | ✅ 08:00→08:30 网格同步 + 落库 `startTime`；恢复默认后 `slots: null` 且回 08:00 |
| 导出 ICS | ✅ 写出 `E:\ik\download\课表.ics`（23136 字节，`file` 识别为 iCalendar calendar file，82 个 VEVENT，CRLF） |
| 深色模式 / Dock 9 项视觉 | ✅ `html.dark` + 底色 `#17151C`，域色课程块与【导】角标对比度正常；Dock 9 项不溢出 |

- **点验驱动的修复**：ICS 导出原用前端 Blob 下载（桌面端静默失效）→ 改后端写盘（详见上一条）
- **仍未点验**：旧 localStorage 8 面板持久化值的升级场景（需先清 `campushub-ui` 再打开）
- **验证**：`cargo test --workspace` → 153 passed / 0 failed / 4 ignored；`tsc --noEmit` 0 错误
- **验收产物清理**：假课程「验收手动课」、3 条 override、导出的 `.ics` 均已清除，库中只留真实 8 门课（`slots: null`、0 override）

## 2026-09-18 · M2.5 收尾轮修复：ICS 导出改由后端写入下载目录（WebView2 不处理前端 Blob 下载）

- **模块**：`tauri-app/src-tauri/src/commands/timetable.rs`、`tauri-app/frontend/src/panels/TimetablePanel.tsx`、`docs/superpowers/plans/2026-09-18-m2.5-timetable.md`（§2.3 命令表）、`.codewiki/`（learning 补节 + 模块文章）
- **缺陷现象（真机验收发现）**：课表页「导出 ICS」点击后无任何反应——无提示、`%USERPROFILE%\Downloads` 无文件、全盘无新 `.ics`、页面无报错；CDP `Input.dispatchMouseEvent`（真实鼠标事件、带用户手势）重试同样无效
- **根因**：Tauri/WebView2 默认不处理下载（`DownloadStarting` 未接管），前端 `URL.createObjectURL` + `a[download]` 静默失效——Blob 下载交付在桌面端不可用
- **修法**：
  1. 后端 `export_ics` 改为生成 ICS 后写入 `dirs::download_dir()` 下的「课表.ics」（已存在直接覆盖，不加时间戳后缀；取不到下载目录 / 写盘失败均 `CommandResult::err` 中文原因并附系统错误信息），返回写入的完整路径；写盘抽成纯函数 `write_ics_to(dir, text)` 并新增临时目录单测（断言固定文件名、内容含 `BEGIN:VCALENDAR`、覆盖语义）；`build_ics` 纯函数未动
  2. 前端 `exportIcs()` 删除 Blob / `createObjectURL` / `revokeObjectURL` / 动态 `<a>` 段，改为调命令后在既有提示位显示「已导出到 <路径>」或错误原因
  3. 契约 §2.3 `export_ics` 行同步改写并注明根因；CodeWiki learning 文章补「WebView2 不处理下载：不要用 `a[download]` 交付文件」节
- **验证**：`cargo test --workspace` → **153 passed / 0 failed / 4 ignored**（基线 152 + 新增写盘单测 1）；`npx tsc --noEmit` 0 错误；`npm run build` 通过
- **真机点验（补记）**：已被随后的 CDP 远程调试点验覆盖——点「导出 ICS」→ 提示「导出到 `E:\ik\download\课表.ics`」，文件 23136 字节、`file` 识别为 iCalendar calendar file、82 个 VEVENT、CRLF 换行，见下一条

## 2026-09-18 · M2.5 收尾轮：作息时间表可编辑 + 停课 override 渲染口径冻结

- **模块**：`crates/campus-schedule`（model/weeks）、`tauri-app/src-tauri`（commands/timetable、infra/timetable、lib.rs）、`tauri-app/frontend`（TimetablePanel.tsx、types.ts）、`docs/`（计划 §2.1/§2.3/§2.5.1 契约追加）、`.codewiki/`
- **作息时间表编辑（契约 §2.1/§2.3 收尾轮追加落地）**：
  1. `CourseTableConfig` 新增 `#[serde(default)] slots: Option<Vec<TimeSlot>>`（旧文件缺省 `None`；`None`/空 = 内置校本大节表，有值 = 用户编辑过的唯一事实源）；`TimeSlot` 补 derive `PartialEq`
  2. 取值单点化：新增 `effective_slots(config)`（`config.slots` 优先、回落 `campus_portal::block_time_slots()`），`build_timetable_view` 与 `build_ics` 全部改走它——网格行 / ICS 展开 / 大节号→时间查找三处同一份 slots，不再各调各的
  3. 新命令 `save_time_slots(slots: Option<Vec<TimeSlot>>) -> TimetableView`（命令数 34 → 35）：`None` 恢复内置、`Some(v)` 校验后保存（≥1 条、≤20 条、number 正整数严格递增不重复、HH:MM 且 end>start，非法一律中文 err）；返回刷新后的视图，前端免二次拉取
  4. 前端：顶栏「作息」按钮 → `SlotsEditor` 弹层（原生 `<input type="time">`，增删行、大节号 = 行序不手填、「恢复本校默认」仅自定义态可用）；保存成功用返回的 `TimetableView` 直接更新，网格行按 `slots.len()` 渲染（本就不写死 5）
- **停课 override 渲染口径（契约 §2.5.1 冻结）**：`new_day` 有值 = 只停「该周 · 星期 d」那一次；`None` = 该课在 weeks 列出的周次内整周全停。`buildWeekBlocks` 既有行为已符合两档，注释改为明确引用契约；`notice.rs`「提星期才填 new_day」现状与契约相符，未改动
- **取舍**：大节号由行序生成（天然严格递增，后端校验仅防御）；`Some(空数组)` 防御性归一为 `None`；今日页「下一节课」仍直连 `block_time_slots()`（无 config 可读，与课表页自定义作息可能短暂分叉，属已知取舍）
- **验证**：`cargo test --workspace` → **152 passed / 0 failed / 4 ignored**（基线 148 + 新增 4：作息校验 / parse_hm / effective_slots 回落 / 自定义行数视图组装）；`tsc --noEmit` 0 错误；`npm run build` 通过
- **遗留**：作息弹层与停课两档渲染未真机点验（点击注入限制，待主智能体 WebView2 远程调试点验：一条「只停周四」+ 一条「没提星期」的通知）

## 2026-09-18 · M2.5 收尾：真机验收（导入 8 门课 / 调课自动生效 / 手动课程零变动 / ICS）

- **模块**：验收与文档（无产品代码净改动）；`.codewiki/learnings/tauri-webview-ui-verification.md` 新增
- **真机验收（真实账号 + 真机 Tauri 窗口，2026-09-18 晚）**：
  1. **教务导入**：`导入完成：新增 8 · 更新 0 · 停开 0 · 共 8 门`；顶栏「第 2 周 / 共 19 周」（学期与教学周来自门户学期信息）；周网格按后端下发的 5 大节作息（08:00/10:10/13:45/15:35/18:30）渲染，每块带【导】角标，今日列高亮；块位置与节次一致（如「信息安全 周二 5-6 节」落大节 3 = 13:45，`ceil(5/2)=3`）
  2. **自动对比更新幂等**：再次导入 → `新增 0 · 更新 0 · 停开 0`，课程集合不变
  3. **手动课程零触碰**（PLAN 验收项）：手动添加一门后二次导入仍原样保留（列表标「手动」），未被停开、未被覆盖
  4. **调课通知自动调整**（PLAN 验收项）：粘贴「第5周周四3-4节 信息隐藏与取证技术 调整到 D4-305」→ `conf=high`（reason 为空）→ 采纳后第 5 周出现【调】角标新时段块 + 原时段「已调出」虚线占位，「已生效调整」列表可整批撤销
  5. **ICS 导出**：`export_ics` 返回 **29788 字符 / 129 个 VEVENT**
  6. **TGT 持久化**：登录后 `session.json` 出现 DPAPI 密文 `tgtB64`，教务会话失效时可静默重 SSO（无 TGT 时报「教务会话已失效，请重新登录」）
- **验收手段说明（重要，供后续接手者）**：本机 ZCode 长期占据前台（`GetForegroundWindow()` 为 ZCode），坐标点击被别窗接收、UIA `InvokePattern`/AXPress 在 WebView2 上只设焦点不派发 DOM click（Dock 面板切换等少数按钮例外），**鼠标交互无法注入 Tauri 窗口**。故本轮改用「临时验收代码（面板挂载时自动跑一次动作序列）+ 结果落库回读」取真机证据：临时块经 `git checkout` 完全移除（**未进任何提交**），验收产物（2 门假课程 + 1 条 override）已从 `%APPDATA%/campushub/timetable.json` 清理，含产物的备份仅留在 `%TEMP%/campushub-m25-verify/`。**「鼠标点击 → 命令」这一段未经真机点验**，仅有源码级证据（dev server 下发的模块中确认存在 `onClick: doImport`）。详细方法与判据见 CodeWiki 学习条目。
- **验证**：`cargo test --workspace` → **148 passed / 0 failed / 4 ignored**；真实账号 live `jwglxt_sso_kbcx_live` ok；`tsc --noEmit` 0 错误
- **遗留**：① 详情浮层定位、Dock 9 项视觉、深色模式色板对比度未逐项点验（受注入限制）；② ICS 的浏览器下载行为未点验（命令返回已在真机序列中走到）

## 2026-09-18 · M2.5 批次 4：课表页 UI（周视图 / 详情浮层 / 手动课程 / 调课待确认 / ICS 导出）

- **模块**：`tauri-app/src-tauri`（`get_timetable` 出参按契约修订改为 `TimetableView`）、`tauri-app/frontend`（`PanelId` 8→9 项、`TimetablePanel.tsx` 新建、`types.ts` 课表契约 12 接口、DockNav/AppShell/uiStore/index.css 接线）、`docs/`（计划 §2.3 TimetableView 修订落档）、`.codewiki/`
- **`TimetableView` 修订（批次 4 前契约修订落地）**：`get_timetable` 返回 `TimetableView{ timetable, slots, currentWeek, today }`——`slots` = `campus_portal::block_time_slots()` 下发（**时间标签唯一事实源，前端不硬编码**）、`currentWeek` = `campus_schedule::current_week`（无开学日/今天越出学期 → null）、`today` = "YYYY-MM-DD"。组装抽纯函数 `build_timetable_view` 可单测；`parse_notice` 的周次口径与之同源。取舍见 `.codewiki/decisions/timetable-view-contract.md`。
- **前端契约镜像（`types.ts`）**：M2.5 课表 12 接口照 Rust camelCase 序列化——`Course`/`CourseOverride` 的 Option 字段恒存在 `| null`；`NoticeCandidate` 的 Option 字段 Rust 侧 `skip_serializing_if` 缺省省略 → TS 用**可选属性**（两类形态不可混写）。`PanelId` 追加 `"timetable"`（types + DOCK_ITEMS + PANEL_MAP 三处同步）。
- **uiStore persist 兼容**：persist 升 `version: 1` + `migrate`——旧持久化值（8 面板之一）原样保留，非法 `activePanel` 兜底回 `"today"`（防 `PANEL_MAP` 查空白屏）。
- **DockNav 9 项**：课表（`CalendarRange` 图标、sched 湖蓝域色）置于「今日」之后；9 项总宽约 416px，1280px 默认窗口不溢出；磁吸/胶囊/圆点动画按 `DOCK_ITEMS` 遍历注册，无需额外适配。
- **课表色板**：`COURSE_PALETTE` 8 档全走 token——6 个既有域色 + index.css 新增 `--color-aqua`/`--color-rose`（深主题各提亮一档）；**导入课程 `colorIndex` 是课名哈希大数（`stable_color`），取色一律 `% 色板长度`**，与手动课程 0..=7 下标统一。
- **`TimetablePanel`（单文件）**：
  - 顶栏：`第N周 / 共M周`（N=视图周，默认 currentWeek，null 时按第 1 周并显示「尚未设置开学日」琥珀提示）+ `◀ 本周 ▶`（clamp 1..totalWeeks）+ 「导入/同步」（成功摘要条：新增/更新/停开/共 N 门 + `changes[]` 逐条、失败中文红字；成功后回当前教学周）+ 「导出 ICS」（Blob 下载「课表.ics」）。
  - 周视图：列 = 周一…周日（列头日期由 `semesterStartDate + (周-1)×7` 推、今日列按 `today` 字符串比对高亮）；行 = 后端 `slots` 5 大节；块 absolute 按大节跨度铺（小节→大节 `ceil(小节/2)` 与 ICS 展开同口径）；【导】= source=import、【调】= 该块关联生效 override。
  - override 合成（`buildWeekBlocks` 纯函数）：停课 → 原时段虚线「已停」占位；调课（新时间≠原时间）→ 原时段虚线「已调出」+ 新时段实体块；仅换教室 → 原位渲染新教室；补课 → 新增实体块；同一课多条 override **逆序取最后一条**（与后端 upsert 幂等呼应）；同日重叠轻量分列（连通簇 + 贪心占道，`grid::merge_courses` 语义的前端重写，不在 IPC 面）。
  - 详情浮层：fixed 定位（视口 clamp、下放不下上翻、Esc/点外关闭）；教师/教学班（classId）/周次/教室；**`remark` 按来源区分标签**——导入课程装的是「课程性质·考核方式」（正方 `kcxz·khfsmc`），手动课程才是备注；该课 override 列表可撤销；「手动添加同款」预填表单 + 「编辑」（提示导入课程修改会被下次导入覆盖）。
  - 手动表单：名称/教师/教室/星期/起止小节/周次/颜色/备注，周次文本 `1-8,10` 混排解析 + 全部/单周/双周快捷；新增走 `add_course_manual`、编辑走 `update_course`。
  - 调课通知区：粘贴 → `parse_notice` → 候选列表（high=「可自动应用」绿标、low=琥珀 reason+excerpt）→「采纳」`apply_override`；已生效 override 列表「撤销此通知调整」= `revoke_notice(sourceNoticeId)` 整批撤销。
  - 全部课程列表：按星期/节次排序平铺，`disabled=true` 灰显 +「已停开」徽标（**不画进网格**），行内编辑/删除（删除 confirm，后端级联清理 override）。
  - 四态齐全：guest 登录空态 / loading 骨架 / ready 0 门「导入课表+手动添加」引导 / error 重试；无任何假数据。
- **验证**：`cargo test --workspace` → **148 passed / 0 failed / 4 ignored**（本批新增 2：`build_timetable_view` 组装断言 slots=5 大节/currentWeek 口径/camelCase 键 + 无开学日/开学前 → currentWeek=null）；`tsc --noEmit` 0 错误；`npm run build` 通过。真机（导入 8 门课显示、角标、手动课程多次导入零变动、Dock 9 项视觉）由主智能体验收。
- **遗留**：① 停课 override 的 `newDay=null`（通知未提及时）按「该课该周全停」渲染，契约未细化该点；② 视图周切换无日期越界防呆之外的提示（开学前/放假周网格空白属预期）；③ 作息时间表编辑 UI 不做（计划 §5 明确不做）。

## 2026-09-18 · M2.5 批次 3：调课通知 L1/L2 解析与 override 命令

- **模块**：`crates/campus-schedule`（新增 `notice.rs` 解析内核模块）、`tauri-app/src-tauri`（3 新命令 **31 → 34**，`commands/timetable.rs` 扩展）、`.codewiki/`
- **依据**：计划 `docs/superpowers/plans/2026-09-18-m2.5-timetable.md`（L1/L2 冻结语义 §2.5、命令面 §2.3——本条不重复抄契约）
- **`notice.rs` 纯函数内核**（`parse_notice_text(text, courses, current_week) -> Vec<NoticeCandidate>`，全部 std 字符串处理，**零新依赖**——workspace 原无 regex）：
  - **L1 提取**：课程名 = 对本地课程名做 contains 匹配（`《信息安全》` 书名号形态天然命中子串；多名命中时裁剪被长名包含的短名——文本含「信息安全实验」必含「信息安全」）；周次 `第3周` / `3-4周`（区间优先，避免把「4」拆成单周）/ `本周`（需 `current_week` 锚点，None = 无法确定）；星期 `周一…周日` / `星期一…星期日` / `星期天`（「周二至周四」区间取第一个，降级场景由用户确认）；节次 `3-4节` / `第3节`（**单节强制「第」前缀**，避免「共16节课」「3节连上」节次数误提）；教室 `D4-207` 形态 token（`字母数字-数字`）优先、「教室：/教室:」后内容兜底；类型关键词 停课>补课>调课（无关键词默认 Rescheduled，不影响置信）。
  - **新旧时间消歧**：调课通知「由周一3-4节调整到周四5-6节」的新时间在箭头词（调整到/调至/改到/换到/更换为…12 个）之后——周次/星期/节次/教室四提取器一律 **tail 优先、全文回退**（新时间缺表述时旧表述仍可命中）。
  - **L2 置信**：课程唯一命中且周次/星期/节次齐全 → `high`（reasons 为空 ⇔ High）；缺任一要素 / 同名多门 / 多名 / 0 命中 → `low`，`reason` 中文写明每项降级原因（「；」连接）。
  - **`noticeId`** = `manual:<16 位十六进制>`（`DefaultHasher` 对正文哈希，同进程/同版本确定；**非密码学哈希，仅作去重与撤销键**，M5 接公告流时改用公告 id）。
- **3 条命令**（`commands/timetable.rs`；本地操作无需登录，沿用 `mutate_timetable` 骨架，无进程内互斥）：`parse_notice(text)` 读本地课表 + `current_week`（`chrono::Local::now()` + `campus_schedule::current_week`）解析、**不入库**；`apply_override(candidate)` 校验 courseId 落在本地课程（缺失/已删 → 业务失败「通知未匹配到本地课程」）→ 字段级拷贝写 `overrides`（**同一 noticeId+courseId 重复采纳幂等覆盖**，`auto_applied` = confidence==High）；`revoke_notice(noticeId)` 按 `source_notice_id` 整批删除返回条数（0 条幂等成功）。转换抽纯函数 `candidate_to_override`（命令与单测共用）。
- **语义裁决**：停课通知解析出的星期/节次**原样保留**进 override（model.rs `new_day` 注释同步更新——「停哪一次」需要 day 定位，一周多节次的课程只停指定那次；契约未细化该点，见 `.codewiki/decisions/timetable-notice-l1l2.md`）。
- **验证**：`cargo test --workspace` → **146 passed / 0 failed / 4 ignored**（本批新增 15：campus-schedule 25→37（notice 12——高置信/低置信缺要素/同名多门/0 命中书名号/停课/「本周」与区间/节次形态/星期形态/教室形态/类型关键词/noticeId 稳定），campus-hub 26→29（override 幂等覆盖/整批撤销/auto_applied 置信度 3 个））；`tsc --noEmit` 0 错误。模拟通知端到端由单测覆盖（真实文本 → 候选全字段断言）；live 粘贴解析由主智能体验收时跑。
- **遗留**：① 单次解析只产出 1 个候选——教务通知多条调整混排（「A 调至 X；B 停课」）需用户分次粘贴；② 「周二至周四」区间表述取第一个星期（可能非用户本意，Low 场景由用户确认）；③ 教室 token 形态（`字母数字-数字`）之外的教室名（如「C5科教中心313」）仅「教室：」兜底可提取。

## 2026-09-18 · M2.5 批次 2：课表自动对比更新 + 手动课程 + ICS 导出

- **模块**：`crates/campus-schedule`（新增 `diff.rs` 导入 diff 纯函数模块）、`crates/campus-portal`（`block_time_slots()` 提升 `pub` 并 re-export）、`tauri-app/src-tauri`（5 新命令 **26 → 31**，`commands/timetable.rs` 扩展）、`tauri-app/src-tauri/Cargo.toml`（补 `chrono` 依赖）、`.codewiki/`
- **依据**：计划 `docs/superpowers/plans/2026-09-18-m2.5-timetable.md`（diff 冻结语义 §2.4、命令面 §2.3、节次口径 §1.3——本条不重复抄契约）
- **diff 纯函数（`campus-schedule/src/diff.rs`）**：`diff_courses(existing, incoming) -> DiffResult{courses, added, changed, removed, changes}`——匹配键 = 课程名 + `class_id`（缺失退化仅课程名）；新出现新增 / 字段变化更新（星期、节次、周次、教室、教师五字段字段级文案，如 `信息安全 教室 D4-207 → D4-305`，多字段「；」连接）/ 消失置 `disabled=true` 不删记录；**Manual 课程零触碰**（不参与匹配、永不停开，专门单测）；已停开课程再次消失不重复计数（removed = 本次新发现）；停开课程复活（`disabled=false`）计入 changed；匹配成功整条采用新数据但 **id 沿用旧库**（override 挂 course_id，必须稳定）；周次排序后比较（手工乱序不误报）；`format_weeks` 连续区间合并（1-12）。取舍记录见 `.codewiki/decisions/timetable-diff-manual-and-ics.md`。
- **`import_timetable` 命令**：需登录（无会话「请先登录」；`JwglNotLogin` 中文透出「教务会话已失效，请重新登录」）。学期信息（会话内缓存）推导参数：`xnm` = `start_date` 前 4 位、`semester` `"1"→3 / "2"→12`（**不用 `grade`**，冻结口径）→ `fetch_timetable_json`（901→TGT 静默重进在 campus-auth 内部）→ `parse_kb_response` → diff 合并旧库 → 落库（同时以学期信息初始化/更新 `semester_start_date` 与 `semester_total_weeks`，单字段解析失败保留旧值）→ `ImportResult{added, changed, removed, total, changes}`（total = 合并后课程总数，含停开保留记录）。
- **手动课程三命令**（本地操作、无需登录）：`add_course_manual(input)`（`source=Manual`、colorIndex 由入参、id=`manual-<纳秒>` 与导入 id `<table_id>-<jxb_id>` 前缀不同永不冲突；入参校验：课程名/星期 1-7/节次/周次 ≥1）；`update_course(course)` 按 id 整条替换（任意来源可编辑）；`delete_course(id)` 级联清理该课程挂载的 override。三者共用 `mutate_timetable`（load→改→save）骨架；无进程内互斥（前端交互串行，契约 §2.2 原子性由调用方保证）。
- **`export_ics` 命令**：返回展开式 VEVENT 文本（不落盘，前端 Blob 下载）。每门未停开课程 × 其每个教学周各一个 VEVENT（不依赖 RRULE）；日期 = `semester_start_date` + `(周次-1)×7 + (星期-1)` 天；时间取校本大节作息——`campus_portal::block_time_slots()` 本批次提升 `pub`（与今日页同一事实来源，未复制常量、49 个既有测试全过），**大节号 = `(起始小节+1)/2`**、结束时刻取结束小节对应大节 end_time（`3-4节` → 10:10-11:50）；大节越界跳过不伪造；TEXT 转义 + CRLF；**floating local time**（无 `Z`/`TZID`，RFC 5545 合法、Outlook/Google 按导入时区解释）；缺 `semester_start_date` 报「请先完成一次导入」。
- **验证**：`cargo test --workspace` → **131 passed / 0 failed / 4 ignored**（本批新增 19：campus-schedule 14→25（diff 11 个）、campus-hub 18→26（ICS/导入辅助 8 个））；`tsc --noEmit` 0 错误。live（真实账号导入 8 门课）由主智能体验收时跑。
- **遗留**：① M2.5 批次 1 无 CHANGELOG 条目（其内容见提交 e155f98 的 commit message，已于批次 3 时补录为下方独立条目）；② `Semester` 暑期 `"3"` 未映射（契约口径仅 1/2，未知序号报错）；③ ICS 未做 RFC 5545 行折叠（字段均为短文本，实测远低于 75 字节）。

## 2026-09-18 · M2.5 批次 1：教务课表拉取 + TGT 持久化 + 本地课表存储（补录，原内容见提交 e155f98 commit message）

- **模块**：`crates/campus-auth`（`jwglxt.rs` 课表拉取、`error.rs`）、`crates/campus-schedule`（`model.rs`）、`tauri-app/src-tauri`（1 新命令 **25 → 26**，`infra/timetable.rs` 新建）
- **教务拉取（campus-auth）**：`fetch_timetable_json(tgt, xnm, xqm)` 先直接 POST `kbcx/xskbcx_cxXsKb.html?gnmkdm=N2151`（XRW+CT 头组，body 仅 xnm/xqm），**901（教务会话失效）且有 TGT 时经 `jwglxt_sso` 静默重进后重试一次**，重进失败/仍 901 归一为 `JwglNotLogin`；`tgt=None` 直接 `JwglNotLogin`。响应判定抽纯函数 `interpret_kbcx_response`（901→JwglNotLogin、200 JSON 原文透传解析交上层、200 登录页 HTML 与其他状态码→Parse），离线单测 4 个；`error.rs` 新增 `JwglNotLogin` 变体；含教务 SSO 侦察前置产出（`sso_ticket`/`jwglxt_sso`、live 测试）。
- **TGT 持久化**：`SessionRecord` 加 `tgtB64`（**DPAPI 密文**，serde default 兼容旧格式）；`persist_session`/`load_session`（返回 `StoredSession`）/`restore_session` 同步；`CasSession` 加 `tgt` 字段并在恢复时回填；`finish_login` 公共路径统一落盘 TGT（`login_saved` 免密重登同享）。单测覆盖 TGT roundtrip、落盘无明文断言、旧格式兼容。
- **课表存储与模型**：`campus-schedule model.rs` 的 `Course` 加 `disabled`（停开标记，serde default）；新增 `Timetable{config, courses, overrides, updated_at}`（camelCase）。`infra/timetable.rs` 新建：`timetable.json` 明文读写（非凭据，与 profile.json 同级），**缺失/损坏回空课表不报错**（课表页首屏不白屏，坏文件保留现场）。`commands/timetable.rs` 新增 `get_timetable`（无入参纯本地读取），`lib.rs` 注册（命令数 25→26）。
- **验证**：`cargo test --workspace` → **113 passed / 0 failed / 4 ignored（含 live）**；`tsc --noEmit` 0 错误；CodeWiki 4 篇更新 + index/meta 同步（18 篇 up to date）。live（登录后拉取返回 8 条并落库、未登录返回「请先登录」）由主智能体验收。

## 2026-09-18 · M2 遗留项：日程月视图/会议并入与应用可达性元数据

- **模块**：`crates/campus-portal`（新增 access 可达性模块、会议解析纯函数、会议查询与学期缓存）、`tauri-app/src-tauri`（1 新命令 **24 → 25**，`get_schedule_month` 并入会议）、`tauri-app/frontend`（SchedulePanel 月视图/角标、AppsPanel 可达性徽标与分级点击）、`docs/`、`.codewiki/`
- **依据**：计划 `docs/superpowers/plans/2026-09-18-m2-portal-pages.md`（接口通则 §1.1、冻结契约 §2.1，本条不重复抄表）；B 块以设计文档**附录 A**（应用中心 30 应用 SSO 实测矩阵）为唯一事实来源。最终形态见设计文档**附录 H**。
- **A1 日程月视图 + 每日计数角标**：
  - 新命令 `get_schedule_day_counts(startMs, endMs)` 透传协议层 `query_schedule_day_counts`（`getCountBetweenTime`，bs-schedule 独立信封）——前批已就绪的协议能力本轮补上 IPC 与前端消费。
  - 前端 `SchedulePanel`：头部新增 周/月 视图切换（切换器 + 箭头按视图切周/切月，月标题显示 `YYYY年M月`）；月视图 = 自然月网格（首格 = 当月 1 日所在周的周一，列序与周视图一致，5 或 6 行），日期格显示**每日日程数角标**（服务端 `count`），今日格高亮；**点击某天跳到该天所在周**（明细按当前 5 类过滤取数）。
  - ⚠️ **角标口径（如实）**：bs-schedule 计数接口**无分类过滤参数**，角标 = 当日全量日程数（不伪造过滤后计数，会议卡来源亦不计入）；「5 类过滤生效」体现为全不选时月视图同样不发请求显示空态、点击日期跳周后明细按 codes 过滤。
  - 降级：月视图计数失败只降级角标（错误条 + 重试），日历网格与跳转不受影响，不白屏。
- **A2 校级会议并入（日程页）**：
  - 接口：`GET api/uppexcard/ext/dynamicData/10.1.90.34/ZCHY?DJZ=<周次会议标题>&pageNum=1&pageSize=20`，门户信封 `meta.success`。
  - **DJZ 按周过滤（实测四组对照）**：`第二周会议日程安排表`→6 条、`第一周…`→3 条、`第九周…`→0 条、**无周次前缀→0 条**——标题必须由教学周次构造：`meeting_query_title(week)` = `第<中文数字>周会议日程安排表`（`week_to_chinese` 纯函数覆盖 1–99）；教学周次由区间起点与学期开学日推算（`teaching_week_of`，开学当周 = 第 1 周，开学前返回 None 走降级）。
  - 字段映射：`title=HYMC`、`place=DD`、`classifyCode="Default-Meeting"`（名称/色值由分类列表按 code 映射）；日期由 `NF`(年份)+`RQ`("9月15日") 推出、时间由 `SJ`("下午3:00") 转 24h——全角冒号归一、跳过前导非数字取段首数字、「下午/晚上」且 <12 加 12；**SJ 解析不出按全天（00:00–23:59:59.999），解析出则 `endMs=startMs`**（服务端只给开始时刻，不伪造结束时间，前端对等值显示单时刻）；NF/RQ 缺失或推不出日期的条目跳过。`ZCR`/`CXRY`/`CBDW`（主持人/参会人员/承办单位）拼进 `ScheduleEvent.extra`，前端详情卡展示。
  - **降级承诺**：并入在命令层 `get_schedule_month` 内完成——codes 未含「会议」分类 / 学期信息失败 / 周次推算失败 / 拉取或解析失败 / 0 条 → **空贡献，绝不影响课表日程与日历本身**；会议按教学周次会话内缓存（确认 0 条同样缓存、传输/解析失败不缓存下次重试）；`query_semester_info` 改会话内缓存（今日页与周次推算共用，数据会话内恒定）。
  - **可观测化（工程改进，事后补的教训）**：链路最初把失败全吞成空 Vec——真机出现「会议 0 条且无错误态」时无法定位。现每个失败点留一行 `[meeting-diag]` stderr（学期信息失败 / 周次推算失败 / 请求失败 / 解析失败，client.rs:540/546/580/594），只含环节 + 周次 + HTTP 状态 + 错误类别，**不含 JWT/cookie/响应体**；**成功路径零输出**。完整教训见 `.codewiki/learnings/meeting-proxy-week-title-and-observable-degradation.md`。
  - 离线定位记录：以真机捕获样本验证解析层（6 条解析成功）与 URL 编码层（`Url::parse` 对中文 query 自动 percent-encode，与官方请求逐字节一致），从而把真因范围收窄到网络响应/运行时输入；**早前那次静默 0 条的真因未留证据**（诊断是事后补的），失败路径可观测化作为本轮工程改进保留。
- **B 应用可达性元数据与提示（不实现 URL 包装）**：
  - 新模块 `access.rs`：附录 A 实测矩阵代码化 `ACCESS_TABLE`（`access.rs:31`——**cas 8 host**：whall/cxcyjy/jwgl/yd/`10.3.100.110`/fysso.chaoxing/lib/wanfang；**webvpn 7 host**：jxzlbz1/cwbx/tsgcnki/tsgieee/tsgscid/tsgwebof/tsgkjyy；**unavailable 2 host**：lw（SSO 断停自家登录页）/cwxu.flyread.com.cn（自有登录非 CAS））+ `classify_app_access`（`access.rs:56`，host 精确或子域后缀匹配、前缀伪造不命中，未命中回落 `external`）。仅收录有实测结论的 host——`sygl`/`tsggcszh`/`tsgzzwy`/`www1` 附录 A 无结论，按回落规则 `external`；「联创文印/馆藏数字化」host 未出现在目录 30 条 appLink 中，不编造进表。
  - `AppItem.access`（`"cas" | "webvpn" | "external" | "unavailable"`，`lib.rs:191`）在解析层推导——**不信任门户 isCas 字段**（附录 A：有标 cas 实则停自家登录页/仅 WebVPN 可达的），单测 `access_overrides_portal_iscas` 钉死「isCas=1 但表归 webvpn → 以表为准」。
  - 前端 `AppsPanel`：`accessBadge`（`AppsPanel.tsx:42`）webvpn → 「需校园网/WebVPN」、unavailable → 「暂不可用」徽标；点击策略（`openApp`，`AppsPanel.tsx:147`）——**webvpn 提示后仍打开原链接**（校内无感直达、校外失败有解释）、**unavailable 只提示不打开**（实测死链/自有登录，打开无意义）、cas/external 直开不变。
  - **WebVPN URL 包装未做（实测依据，归 M4）**：网关对未登录请求一律回落——会议代理端点实测无凭据 GET 返回 302 → `Location: 首页`；应用域名的三种明文包装形式（`/http/<host>`、`/https/<host>`、内网 IP）最终 URL **完全相同**（网关丢弃目标路径）。包装格式在无 WebVPN 会话前提下**无法验证**，故不写推测实现；「WebVPN 会话打通 + URL 包装 + A 类 CAS 直达签发」整体归 M4（论证见附录 H）。
- **验证**：`cargo test --workspace` → **102 passed / 0 failed / 3 ignored**（campus-portal **49**，本批新增 12：周次中文数字、自然语言时间 24h 转换、日期区间回读、教学周次推算、会议字段映射与降级、会议标题构造回归、区间保留回归、可达性精确/子域/回落归类、小写序列化、不信任 isCas 回归）；`cargo check -p campus-hub` 通过；`tsc --noEmit` 0 错误；`vite build` 通过；诊断收敛后复跑全绿（成功路径零 stderr 输出）。**真机验收**：月视图角标与周/月切换正常；周视图 4 课程块 + **6 会议块**同屏渲染（周二两场 / 周三 / 周四两场 / 周五，时刻与 `SJ` 解析一致）；应用页徽标与分级点击提示符合预期。
- **遗留**：① WebVPN B 类包装（M4，依据见上）；② 月视图角标为 bs-schedule 全量计数（无分类参数、不含会议，如实呈现）；③ 待办列表「有数据」路径仍未真机验证（沿用批次 2 遗留）；④ 慧新E校实时直连（可选增强，未做）。

## 2026-09-18 · M2 批次 3：应用页 + 日程页数据接线（M2 完成）

- **模块**：`crates/campus-portal`（应用/日程 DTO 与解析、图标代拉、URL 校验分工扩展）、`tauri-app/src-tauri`（4 新命令，20 → 24）、`tauri-app/frontend`（AppsPanel/SchedulePanel 接真实数据 + TS 契约）、`.codewiki/`、`docs/`
- **计划**：`docs/superpowers/plans/2026-09-18-m2-portal-pages.md`（实测接口全表 §1.2、请求通则 §1.1、冻结契约 §2.1 批次 3——本条不重复抄表）
- **`campus-portal` 新增 DTO 与解析**（全部纯函数 + 脱敏单测）：
  - DTO 六个：`AppItem`（`iconUrl` 为后端拼好的 data URL；`appIcon` UUID 以 `icon_id` 承载并 `#[serde(skip)]`，不透传 IPC）/ `AppGroup`（部门维度，id=name）/ `AppCatalog`（计划 §2.1 `{groups}` 的兼容扩展：追加 `pinned` = `queryMyStore` 收藏条目）/ `ScheduleClassify` / `ScheduleEvent`（classifyName/color 由分类列表按 code 映射补全——明细的 `scheduleClassifyName` 实测可为 null，不可依赖）/ `ScheduleDayCount`
  - `schedule_data`：**bs-schedule 独立信封** `{code:"0",msg,data}`，成功判据 `code=="0"`（宽松兼容 code 为数字的形态），失败取 `msg`/`message` 给可读中文——与门户 `meta.success` 信封分开、不混用解析器（计划 §1.1 的两信封约定落地）
  - 解析五件套：`parse_app_groups`（v2 分组形态 `data[].{depName,appList}`；空 depName 按「未分组」保留、应用不因分组字段异常丢失；缺 appId 条目跳过）/ `parse_app_items` / `parse_schedule_classify`（缺 classifyCode 跳过）/ `parse_schedule_events`（缺 id / 时间非数字跳过；分类 code 取 `typeCode` 为主、`scheduleClassifyCode` 兜底）/ `parse_schedule_day_counts`；`guess_image_mime` 按**魔数**判 MIME（PNG/JPEG/GIF/WEBP/SVG），不信任响应 Content-Type（文档库静态资源常给 `application/octet-stream`），识别不出按「无图标」降级——避免把 HTML 错误页伪装成 data URL
- **`client.rs` 三处扩展**：
  - 请求头组抽取为 `with_portal_headers`——GET / POST / 图标下载**三处同源同组**，计划 §1.1 全表只写一遍
  - 新增 `post_json`：门户同源 POST JSON，头组同 GET 另带 `Content-Type: application/json`；bs-schedule 的区间/计数接口**缺头返回 500「系统错误」（实测）**，故与 GET 共用全量头组
  - 新增 4 查询：`query_app_catalog`（分组 + 收藏两接口 + **带会话并发代拉全部图标**：附件 id 去重后 `buffer_unordered(4)`，单图标失败降级 None、不阻塞目录返回）/ `query_schedule_classify`（**会话内缓存**，实测 5 类静态数据仅首次真发请求）/ `query_schedule_events`（body 字段与官方前端逐字一致）/ `query_schedule_day_counts`（月视图角标用，本批前端未消费，能力先落协议层）
  - `app_icon_data_url`：拉 `<base>/zuul/docrepo/download?attachmentId=<UUID>` → data URL，**结果内存缓存**（`None` = 服务端确认给的不是图片，同样缓存防每次刷新重试；传输类失败不缓存、下次自动重试）；附件 id 只允许 UUID 字符集
- **图标代拉方案（真机验证成功）**：门户应用图标是**同源受保护资源**（`zuul/docrepo` 下载接口，需会话 Cookie），前端直连会因跨站 Cookie 拿到裂图 → 改为**后端带会话并发代拉字节、魔数判 MIME、base64 编码为 data URL 放进 `iconUrl`**；失败降级 `iconUrl: null` + 前端占位图标，**目录永不因图标失败阻塞**
- **URL 校验分工定案（`article.rs`）**：新增 `is_http_url`（**协议白名单**，仅 http/https），`open_app` 用它。理由：appLink 来自校方应用目录（受信来源）、后端不抓取它（无 SSRF 面）、只在系统浏览器打开——实测 30 个应用中 **16 条链接在校园域外**（万方 4、虚拟图书馆 4、一卡通 `10.3.100.110` 4、超星泛雅 2、中国知网 2），域名限制只会拦掉学校自己的合法应用；真机点「一卡通」在修前会被「仅支持校园官网链接」拦住。**`is_allowed_info_url`（域名白名单 `*.cwxu.edu.cn`）保持不变**，继续守「后端要抓取的正文 URL」（`fetch_info_detail`）与 `open_in_browser`；单测双向钉死（协议白名单放行知网/万方/`10.3.100.110` 等形态、拒绝 `file:`/`javascript:`；域名白名单回归用例确认未被放宽）
- **新增后端依赖**：`base64` 0.22（图标编码）、`futures-util` 0.3（`buffer_unordered` 限并发）
- **tauri 接线**：新增 4 命令 `get_app_catalog` / `get_schedule_classify` / `get_schedule_month` / `open_app`（命令数 **20 → 24**）；`open_app` 复用 `open_url_in_browser` 的打开方式（官方 `tauri-plugin-opener` Rust API，不手写 Win32）但校验换成协议白名单，`isCas` 为契约保留字段、当前不影响打开策略；`get_schedule_month` 对区间倒挂拒绝（前端 bug 防御，不透传服务端）
- **前端**：`AppsPanel.tsx`（常用钉选区 + 按部门分组网格 + 图标/占位 + 点击 `open_app` + 四态）；`SchedulePanel.tsx`（周视图 + 5 类彩色过滤 chips + 详情卡 + 四态；周切换显示日期区间如 `9.14 – 9.20`，今日列高亮仅当前周生效，全不选分类不发请求）；`types.ts` 同步契约 6 接口
- **验证**：`cargo test --workspace` → **90 passed / 0 failed / 3 ignored**（`campus-portal` 37，批次 3 新增 10：分组宽松形态与错误路径、收藏条目、日程分类/事件映射与跳过规则、每日计数、魔数判 MIME、协议白名单放行与拒绝、域名白名单不放宽回归）；`cargo check -p campus-hub` 通过；`tsc --noEmit` 0 错误；`vite build` 通过；**真机验收**：应用页 = 常用钉选 + 按部门分组网格（公共服务/教务处/教师发展/学工部/财务处/图书馆/保卫处/信息化中心等）+ **真实彩色图标全部正常显示** + 点「一卡通」成功在系统浏览器打开（落点统一身份认证平台，修前会被域名白名单拦住）；日程页 = 周视图 `9.14 – 9.20` + 5 类彩色过滤 + 真实课程块（10:10–11:50、13:45–15:25，与学校日程服务实测作息一致）+ 今日列高亮 + 点击块看详情
- **遗留**：① **WebVPN B 类包装未做**——`open_app` 目前一律按原链接协议直开，WebVPN 会话未打通（属 M4 范围），校外访问 B 类应用由浏览器侧自行报错；② **月视图与每日计数角标未做**——`getCountBetweenTime` 协议层就绪（解析 + 方法 + 单测），未加 IPC 命令、前端未消费；③ 计划的 3 个应用接口未实现（全量 `queryApp` / `queryClassifyList` / `queryDepLabel`）——实测 `v2/queryApp` 分组自带部门名且 appList 总数与全量接口一致，后两者是官方筛选器数据源、本 UI 无筛选器；④ 待办列表「有数据」路径仍未真机验证（该账号三栏均 0 条，沿用批次 2 遗留）

## 2026-09-18 · M2 批次 2：资讯页 + 待办页数据接线

- **模块**：`crates/campus-portal`（新增 article 模块）、`tauri-app/src-tauri`（6 新命令 + opener 插件 + CSP）、`tauri-app/frontend`（InfoPanel/TodoPanel 接真实数据 + TS 契约）、`.codewiki/`、`docs/`
- **计划**：`docs/superpowers/plans/2026-09-18-m2-portal-pages.md`（实测接口全表 §1.2、栏目 id↔名称表、冻结契约 §2.1 批次 2——本条不重复抄表）
- **`campus-portal` 新增 article 模块**（`article.rs`，官网静态页 → 安全 HTML 片段，纯函数 + 脱敏单测）：
  - `is_allowed_info_url`：正文域名白名单（scheme 仅 http/https + host 精确后缀匹配 `cwxu.edu.cn` 及子域），单测覆盖 `cwxu.edu.cn.evil.com` / `cwxu.edu.cn@evil.com` / `ftp:` / `javascript:` / query 参数藏白名单域名等绕过形态（计划红线 3，防 SSRF/钓鱼）
  - `extract_article`：博达 webplus 文章页提取（标题 `h2` 优先、`<title>` 兜底；容器 `div.v_news_content` 优先、`[id^=vsb_content]` 兜底），按**标签/属性白名单重建** HTML 片段——script/style/iframe/form 等危险标签连同子树整段剔除、`on*` 事件属性与 style/class 一律丢弃、src/href 相对地址转绝对且仅保留 http/https（图片额外放行 `data:image/`）；重建时文本/属性值重新转义（`&` 最先替换）
  - `is_auth_wall`：判定正文页被官网鉴权开门页拦截（三依据：HTTP 非 2xx / 重定向最终 URL 命中 `/system/resource/code/auth/auth.htm` / 页面 `<title>` 精确为「系统提示」——解析 title 而非子串搜索，正文偶含该词不误判）
- **`campus-portal` 扩展**：`parse.rs` 新增 4 个解析纯函数（`parse_info_columns` / `parse_info_list` / `parse_todo_tabs` / `parse_todo_list`）+ 实测 7 栏目常量 `KNOWN_COLUMNS`（订阅接口实测只返回 3 个，未订阅栏目按实测全量顺序垫底补全）+ 待办字段多候选键宽松映射 `todo_item_field`（真实字段形态未实测，见遗留）；`client.rs` 新增 5 个接口方法（资讯栏目 / 资讯列表 / 正文抓取 / 待办分栏 / 待办列表），`columnId`/`tabId` 拼接查询串前做输入校验。**正文抓取用裸 `http_client`，不带门户鉴权头**——正文页是公开静态页，JWT 只发门户同源，绝不随正文抓取泄漏到其他域名
- **新增后端依赖**：`scraper` + `ego-tree`（HTML 解析与树遍历；手写 tokenizer 不可靠，弃）
- **契约扩展 `InfoDetail { title, html: string|null, needsBrowser: boolean, url }`**（计划 §2.1 `InfoDetail{title,html}` 的兼容扩展），三分类结果：
  - 正常：`needsBrowser=false` + 清洗后 HTML，前端内嵌渲染；
  - `needsBrowser=true`（**不是错误态**）：正文受官网鉴权保护，前端显示「正文需在浏览器中查看」+「在浏览器打开原文」+「返回列表」，不显示错误/重试（站点侧拦截与网络无关，重试无效）；
  - 真错误：网络/解析异常，错误态可重试。
  - 根因（实机验证）：`content.jsp` 形态正文（通知公告 columnId 9 / 规章制度 5d2c45d23866497cb2bfe93e9f136bb2 两栏）无论带不带 Cookie/UA/Referer 都停在官网鉴权页，且无 `/info/` 替代形式（404）；其余五栏（校园要闻/校园快讯/教务处/学工处/团委）为 `/info/<栏目>/<id>.htm` 可正常抓取（jwc/xgc/tw 三站容器均为博达标准 `vsb_content*`/`v_news_content`）。完整教训见 `.codewiki/learnings/cwxu-official-site-content-extraction.md`
- **tauri 接线**：新增 6 命令 `get_info_columns` / `get_info_list` / `get_info_detail` / `get_todo_tabs` / `get_todo_list` / `open_in_browser`（命令数 **14 → 20**）；`open_in_browser` 走官方 `tauri-plugin-opener`（Rust 侧调 API，不开放前端直接 invoke 插件命令，无需额外 capability），**白名单校验在可复用 helper `open_url_in_browser` 内部第一行**（复用 `is_allowed_info_url`，与正文抓取同一事实来源，批次 3 `open_app` 复用）；`lib.rs` 注册插件；`tauri.conf.json` CSP `img-src` 增加 `https/http://*.cwxu.edu.cn`（正文官网图片显示的必要配套，域名仍限校园官网）
- **前端**：`InfoPanel.tsx`（7 栏 rail + 列表 + 内嵌正文 + 分页 + 四态；`needsBrowser` 分支无错误态；正文内 `<a>` 导航统一拦截，WebView 不随正文跳转外站；过期正文响应按 URL 比对丢弃防串台）；`TodoPanel.tsx`（三栏 rail + count 徽标 + 列表 + 空态 + 四态；接口实际返回 6 个 tab、前端按契约只展示 todo/done/apply，名称接口优先失败回落兜底）；`types.ts` 同步契约 7 接口；两页分页均按 `items.length == pageSize` 满页判断（服务端 `total`/`pageCount` 实测不可靠，不伪造页码）
- **验证**：`cargo test --workspace` → **80 passed / 0 failed / 3 ignored**（`campus-portal` 27，批次 2 新增 14：白名单绕过形态、正文提取/清洗/兜底/错误路径、auth wall 三依据判定与正常页不误判、栏目兜底/列表过滤/待办多候选键解析）；`cargo check -p campus-hub` 通过；前端 `tsc --noEmit` 0 错误；`vite build` 通过；**真机**：资讯页 7 栏 rail、通知公告与校园要闻各 10 条真实列表（与门户一致）、分页可用；点开校园要闻一条 → 应用内正文渲染（标题/段落/图片正常）；点开通知公告一条 → 「正文需在浏览器中查看」+「在浏览器打开原文」（不再报错）；待办页三栏 + 空态（该账号三栏待办数确实均为 0）
- **遗留**：待办列表「有数据」路径未真机验证（账号无数据，仅单测覆盖，多候选键映射待真机校准）；未订阅栏目的列表点击未实测；正文提取的实测覆盖 = `/info/` 三站 + auth 门识别，`content.jsp` 系栏目按设计降级（见 learnings）

## 2026-09-18 · M2 批次 1：门户数据接线基建 + 今日页真实数据

- **模块**：`crates/campus-portal/`（新）、`crates/campus-auth`（两处最小改动）、`tauri-app/src-tauri`（新命令）、`tauri-app/frontend`（TodayPanel 接真实数据 + TS 契约）、`.codewiki/`、`docs/`
- **计划**：`docs/superpowers/plans/2026-09-18-m2-portal-pages.md`（实测接口全表 §1.2、校本作息与大节语义 §1.3、冻结契约 §2.1——本条不重复抄表）
- **新 crate `campus-portal`**（协议单点第四员，与 `campus-auth` 平级、不依赖 tauri，安卓可复用）：
  - `client.rs`：`PortalClient` 内部持 `CasClient`（clone 共享 Cookie jar，不重建会话）；统一请求头 helper 按门户前端同款组注入（`Authorization` JWT 无 `Bearer` 前缀、`loginUserId`/`loginUserName`、`loginUserOrgId`、`appid: ly-upp`、`csrfTimestamp`/`csrfToken` 现算、`X-Requested-With`、`Accept`；计划约定 POST 另加 `Content-Type`，本批三接口均为 GET）；JWT 与资料存 `Arc<Mutex<Option<AuthHead>>>` **按会话内存缓存**（`AuthHead` 不派生 Debug、无日志、不落盘，guard 在 await 前 drop 不跨 await 持锁）；csrf 直接复用 `campus_auth::cas::csrf_token`，门户 base 复用 `PORTAL_PROBE` 不设第二事实来源
  - `parse.rs`：解析全部纯函数——`parse_semester_info` / `parse_wallet_summary`（钱包卡 `data.data` 是内嵌 JSON 字符串需二次 parse；`loginUrl` **结构体不定义该字段**、直接丢弃）/ `parse_week_schedule`；校本大节表 `block_time_slots()`；`course_from_cell` / `elapsed_slot_count` / `next_course` / `next_course_from_now`（跨天与周末守卫）
  - `lib.rs`：`PortalError`（`NotLogin("请先登录")` / `Http` / `Parse`）+ DTO（`SemesterInfo` / `WalletSummary` / `CourseBrief`）
- **`campus-auth` 最小改动两处**：新增 `http_client()` getter（供 campus-portal 复用同一 client/jar）；`PORTAL_PROBE` 提升为 pub（门户 base 单一事实来源）
- **tauri 接线**：新增命令 `get_portal_overview`（`commands/portal.rs`，聚合 DTO `PortalOverview{semester, wallet, nextCourse, fetchedAt}`，**子字段失败互不阻塞**、失败项为 null 前端回落空态）；`lib.rs` 注册（命令数 **13 → 14**）；`infra/state.rs` 的 `CasSession` 挂 `portal: PortalClient`，`restore_session` 与会话重建时构造（缓存生命周期 = 会话生命周期，登出即整体丢弃）
- **前端 TodayPanel 接真实数据**：四态（首次加载骨架 / 有数据 / 空 / 出错可重试，游客不取数）；钱包三卡真实数字、单项取失败回落 "—"；「下一节课」横幅无课/取失败隐藏；**不伪造数据**；`types.ts` 新增 `SemesterInfo` / `WalletSummary` / `CourseBrief` / `PortalOverview` 4 个 TS 契约接口
- **校本作息与大节语义（实测发现，M2.5 课表页同用）**：门户课表矩阵 `queryAWeekSchedule.resultsJsonArr` 为 7 行 × 10 列，**行 = 星期、10 列 = 5 大节 × 2 小节**，一门课占相邻两列（列对 (1,2)=大节1 … (9,10)=大节5），大节 = 100 分钟。校本大节时间：大节1 08:00（未实测，反推）、**大节2 10:10、大节3 13:45（实测）**、大节4 15:35（推算）、大节5 18:30（未实测）；实测锚点是学校自身日程服务 `bs-schedule` 的 `Default-class` 事件（15 条跨 4 周）。**真机修正**：早期实现按 `campus-schedule::default_time_slots()` 上游默认 13 节表把大节4 显示成 14:50（其第 7 节恰为 14:50），真机验收改为 **15:35**；`campus-schedule` 上游默认表**故意未改**（被金标测试钉住，校本化留待 M2.5），校本口径收敛在 `campus-portal::block_time_slots()`
- **安全纪律**：网关 JWT 与邮箱 `loginUrl`（内含 authkey，等同凭据）只在内存中使用——缓存不落盘、`AuthHead` 无 Debug 派生、全程无日志输出；钱包卡解析对 `loginUrl` 以「结构体不定义该字段」方式直接丢弃，绝不进日志/文档/前端
- **验证**：`cargo test --workspace` → **66 passed / 0 failed / 3 ignored**（其中 `campus-portal` 新增 13 个，含回归用例 `next_course_maps_column_pair_to_block_start` 钉「列对→大节起始时刻」口径）；前端 `tsc --noEmit` 0 错误；`vite build` 通过；**真机**：今日页一卡通余额 **102.51**、未读邮件 **1**、在借图书 **8 本**（与门户首页三卡逐项一致），「下一节课 **15:35** · 信息安全 · C5科教中心313」
- **遗留**：大节 1/4/5 时间未实测（M2.5 校本化作息时校准）；日程服务课表数据不完整（缺某门课），课表以门户矩阵为准、日程服务仅用于锚定作息（教训已入 `.codewiki/learnings/`）；慧新E校实时直连按计划 §1.4 列为批次 4（可选，待用户拍板）

## 2026-09-18 · 头像裁切器 + 上传回学校（执行上一轮延后项）

- **模块**：`tauri-app/frontend`（`AvatarDialog` 重写、`authStore`、依赖）、`tauri-app/src-tauri`（头像命令与体积守卫）、`crates/campus-auth`（门户上传协议）、`.codewiki/`、`docs/`
- **起因**：执行上一轮明确延后的两项（react-easy-crop 拖拽缩放裁切器、头像上传回学校）；用户选定「两项都做」并追加三条约束——200KB 限制下尽量保留高分辨率、增加「只改本机不上传」选项、本机头像上限可放宽
- **门户上传协议（实测取证，两次真机上传）**：`POST /api/authc/users/portraitChange`；JWT 放 `Authorization`（**无 `Bearer` 前缀**，值取 `tryLoginUserInfo.data.tokenId`）+ `loginUserId`/`loginUserName`/`loginUserOrgId` + `X-Requested-With`；`csrfToken = md5("timestamp=<毫秒>,key=<门户前端常量>")`（小写十六进制）；成功判据 `meta.success`，失败保留服务端 message；**服务端按上传内容原样存储**（不再压缩），故 200KB 内的分辨率取舍全部由客户端决定；官方原图 `headPortrait` 实测仅 123×123
- **`crates/campus-auth`**：新增 `csrf_token()`（+ 内部 `to_hex`）、`portal_change_portrait()`（现拉媒体资料取 JWT/ids → 现算 csrf → 上传 → 校验 `meta.success`）、`parse_portrait_change_response()`；补 `md-5` 依赖；新增 csrf 金标向量与 portrait 解析/失败回显等单测（密钥常量只存源码，文档与 wiki 不写明文值）
- **`tauri-app/src-tauri`**：新增命令 `upload_official_avatar`（未登录→「请先登录」；`validate_official_data_url` 守卫 `data:image/` 前缀与 200KB 上限，超限文案「学校头像上限 200KB，请调小尺寸或质量」；成功后回读 `getLoginInfo` 落盘最新官方头像并返回）；`AVATAR_MAX_B64` 512KB→**2MB**（文案「本机头像过大（上限 2MB）」）；日志只打码用户名，**绝不记录 data URL**；命令数 12→13
- **`AvatarDialog` 重写（两条保存路径）**：`react-easy-crop@^6.2.3` 接入 1:1 裁切（拖拽取景 + 滚轮/滑杆缩放 + 圆形/方形切换 + 重置/换一张）
  - **仅保存到本机**：边长 `min(1024, 裁切边长)` 不放大，有透明通道出 PNG、否则 JPEG q0.92，PNG 超 2MB 回退白底 JPEG；上限放宽到 2MB
  - **保存并上传学校**：尺寸阶梯 `[1024…320]` × 质量阶梯 `[0.95…0.6]` 取第一个 ≤200KB 组合，源分辨率不足时 clamp 不放大（推高分辨率即用户第一条约束的实现方式）；成功后同时写本机并提示「已上传学校 · WxH · N KB」
  - 两条路径的体积预估与落盘**同源同函数**（250ms 防抖重编码），界面数字与实际字节一致
- **缺陷修复（P0，打开弹窗即白屏）**：裁切回调 `useCallback` 原先写在 `if (!open) return null;` 之后，`open` 变化导致 Hook 数量变化，React 19 直接卸载根节点（本仓无 error boundary）；已把 Hook 提到提前 return 之前。另补 `react-easy-crop/react-easy-crop.css` 导入（缺它裁切器无样式）
- **验证**：`campus-auth` 单测 **26 passed / 3 ignored / 0 failed**（含 `csrf_token_golden_vector`）；`tsc --noEmit` 0 错误；`vite build` 通过；**真机双路径实测**——本机保存后 `profile.json.localBase64` = 141385 字节 PNG 900×900（与界面预估「约 139 KB」吻合）、右上角与首页头像同步更新；上传学校后 `getLoginInfo.headPortrait` 由 PNG 123×123 / 28657 字节 → JPEG 123×123 / 4852 字节、`officialFetchedAt` 刷新，证明「会话→JWT/ids→csrf→上传→回读」整链路真实生效（本轮以「用户当前学校头像重新上传」作为测试载荷，视觉效果不变）
- **数据与隐私**：本轮验收截图**不入库**（画面含账号真实肖像，避免写入 git 历史），临时文件已移出仓库至 `%TEMP%/campushub-avatar-verify/`；⚠️ 另需注意上一轮已入库的 `docs/verify/ui2-*.png` 中同样含该肖像，是否清理由用户决定
- **未做**：未登录守卫（「请先登录」分支）未做真机演练（需登出 + 真实验证码重登，成本高于收益；逻辑为单行状态判断）；M2 门户数据接线仍按上一轮结论待用户确认后再启动

## 2026-09-18 · 外壳视觉重设计 + 游客模式 + 右上角账号系统与头像

- **模块**：`tauri-app/frontend`（设计系统/壳/8 面板/账号与头像 UI）、`tauri-app/src-tauri`（头像与账号命令、登录显示名）、`crates/campus-auth`（门户资料接口）、`docs/`（计划/设计文档/验收截图）
- **起因**：用户反馈三条——前端观感廉价、未登录被登录页锁死主界面、账号无头像；并要求「登录收进右上角账号系统」+ 复查融合门户找遗漏
- **门户复查（真实账号实机）**：顶栏账号菜单=我的账号/上传头像/退出（官方上传头像仍为裸文件框+三行红字，无裁切压缩）；**官方头像可经 `GET /api/upp/userControl/getLoginInfo` → `data.headPortrait` 取回**（base64 PNG，~38KB）；真实姓名经 `POST /tryLoginUserInfo` → `data.userName`/`departmentName`；铃铛=四类消息（系统/办事/资讯/日程）；门户无游客态；首页接口全景（`getPageContent`/`querySimpleInfoCenter`/`querySimpleFlowItems`/`queryAWeekSchedule`/`querySemesterInfo`/`queryBriefMessage`/`queryMyStore` 等）实测 URL 已记入设计文档附录 C（M2 直接复用）
- **游客模式**：`App.tsx` 去掉登录门禁，壳与各面板恒可进入；启动 `check_session` 非阻塞探测；登录入口收敛到账号菜单 / 今日页引导条 / 各页空态按钮，均调同一登录弹层（四态状态机 + CAPTCHA_MANUAL 手动兜底 + 已保存账号免密）
- **右上角账号系统**：`AccountMenu`（未登录：登录 + 已保存账号免密/删除 + 外观 + 设置；已登录：身份头 + 上传头像 / 同步学校头像 / 切换账号 / 外观 / 设置 / 退出）；`LoginDialog`（由 LoginPanel 迁移，全屏页删除）；`AvatarDialog`（拖拽/选择 → Canvas 中心裁方 → 256×256 → JPEG q0.9/PNG，显示「原 X → Y」，512KB 上限）
- **头像三态**：本地 > 官方 > 首字默认；登录后自动同步一次学校头像；游客态不展示账号头像（回落「锡」占位）
- **后端**：新增 5 条命令 `get_avatar` / `set_avatar` / `clear_avatar` / `sync_official_avatar`（无会话→「请先登录」）/ `remove_account`；头像落盘 `%APPDATA%/campushub/profile.json`（明文 base64，非凭据，≤512KB）；`list_accounts` 增 `displayName`；`campus-auth` 新增 `portal_user_profile()` + 纯函数 `extract_user_profile()`（含 4 个离线单测）；`finish_login` 取真实姓名落库，失败回退学号且不抹旧值
- **视觉执行层**（域色 token 与底部 Dock 签名保留不变）：新增字号阶（28/22/18/14/12）、圆角阶（14/10/8）、**域色染色阴影**、`surface-2`/`line-strong`、`:focus-visible` 统一品牌色 outline、`prefers-reduced-motion` 降级、body 域色径向氛围底；新建 `PanelHeader`/`EmptyState`/`Avatar`/`Surface` 共享组件；8 面板重做（今日页头像问候 + 登录引导条 + 5/4/3 非对称钱包卡 + 禁用态快捷动作；其余页「数据接入中」职业化空态，不再裸写"建设中"）
- **根因修复（P0）**：shadcn 语义 token（`--primary` 等）此前只写在 `:root`、未进 Tailwind v4 `@theme`，`bg-primary`/`text-primary-foreground`/`bg-card`/`border-border` 等工具类**从未生成**（主按钮长期渲染为裸文字）；改用 `@theme inline` 映射到域色 token，产物 CSS 已见 `.bg-primary{background-color:var(--color-brand)}`
- **验证**：`cargo test --workspace` → **47 passed / 3 ignored / 0 failed**（基线 35 → 43 → 47）；`tsc --noEmit` → 0 错误；`vite build` 通过；**真机 `tauri dev` 全流程实测**（补上交接报告欠账）：冷启动落游客态主界面 → 账号菜单 → 已保存账号免密登录 → 学校头像自动同步 + 门户真实姓名落库 → 退出登录回落游客态；验收截图 `docs/verify/ui2-*.png` 8 张（游客今日/游客菜单/游客待办/登录弹窗/头像弹窗/登录态今日/登录态菜单/登录态待办）
- **延后**：react-easy-crop 拖拽缩放裁切器、头像上传回学校（需授权）、教学周与真实数据接线（M2/M2.5/M5）

## 2026-09-18 · 交接报告 HANDOFF.md（面向 M2 接手方）

- **模块**：文档（无代码行为改动）：新增 `docs/HANDOFF.md`；`crates/campus-auth/tests/captcha_solve.rs` 注释同步；`.codewiki/` 索引与基线
- **摘要**：为下一阶段（M2 门户各页 / M2.5 课表接线与 UI）接手方补写交接报告，含：
  - 现状坐标：M0/M1 已交付验收、`campus-schedule` 算法内核就绪（12 测试）、M2 未动工（除登录面板外 8 个面板为占位）
  - 仓库与协作现状：无远端、本地 ff 合并（不走 git-merge-push.sh）、`git add` 明确路径、凭据与验证码红线
  - 代码地图（三 crate + tauri 接线 + 前端面板状态表）与分层约定（协议在 crates、IPC 契约 `CommandResult` 三态）
  - 已打通链路可复用事实：CAS 全流程（含手动跟随重定向原因）、验证码方案与三分类指标、正方课表数据源（jwgl 端点/`oldzc` 位掩码）、慧新E校 `synAccessSource=app`
  - 运行命令（全量测试/前端构建/tauri dev 与打包/live 测试/验证码样本重建与评测）、铁律 5 条、已知坑索引（指向 `.codewiki/learnings/`）
  - 下一步切入点（建议先补门户业务接口侦察 + 建 `crates/campus-portal`）、M2.5 剩余项（含 `PanelId` 追加 `timetable` 需同步的三处）、未决风险表
- **附带修正**：`captcha_solve.rs` 两处注释仍写"类内取均值"，而实现早已改为"每类保留最多 `PER_CLASS_LIMIT`(=10) 张样本补丁"（均值会使同类 NCC 落到阈值边缘、正确率实测跌至 54%），注释已同步；CodeWiki `cw index` + `cw meta update` 推进基线至 `4b58765`
- **验证**：`cargo test --workspace` → 35 passed / 3 ignored / 0 failed；`tauri-app/frontend` `npm run build` → 通过（vite 6.4.3，JS 537.91 kB）；`cw status` → up to date
- **遗留**：桌面窗口内用真实账号点一次登录尚未人工确认（live 测试只覆盖 Rust 协议层），已写入交接报告「未决与风险」首条

## 2026-09-17 · M0+M1 交付：脚手架 + CAS 登录闭环（含验证码自动识别，真实账号端到端打通）

- **模块**：`crates/campus-auth/`（新）、`tauri-app/{frontend,src-tauri}`（新）、根 workspace、CodeWiki
- **计划与评审**：`docs/superpowers/plans/2026-09-17-m0-m1-foundation.md`（12 任务）——先经 deepseek-flash 独立评审（5 P0/10 P1/14 P2 全部消化，含 golden 值实测固化、reqwest CookieStore 读回限制、shadcn CLI 行为、beforeDevCommand cwd 等）
- **M0 脚手架**：
  - 根 Cargo workspace（campus-schedule + campus-auth + tauri-app/src-tauri）
  - 前端：Vite 6 + React 19 + TS strict + Tailwind v4；域色 token 系统（10 色 + shadcn 语义映射 + 深色提亮档 + Outfit 数字字体）；`tauriApi.ts` 唯一 IPC 出口 + `CommandResult` 三态契约；AppShell 顶条 + **域色悬浮 Dock 导航**（8 项、framer-motion 域色胶囊/指示条、gsap 磁吸、reduced-motion 降级）；8 面板骨架；shadcn button/input/card/tooltip 源码入库
  - `src-tauri`：AppState（tokio Mutex 锁纪律）+ DPAPI 裸 FFI 账号加密库 + session.json 会话持久化 + Tauri 2 配置（capabilities 最小集、CSP img-src data:）
  - **CodeWiki 初始化**（7 篇架构/模块/概念/决策文章 + 3 篇踩坑记录）
- **M1 登录闭环**：
  - `crates/campus-auth`：textbook RSA（golden 对拍线上 JS，2 组固化向量）、CAS 客户端（kaptcha/login/16 错误码映射/**手动跟随 302 链**/portal_probe）、**RecordingJar**（自实现 CookieStore 记录会话，弥补 reqwest 0.12 内部 Jar 不可读回）
  - **算术验证码自动识别 100%**：颜色不变强度图（`765-Σrgb` 按峰值归一化）+ bbox 锚定 16×14 画布 + NCC(0.88/0.03) + ±1px 位移补偿；**模板集 70/70、holdout 30/30、自信错误 0、拒绝 0**（三分类评测口径）
  - 7 条 Tauri 命令（login/login_manual/login_saved/get_captcha/check_session/logout/list_accounts）+ 重试状态机（仅 WrongCaptcha 重试 ≤3、NOUSER 绝不自动重试、识别失败不消耗 CAS 错误计数）
  - 登录页（四态状态机 + CAPTCHA_MANUAL 手动兜底 + 已存账号免密重登）+ 今日页骨架
- **验证（均为实际输出）**：`cargo test --workspace` 全绿（campus-schedule 12 + campus-auth 12 + campus-hub 11）；clippy 新增代码零告警；`npm run build` 通过；**真实账号 live 测试通过**（`cargo test -p campus-auth -- --ignored cas_live`：自动识别 → CAS 签发 ST/TGT → SSO 回跳 → 门户三 cookie → 会话 Alive）；`npm run tauri dev` 起真实窗口渲染登录页（截图 `docs/verify/`）；浏览器验收 Dock 域色胶囊/深浅主题/面板切换（截图 `docs/verify/`）
- **踩坑记录（写进 CodeWiki learnings）**：验证码识别五个独立根因（彩色灰度丢形/片序错/细笔画空网格/平局误杀/漏乘法分支）、CAS SSO 三坑（重定向中断/明文落点断连/探测误判）、批量图片标注不可靠（改结构化拼图 + 客观特征交叉校验）
- **工程修正**：新增 `tauri-app/package.json`（承载 Tauri CLI——CLI 只从 cwd 及子目录发现 src-tauri）；`.gitignore` 补 dist/ 与样本/拼图 PNG
- **未做（后续里程碑）**：M2 门户各页、M2.5 课表页与调课通知引擎、M3 一卡通/电费、M4 WebVPN 路由、M5 通知中心

## 2026-09-17 · 导航改版：悬浮 Dock 标签栏（用户指定，对齐 CampusLogin）

- **模块**：设计文档 §4/§6/§7
- **摘要**：用户指定页面切换采用 Wxxy-CampusLogin 同款悬浮标签栏。分身取证其 `DockNav.tsx` 实现（fixed bottom-5 玻璃 Dock、纯图标+hover tooltip、framer-motion 弹簧胶囊与圆点指示条、gsap 磁吸放大 scale 1.35、zustand activePanel 切换 + AnimatePresence 过渡 + useDeferredValue、内容区 pb-28 预留），设计文档已按本项目语境改写：
  - 顶栏导航废除，改为底部悬浮 Dock（8 项：今日/资讯/待办/日程/应用/钱包/电费/设置）；顶条仅剩 ⌘K/通知/主题/账号
  - **域色创新**：激活胶囊/指示条/图标用该项域色（非统一色），域色编码系统从内容延伸到导航
  - 依赖增量：framer-motion + gsap（磁吸可裁剪）；签名元素由「域色 spine」更新为「域色 Dock」
- **验证**：取证基于 CampusLogin 源码逐文件核查（组件/样式/交互/依赖均文件:行号级证据）

## 2026-09-17 · 课表核心 Rust 移植（campus-schedule crate，M2.5 首个交付）

- **模块**：Rust 协议核心（`crates/campus-schedule/`，纯逻辑无 Tauri 依赖，为安卓 path 依赖预留）
- **摘要**：按用户指定将 shiguangschedule（拾光课程表，Apache-2.0）的算法与数据模型**移植到 Rust**（原计划 TypeScript，用户改定 Rust）：
  - `model.rs`：Course/CourseTableConfig/TimeSlot（serde camelCase 对齐前端 IPC）+ **CourseOverride 调课叠加模型（自建，上游缺口）**+ CourseSource 导入/手动源隔离 + `expand_week_mask` 正方周次位掩码展开
  - `weeks.rs`：周次计算三函数（Kotlin AppSettingsRepository.kt:138-224 → Rust/chrono），支持自定义周起始日
  - `grid.rs`：time_to_grid_scale/grid_scale_to_time/merge_courses（Kotlin WeeklyScheduleViewModel.kt:324-754 → Rust），重叠分簇+贪心分列
  - `timeslots.rs`：默认 13 节作息常量
  - `zhengfang.rs`：正方课表响应解析器（本项目原创），坏数据跳过、课程名稳定配色
- **验证**：cargo test **12/12 通过**——golden 锚定真实教务数据（开学日 2026-09-07 → 2026-09-17=第 2 周与门户一致）、位掩码 4095→1-12 周、分列（单列/两列/链式复用/非本周淡化）、正方样例解析
- **合规（Apache-2.0）**：`crates/campus-schedule/NOTICE.md`（上游逐文件映射+修改说明）、上游 LICENSE 副本（LICENSE-shiguangschedule.txt）、根 `THIRD-PARTY-NOTICES.md`、被移植文件头「Adapted in part from … Modified」标注；不使用上游名号
- **上游参考仓库**：完整克隆至工作区 `../shiguangschedule`（独立仓库，不进本项目 git）
- **待办（M2.5 后续）**：调课通知 L1 规则解析引擎、自动更新 diff、教师课表端点、ICS 导出、Tauri 命令层接线

## 2026-09-17 · 首页 v2 定稿 + 课表功能立项（M2.5，用户拍板）

- **模块**：PLAN.md / 设计文档（§4/§5/§5.1/§11）
- **首页 v2（用户指定精简）**：去掉资讯/快捷应用/待办/会议/信息服务/课表卡片六模块，只留问候+钱包三卡+下一节课横幅+固定快捷动作；各模块独立成页靠 M5 通知触达
- **课表立项 M2.5（推翻原"暂不做"）**，五条需求逐条落计划：
  - 教务自动导入（正方接口 `xskbcx_cxXsKb.html` 2026-09-17 实测通过，含周次位掩码解析）
  - 调课通知自动调整（L1 规则提取 + L2 置信分级自动/待确认，override 模型自建、可按通知 id 撤销）
  - 自动对比更新（快照 diff，匹配键课程名+jxb_id，仅动 import 课程）
  - 导入课程 source 标签隔离（手动课程零触碰）
  - 算法/数据模型移植 shiguangschedule（分身评估：Kotlin/Compose 不可直接复用，移植周次计算/分列算法/weeks 显式列表，Apache-2.0 合规清单）
- **侦察新增**：PLAN.md §3.4 正方教务系统（SSO 入口/课表接口/响应字段）；慧新E校 sessionStorage 缺陷与两源余额偏差入侦察结论
- **风险表新增**：调课通知自然语言解析风险（上线初期默认全人工确认观察期）、教师课表接口待侦察、Apache-2.0 合规
- **验证**：课表接口真实登录态实测（8 门课全字段）；shiguangschedule 评估基于 GitHub 仓库文件树+源码逐文件核查

## 2026-09-17 · 走查遗漏补测（设计文档附录 B，8 项）

- **模块**：设计（`docs/design/frontend-design.md` 附录 B）
- **摘要**：复查发现 8 处未覆盖项并全部补测：资讯正文（新开标签跳官网静态页 → 内嵌阅读优化点坐实）、会议详情页（主持人/参会人员字段）、订阅管理（60 栏目池+拖拽排序）、顶栏全局搜索（检索中心+热搜榜）、应用详情页（确认不存在）、CAS 自助服务三 tab（改密走官方，客户端外链）、慧新E校「我的」页 + 退出流程（单点登出）
- **新痛点**：慧新E校 token 存 sessionStorage 不跨标签页，新开标签必掉登录（客户端后端持 token 规避）；一卡通余额门户 28.01 vs 慧新E校 17.51 两源不同步（客户端以实时源为准）
- **额外情报**：热搜榜暴露未上架的 OA 系统（v1 不覆盖）
- **验证**：全部基于真实登录态浏览器走查

## 2026-09-17 · 应用中心 30 应用逐站 SSO 实测（设计文档附录 A）

- **模块**：设计（`docs/design/frontend-design.md` 附录 A）
- **摘要**：补测应用中心全部 30 个应用（上轮仅测了门户自身页面）：从 `/api/upp/appStore/v2/queryApp` 接口拉取全量清单（含 appLink/isCas 元数据），浏览器逐站访问记录最终落点：
  - **A 类 CAS 直达可用 ~17 个**：教务系统（正方）、创新创业、超星泛雅、whall gemini 表单组（心理预约/请假/贷款/监控/报告厅）、办事大厅、yd.cwxu.edu.cn 表单组（邮箱/报修/漏洞单）、一卡通 SSO 桥、校园一键通、电子资源、万方等
  - **B 类需 WebVPN 会话 7 个**：教学质量保障/财务系统/知网镜像/IEEE/ScienceDirect/SCIE/图书馆空间管理（域名仅经深澜网关可达，M4 打通后自动可用）
  - **C 类异常 4+**：毕业论文系统标记 cas 实则 SSO 断；联创文印/馆藏数字化死链；两个学生表单教师账号 403（权限问题）
  - **关键结论**：官方 `isCas` 字段不可信，客户端需自建 `appAccess.json` 可达性元数据与 A/B/C 打开策略引擎；顺带采集门户 M2 全部数据接口清单（应用/资讯/待办/课表/会议/邮箱卡/消息）
- **验证**：逐站真实访问（ticket 签发链路+落点判定），无凭据操作

## 2026-09-17 · 门户实机走查 + Windows 前端设计文档（M2 前置）

- **模块**：设计（`docs/design/frontend-design.md`）
- **摘要**：真实账号浏览器走查官方门户全部页面（首页/应用中心/待办中心/资讯中心/日程中心/消息中心/个人菜单/上传头像）+ SSO 桥进慧新E校，产出完整设计文档：
  - **痛点清单 10 项分级**（P0：头像上传零辅助/账号管理割裂/重置密码明文进消息流；P1：信息墙无重点/无推送/资讯扫描性差等）
  - **功能重设计映射表**：官方 16 项功能 → 锡院助手设计（含头像上传三步弹层：1:1 裁切 + Canvas 压缩 ≤200KB 默认 jpg）
  - **信息架构**：7 项顶栏导航 + Ctrl+K 命令面板 + 托盘常驻；「今日」页线框
  - **视觉 token 初稿**：锡院紫品牌锚 + 域色编码系统（钱包绿/资讯紫/待办琥珀/日程湖蓝）+「域色 spine」签名元素；Segoe UI + Outfit 数字字体
  - **选型（分身 GitHub 实测）**：Tailwind v4 + shadcn/ui + lucide-react（124k），规避 AntD/Arco/Semi；裁切 react-easy-crop + Canvas 压缩（仅 1 新依赖）；布局对标 Spacedrive/Cap（Tauri+React 同款栈）
- **验证**：走查基于真实登录态（截图+DOM 快照取证）；选型数据来自 GitHub REST API 实测（star/pushed_at，2026-09-17）
- **隐私**：走查截图含个人信息，一律不入库（仅文档文字记录）

## 2026-09-17 · CAS 真实账号端到端登录 + WebVPN 联动登录验证（M1/M4 侦察）

- **模块**：CAS 登录协议（`docs/cas-recon/`）
- **摘要**：在协议逆向基础上完成真实账号端到端验证：
  - **CAS 真实登录成功**：`POST /v1/tickets` 响应顶层即 `{"tgt":"TGT-...","ticket":"ST-..."}`（无 data 包裹、无 Set-Cookie——CASTGC 由前端 JS 写入，客户端可忽略）
  - **门户 SSO 成功**：shiro-cas 验票 302 后种下 `customsid`（Shiro 会话）/`Authorization`（门户 API 令牌）/`rememberMe`；302 目标为 http:// 明文，客户端应替换 https
  - **WebVPN（深澜 Srun）联动登录成功**：CAS `service=https://webvpn.cwxu.edu.cn/login?cas_login=true` → 初始 `wengine_vpn_ticket` → ticket 回跳 → `wengine-vpn-token-login` 一次性 token → WebVPN 会话建立（首页复查不再跳 /login）
  - 深澜代理 URL 活样本已采集（`/https/77726476706e69737468656265737421<加密hex>/...`，两主机样本入库），为 M4 URL 加密逆向铺路
- **交付物**：`cas.js`（CAS 公共库）、`webvpn.js`（WebVPN 探针）、probe.js 增强（--creds 凭据文件读取、SSO 重定向链验证）、REPORT.md 补真实登录与 WebVPN 章节
- **安全**：凭据经文件读取（`--creds`），命令行/日志/响应均不落明文；`账号与密码.txt` 已加入 .gitignore
- **验证**：真实账号两次探针全部通过（门户会话 Cookie + WebVPN 会话 Cookie 判定）
- **PLAN.md**：侦察结论补 ⑥⑦ 两条（真实登录、WebVPN 联动）；M4 深澜素材就绪

## 2026-09-17 · CAS 统一身份认证登录协议逆向与实测（M1 侦察）

- **模块**：CAS 登录协议（`docs/cas-recon/`）
- **摘要**：逆向 CAS 前端 JS（app.2fb1f8a1ec5d2342de95.js），完整还原账密登录协议并实测打通：
  - 流程：`GET /lyuapServer/kaptcha`（算术题验证码，uid+PNG）→ `POST /lyuapServer/v1/tickets`（username / password=RSA密文 / service / id=验证码uid / code=答案，头 `token=RSA("lyasp"+时间戳)`）→ 响应直含 ST/TGT → `service?ticket=ST` 完成门户 SSO
  - 密码加密：textbook RSA 1024 位（e=010001，little-endian 组块 126 字节，无 padding，hex 不补零）；验证码形态定案为算术题（两一位数 +/-/*，可纯本地识别）
  - 实测：假账号+正确验证码 → `NOUSER`（验证码/加密/字段全部通过），错误验证码 → `CODEFALSE`（对照）；全程无 Cookie 依赖
- **交付物**：`docs/cas-recon/REPORT.md`（协议报告）、`probe.js`（端到端探针）、`rsa30.js`（线上 RSA 模块原样提取，Rust 实现对照基准）、`extract-rsa.js`（提取脚本）
- **验证**：node probe.js 实测（HTTP 200 + 响应 JSON 判定）；RSA 对照测试 3/3 一致
- **PLAN.md**：侦察结论 CAS 段重写为已定案；风险表验证码形态结项；M1 勾选侦察项
- **待办**：真实账号跑一次 probe（用户侧执行），随后进入 M1 Rust 协议核心实现
- 备注：CodeWiki 未初始化（尚无源码，M0 脚手架时 `cw init`）
