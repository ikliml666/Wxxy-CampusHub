// CAS 协议公共库（probe.js / webvpn.js 共用）
const fs = require('fs');
const rsa = require('./rsa30.js');

const CAS = 'https://wxcas.cwxu.edu.cn/lyuapServer';
const PUB_E = '010001';
const MOD = '00b5eeb166e069920e80bebd1fea4829d3d1f3216f2aabe79b6c47a3c18dcee5fd22c2e7ac519cab59198ece036dcf289ea8201e2a0b9ded307f8fb704136eaeb670286f5ad44e691005ba9ea5af04ada5367cd724b5a26fdb5120cc95b6431604bd219c6b7d83a6f8f24b43918ea988a76f93c333aa5a20991493d4eb1117e7b1';
const TAG = 'lyasp';
const UA = 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36';

function enc(plain) {
  rsa.a(131);
  const key = rsa.b(PUB_E, '', MOD);
  return rsa.c(key, plain);
}

function jarHeaders(jar) {
  return Object.entries(jar).map(([k, v]) => `${k}=${v}`).join('; ');
}
function absorbCookies(jar, res) {
  for (const c of (res.headers.getSetCookie?.() ?? [])) {
    const [pair] = c.split(';');
    const i = pair.indexOf('=');
    if (i > 0) jar[pair.slice(0, i).trim()] = pair.slice(i + 1).trim();
  }
}

function readCreds(file) {
  const text = fs.readFileSync(file, 'utf8');
  const m = text.match(/账号\s*([^\s，,]+)\s*[，,]\s*密码\s*(\S+)/);
  if (!m) throw new Error('凭据文件格式应为：账号xxx，密码yyy');
  return { user: m[1], pass: m[2] };
}

// 完整 CAS 账密登录：返回 { tgt, ticket, jar }
async function casLogin(user, pass, answer, service, jar = {}) {
  const { uid } = JSON.parse(fs.readFileSync(require('os').tmpdir() + '/cas-probe-state.json', 'utf8'));
  const body = new URLSearchParams({
    username: user,
    password: enc(pass),
    service,
    loginType: '',
    id: uid,
    code: String(answer).trim(),
    otpcode: '',
  });
  const r = await fetch(`${CAS}/v1/tickets`, {
    method: 'POST',
    headers: {
      'User-Agent': UA,
      'Content-Type': 'application/x-www-form-urlencoded',
      token: enc(TAG + Date.now()),
      Referer: `${CAS}/login?service=${encodeURIComponent(service)}`,
      Origin: 'https://wxcas.cwxu.edu.cn',
    },
    body: String(body),
  });
  const j = await r.json();
  absorbCookies(jar, r);
  const code = typeof j?.data === 'object' ? j?.data?.code : undefined;
  if (code) throw new Error('CAS 登录失败: ' + JSON.stringify(j.data));
  const ticket = typeof j?.data === 'string' ? j.data : (j?.data?.ticket ?? j?.ticket);
  if (!ticket) throw new Error('CAS 响应无 ticket: ' + JSON.stringify(j).slice(0, 200));
  return { tgt: j?.tgt ?? j?.data?.tgt, ticket, jar };
}

module.exports = { CAS, UA, enc, jarHeaders, absorbCookies, readCreds, casLogin };
