# 更新日志

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
