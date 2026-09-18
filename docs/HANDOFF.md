# 锡院助手 Wxxy-CampusHub · 交接报告

> 交接时间：2026-09-18 · 交接点：`master` @ `4b58765`（共 21 个提交）
> 上一阶段执行方：会话 `sess-b27a282c`（本会话在其分支 `docs/sess-b27a282c-handoff` 上补写本报告）
> 读者：接手 M2 及之后的会话 / 协作者（人或 AI）

---

## 〇、2026-09-18 补记（会话 `sess-c2ed9b35`，交接点后已并入 master）

本轮做的是**壳层与服务形态**，不是 M2 业务页，接手 M2 前请先读这四条：

1. **GUI 真机登录欠账已清**：`tauri dev` 用真实账号跑通「冷启动游客态 → 账号菜单 → 已保存账号免密登录 → 学校头像同步 + 门户真实姓名落库 → 退出回落游客态」（正文第九节首条风险关闭）。
2. **开场形态变了**：登录不再是门禁——`App.tsx` 恒渲染壳，`check_session` 非阻塞；登录入口在右上角账号胶囊 / 今日页引导条 / 各页空态按钮，统一弹层（`components/LoginDialog.tsx`；原 `panels/LoginPanel.tsx` 已删除）。登录态读 `stores/authStore.ts`（`status: unknown|guest|authed`），头像三态 本地>官方>首字（游客不展示账号头像）。
3. **命令面 7 → 12 条**：新增 `get_avatar`/`set_avatar`/`clear_avatar`/`sync_official_avatar`/`remove_account`；`list_accounts` 增 `displayName`；`campus-auth` 新增门户资料接口（`portal_login_info` 取官方头像、`portal_user_profile` 取真实姓名/院系）。
4. **M2 直接可用**：`docs/design/frontend-design.md` **附录 C** 记了本轮门户实机复查结论与**首页接口全景**（资讯/待办/课表/学期/消息/应用商店等实测 URL）；8 个面板已重做成「游客空态 + 数据接入中」两态骨架，接数据即用。

> 正文（一至十节）仍是 M0/M1/M2.5 的准确描述，除「第九节首条风险」与「面板/命令面计数」外无需更正。

---

## 一、一句话现状

**M0（脚手架）与 M1（CAS 登录 + 账号管理）已交付并验收完毕**，真实账号 CAS 端到端登录已跑通（CAS 签发 TGT+ST → 手动跟随 SSO 302 链 → 门户种下会话 Cookie → `portal_probe() == Alive`）；**M2.5 的算法内核（`campus-schedule` crate）也已移植完成**（12 个单元测试全过），但课表 UI 与教务导入接线尚未开始。M2（门户各业务页）**尚未动工**——目前除登录面板外的 8 个面板全是「建设中」占位。

验证基线（2026-09-18 复跑）：`cargo test --workspace` → **35 passed / 3 ignored / 0 failed**；`tauri-app/frontend` `npm run build` → 通过（vite 6.4.3，4.5s，JS 产物 537.91 kB，仅有 chunk >500kB 的性能提示，非错误）。

---

## 二、仓库与协作现状

| 项 | 现状 |
|---|---|
| 分支 | `master` 为唯一长期分支；**无 git 远端**，合并走本地 `git merge --ff-only`，**不要**用 `~/.zcode/scripts/git-merge-push.sh`（它按远端流程设计） |
| 提交纪律 | 主题分支 `feat/` `fix/` `docs/sess-<短ID>-<主题>`；`git add <明确路径>`，**禁止 `git add -A`**——仓库根有用户自己的未跟踪脚本 `一键填分4.user.js`，不能误带 |
| 敏感信息 | 根目录凭据文件（`.gitignore` 已忽略）只作人工验证输入；live 测试从 `CAMPUS_HUB_CREDS` 环境变量指向的文件读凭据，**代码/日志/commit message/派单 prompt 一律不得出现明文** |
| CodeWiki | `.codewiki/`（10 篇）随仓库进 git。会话开始跑 `cw status`，收尾 `cw index` + `cw meta update`，wiki 改动与代码改动**同一次提交** |
| 与参考项目关系 | 架构模板 `../Wxxy-CampusLogin`（协议单点 + 平台外壳）；课表算法上游 `../shiguangschedule`（Apache-2.0，已按合规三件套标注） |

---

## 三、代码地图

