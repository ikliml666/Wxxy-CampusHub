// CAS 登录协议端到端验证探针（无依赖，node 18+ 内置 fetch）
// 用法：
//   node probe.js --captcha            # 抓验证码图存 captcha.png，uid 存 state.json
//   node probe.js <学号> <密码> <答案>  # 提交登录（密码不落日志）
// 成功判据：响应 data.code == "NOUSER" 表示验证码+加密+字段全部通过（仅账号不存在）
//          data.ticket 存在表示登录成功
const fs = require('fs');
const path = require('path');
const rsa = require('./rsa30.js');

const CAS = 'https://wxcas.cwxu.edu.cn/lyuapServer';
const SERVICE = 'https://my.cwxu.edu.cn/shiro-cas';
const PUB_E = '010001';
const MOD = '00b5eeb166e069920e80bebd1fea4829d3d1f3216f2aabe79b6c47a3c18dcee5fd22c2e7ac519cab59198ece036dcf289ea8201e2a0b9ded307f8fb704136eaeb670286f5ad44e691005ba9ea5af04ada5367cd724b5a26fdb5120cc95b6431604bd219c6b7d83a6f8f24b43918ea988a76f93c333aa5a20991493d4eb1117e7b1';
const TAG = 'lyasp';
const UA = 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36';
const HERE = __dirname;
const STATE = path.join(require('os').tmpdir(), 'cas-probe-state.json');

function enc(plain) {
  rsa.a(131);
  const key = rsa.b(PUB_E, '', MOD);
  return rsa.c(key, plain);
}

async function main() {
  const args = process.argv.slice(2);
  if (args[0] === '--captcha') {
    const r = await fetch(`${CAS}/kaptcha`, { headers: { 'User-Agent': UA } });
    const j = await r.json();
    fs.writeFileSync(path.join(HERE, 'captcha.png'), Buffer.from(j.content.split(',')[1], 'base64'));
    fs.writeFileSync(STATE, JSON.stringify({ uid: j.uid, kaptchaType: j.kaptchaType }));
    console.log(`kaptchaType=${j.kaptchaType} uid=${j.uid}`);
    console.log(`图片已存 ${path.join(HERE, 'captcha.png')}，看图后运行: node probe.js <学号> <密码> <答案>`);
    return;
  }
  const [user, pass, answer] = args;
  if (!user || !pass || !answer) { console.error('用法: node probe.js <学号> <密码> <验证码答案>'); process.exit(1); }
  const { uid } = JSON.parse(fs.readFileSync(STATE, 'utf8'));
  const ts = Date.now();
  const body = new URLSearchParams({
    username: user,
    password: enc(pass),        // 密码 RSA 密文，不打印
    service: SERVICE,
    loginType: '',
    id: uid,                    // 验证码 uid → v.id
    code: answer.trim(),        // 验证码答案 → v.code
    otpcode: '',
  });
  const r = await fetch(`${CAS}/v1/tickets`, {
    method: 'POST',
    headers: {
      'User-Agent': UA,
      'Content-Type': 'application/x-www-form-urlencoded',
      token: enc(TAG + ts),     // token 头 = RSA(TAG + 毫秒时间戳)
      Referer: `${CAS}/login?service=${encodeURIComponent(SERVICE)}`,
      Origin: 'https://wxcas.cwxu.edu.cn',
    },
    body: String(body),
  });
  console.log('HTTP', r.status);
  const setCookie = r.headers.getSetCookie?.() ?? [];
  console.log('Set-Cookie:', setCookie.map(c => c.split(';')[0]).join(' | ') || '(无)');
  const j = await r.json();
  console.log('响应:', JSON.stringify(j));
  if (j?.data?.ticket) {
    console.log('\n=== 登录成功，携带 ticket 回跳 service ===');
    const r2 = await fetch(`${SERVICE}?ticket=${encodeURIComponent(j.data.ticket)}`, {
      headers: { 'User-Agent': UA }, redirect: 'manual',
    });
    console.log('回跳 HTTP', r2.status, '->', r2.headers.get('location') ?? '(无 Location)');
    console.log('门户 Set-Cookie:', (r2.headers.getSetCookie?.() ?? []).map(c => c.split(';')[0]).join(' | ') || '(无)');
  }
}

main().catch(e => { console.error('FAILED:', e.message); process.exit(1); });
