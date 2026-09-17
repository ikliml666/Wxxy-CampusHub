# 更新日志

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