```
Cargo.toml                       workspace：三个成员 + [profile.release] lto="thin"
crates/campus-auth/              CAS/SSO 协议核心（无 Tauri 依赖，可被安卓 path 依赖）
  src/cas.rs        (356行)      CasClient：kaptcha→login→sso_follow→portal_probe
  src/captcha.rs    (269行)      算术验证码识别（强度图 + 投影切分 + NCC）
  src/rsa.rs         (60行)      textbook RSA 加密 + token 头
  src/jar.rs        (103行)      RecordingJar：可读回会话的 CookieStore
  src/error.rs       (12行)      CampusAuthError
  templates/kaptcha-templates.json  116 条模板，include_str! 编译期内嵌（331 KB）
  tests/                         4 个集成测试文件（captcha_solve / cas_live / cas_parse / rsa_golden）
crates/campus-schedule/          课表算法内核（M2.5，纯逻辑）
  src/model.rs      (152行)      Course/CourseTableConfig/TimeSlot/CourseOverride + 周次位掩码展开
  src/weeks.rs      (132行)      开学日 ↔ 周次互算（支持自定义周起始日）
  src/grid.rs       (400行)      重叠课程分簇 + 贪心分列 + 时间↔网格换算
  src/timeslots.rs   (33行)      默认 13 节作息常量
  src/zhengfang.rs  (199行)      正方教务课表响应解析（本校原创）
tauri-app/
  package.json                    Tauri CLI 宿主（CLI 只从 cwd 及子目录发现 src-tauri，必须在此启动）
  src-tauri/src/commands/auth.rs (546行)  7 条 Tauri 命令 + 重试状态机 + DTO 契约
  src-tauri/src/account/         DPAPI 加解密（裸 FFI，零新依赖）+ accounts.json 多账号存储
  src-tauri/src/infra/state.rs   (152行)  AppState + session.json 会话持久化
  frontend/src/panels/           9 个面板：LoginPanel（已接真实命令）+ TodayPanel（半静态）+ 7 个占位
  frontend/src/components/       AppShell（顶栏 + 面板路由）+ DockNav（底部悬浮 8 项 Dock）
  frontend/src/shared/tauriApi.ts  IPC 唯一出口，返回 CommandResult 三态
docs/                            PLAN.md（根）/ CHANGELOG.md（根）/ design / cas-recon / verify / superpowers/plans
scripts/                         captcha-collect.mjs（样本采集）+ captcha-samples（100 张 PNG 已在 .gitignore）
```

**分层约定**：协议与算法全在 `crates/`（无平台依赖）→ `tauri-app/src-tauri` 只做 IPC 接线与存储 → 前端经 `invokeCommand` 单一出口调命令，命令一律返回 `CommandResult { success, message?, data? }`（camelCase）。新增业务页时按此三层落位。

---

## 四、已打通的链路（可直接复用的事实）

### 1. CAS 登录（`crates/campus-auth/src/cas.rs`）

- 验证码：`GET /lyuapServer/kaptcha` → `{uid, content(base64 PNG), kaptchaType}`，题面为**算术题**「一位数 运算符 一位数 =」
- 登录：`POST /lyuapServer/v1/tickets`（x-www-form-urlencoded，7 字段），头 `token = RSA("lyasp" + 毫秒时间戳)`；成功响应顶层即 `{tgt, ticket}`
- 密码：textbook RSA，1024 位 modulus 硬编码在 `rsa.rs:16`（`CAS_RSA_N_HEX`），e=`0x010001`，126 字节组块、little-endian、hex 不补前导零；golden 对拍 `docs/cas-recon/rsa30.js` 由 `tests/rsa_golden.rs` 固化
- SSO 回跳：**必须手动跟随重定向**。reqwest 默认策略遇到中间跳 body 中断（hyper `IncompleteMessage`）会整链失败，所以 `CasClient` 持有两个 client（`http` 自动重定向 + `http_manual` 手动循环），共享同一个 `RecordingJar`；循环内做 http→https 升级（只升不降）
- 会话判定：`portal_probe()` = jar 有 `customsid` 且首页未被弹回 CAS 域；三 cookie（`customsid`/`rememberMe`/`Authorization`）由 `RecordingJar` 捕获，可 `snapshot()`/`restore()` 持久化（`docs/cas-recon/REPORT.md` 有完整实测记录）

### 2. 验证码识别（零外部依赖，**红线：绝不外发第三方打码服务**）

方案：彩色字符 → 颜色不变强度图（`765 - Σrgb` 按峰值归一化）→ 垂直投影切分（必须恰好 4 片，片序 `[d1, op, d2, =]`，片数不符直接返回 `None`）→ bbox 左上角锚定 16×14 画布 → NCC 最近邻（阈值 `NCC_MIN=0.88`、平局间隙 `NCC_GAP=0.03`，带 ±1px 位移补偿）。

实测指标（100 张样本，70 训练 / 30 holdout）：**正确 100%、自信错误 0、拒绝 0**。三分类口径很关键——**自信错误**会消耗 CAS 连续错误计数（可能锁号），是红线；**拒绝**（返回 `None`）无害，上层刷新换图重试即可（`cas_live.rs` 里就是 `for attempt in 1..=3 { 识别失败 → continue }`，**识别不出绝不提交**）。

> 为什么不用 ddddocr 等成熟库：调研对比后，其错误集中在 `0↔o/O` 类**自信错误**、模型 +13 MB 且许可不明；自研方案在本校这个**单一字体、无噪点、无旋转、渲染位置完全确定**的 kaptcha 上指标更好、体积 331 KB、零依赖。

### 3. 课表数据源（M2.5 已侦察，尚未接线）

- 教务 SSO 入口 `https://jwgl.cwxu.edu.cn/sso/lyiotlogin`（CAS `service` 常量已备于 `cas.rs:19` 的 `JWGL_SERVICE`）
- 课表接口 `POST /jwglxt/kbcx/xskbcx_cxXsKb.html?gnmkdm=N2151`，body `xnm=<学年起始年>&xqm=<3=第1学期/12=第2学期/16=暑期>`
- 响应 `kbList[]` 关键字段：`kcmc` 课程名 / `cdmc` 教室 / `jxb_id` 教学班 ID（自动更新 diff 的匹配键）/ `jc` 节次 / `oldzc` **周次十进制位掩码**（bit0=第 1 周，`expand_week_mask()` 负责展开）
- 已锚定：开学日 2026-09-07 → 2026-09-17 为第 2 周（与门户一致）

### 4. 慧新E校（内网，M3 用）

`berserker` 系接口一律带 `synAccessSource=app`（`pc` 来源被服务端拒，4030 故障结论）；电费查询 `http://10.3.100.110/charge-pc/pays/450` 匿名可用。token 存 sessionStorage 不跨标签页，所以必须由客户端后端持 token。

---

## 五、怎么跑

```bash
# 全量离线测试（35 passed / 3 ignored）
cargo test --workspace

# 前端类型检查 + 构建
cd tauri-app/frontend && npm run build

# 桌面端开发（必须在 tauri-app/ 下起，CLI 只从 cwd 及子目录发现 src-tauri）
cd tauri-app && npm run dev
# 打包 Windows 安装包（NSIS）
cd tauri-app && npm run build

# live 端到端（真实凭据 + 校园网；验证码自动识别，无需人工答题）
CAMPUS_HUB_CREDS=<凭据文件路径> cargo test -p campus-auth -- --ignored cas_live

# 验证码：重采样本 → 重建模板 → 三分类评测（模板由前一步产出，评测只读磁盘模板）
node scripts/captcha-collect.mjs 100
cargo test -p campus-auth --test captcha_solve -- --ignored build_templates
cargo test -p campus-auth --test captcha_solve -- --ignored captcha_solve_eval
```

环境依赖：Rust 工具链 + Node/npm（Tauri CLI 由 `tauri-app/package.json` 的本地依赖提供）。DPAPI 与 `src-tauri` 内嵌测试仅 Windows 生效（`#[cfg(all(test, target_os = "windows"))]`）。

---

## 六、铁律（违反会出事）

1. **验证码永不上传第三方**，识别必须离线本地推理（隐私红线）。
2. **识别不出就不提交**（`None` → 刷新换图）。猜错会吃 CAS 连续错误计数、可能锁号。
3. **凭据零明文落盘**：密码只以 DPAPI 密文进 `accounts.json`；日志用户名打码（`mask_username`）；`list_accounts` 不返回密码字段。
4. 提交用明确路径 `git add <file>`，不用 `-A`；不 `git push --force`；不动其他会话的分支与 worktree。
5. 协议层错误集中在 `crates/` 封装，前端只认 `CommandResult` 三态，不自行解析网络错误。

---

## 七、已知坑与教训（`.codewiki/learnings/` 有完整版）

- `kaptcha-arithmetic-five-roots.md` — 验证码正确率从 1.4% 爬到 100% 的五个独立根因（灰度丢形、**标注错位**、片序错、细笔画被阈值切掉、平局判定用错口径）。最有价值的一条：**批量多图标注不可靠**，改 contact sheet 逐行人工标注后才对。
- `cas-sso-plaintext-redirect.md` — 自动重定向对中间跳 body 中断零容错，必须手动跟随 + 容忍 body 读取失败 + 循环内升级 https。
- `subagent-batch-image-labeling.md` — 分身批量标注图片不可信的实证与替代流程。
- `decisions/recording-jar-session.md` — reqwest 内部 Jar 读不回，故自实现 `CookieStore` 委托并记录。
- 代码内 4 处 `ponytail:` 注释标记了有意的简化与升级路径（`cas.rs` 过期精确检测、`jar.rs` 多域恢复、`timeslots.rs` 作息可编辑、`crypto.rs` 安卓端加密分阶段）。

---

## 八、下一步（建议顺序）

**M2 门户核心页**是主线，但**缺一个协议层**——建议第一个任务就是补侦察 + 建 `crates/campus-portal`：

1. 侦察门户业务接口（资讯分类/列表/正文、待办中心、日程会议、应用中心 `appScheme`），落成 F12 抓包记录 + 新 crate 的解析器与测试（沿用 `campus-auth` 的分层与测试风格）。
2. 首页 v2：`TodayPanel.tsx` 现在有三处占位——钱包三卡（`:20-24`，数据源是慧新E校实时接口，与 M3 交叉）、下一节课（`:27-32`，等 M2.5 导入）、快捷动作（`:35-46`，多数目标页尚未落地）。
3. 各占位面板逐个替换（资讯/待办/日程/应用）。

**M2.5 剩余**（内核已就绪，缺接线与 UI）：

- 教务导入：CAS 会话 SSO 进 jwgl → 调课表接口 → `zhengfang::parse_kb_response()` 落库
- 课表页 UI：周视图 + 周次切换器 + 课程详情浮层 + 手动添加 + ICS 导出。**注意**：`PanelId` 目前冻结 8 项（`frontend/src/shared/types.ts:3`），加 `timetable` 需同步改 `types.ts` + `DockNav.tsx` 的 `DOCK_ITEMS` + zustand persist 兼容
- 调课通知引擎（L1 规则解析 + L2 置信分级，高置信自动应用 / 低置信待确认）

**M3/M4/M5** 的侦察结论已写在 `PLAN.md` 与设计文档附录，不需要重新摸索。

---

## 九、未决与风险

| 事项 | 说明 |
|---|---|
| **GUI 真机登录尚未人工确认** | live 测试验证的是 Rust 协议层；`docs/verify/m1-login.png` 只证明 `tauri dev` 能起窗口并渲染登录页。**建议接手者第一件事**：`cd tauri-app && npm run dev`，用真实账号点一次登录，确认命令面连通（顺带会把账号存进 DPAPI 账号库） |
| 门户接口无文档 | 学校升级可能变更；协议集中在 crate 内封装，失败要给降级提示 |
| `synAccessSource=app` | 依赖服务端当前配置现状，常量集中管理 |
| 教师身份课表端点 | 测试账号是学生角色，教师端点未侦察；导入时按角色选端点 |
| WebVPN URL 加密参数 | 深澜部署的 salt/key 待逆向（M4） |
| 验证码模板时效性 | 若学校改字体/尺寸/题面形态，指标会掉；重跑第五节的三条命令即可重建模板与复评 |

---

## 十、文档索引

| 文档 | 用途 |
|---|---|
| `PLAN.md` | **唯一计划来源**，里程碑勾选状态以此为准 |
| `CHANGELOG.md` | 逐条改动明细（倒序，最新在上） |
| `.codewiki/` | 架构/模块/概念/决策/教训（会话开始 `cw status`） |
| `docs/design/frontend-design.md` | 前端设计定稿（信息架构、Dock 规格、线框、视觉 token、应用中心 SSO 矩阵） |
| `docs/cas-recon/REPORT.md` | CAS/WebVPN 协议侦察原文与实测记录 |
| `docs/superpowers/plans/2026-09-17-m0-m1-foundation.md` | M0+M1 任务级实施计划（已完成，可作后续计划的格式模板） |
| `docs/verify/` | M1 验收截图（登录页 / Dock+待办页 / 今日页深色） |
| `THIRD-PARTY-NOTICES.md` + `crates/campus-schedule/NOTICE.md` | 开源合规（shiguangschedule 逐文件映射） |
