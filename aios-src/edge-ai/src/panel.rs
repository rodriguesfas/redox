//! Minimal HTML control panel served at `/` by the daemon. A single page with
//! inline JS that polls the JSON API — no external assets, dark theme.

pub fn index() -> &'static str {
    r#"<!doctype html>
<html lang="pt-BR">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>AIOS Edge AI OS</title>
<style>
  :root { color-scheme: dark; }
  body { font: 14px/1.5 ui-monospace, Menlo, Consolas, monospace; margin: 0; background: #0f1420; color: #dfe7f5; }
  header { padding: 14px 20px; background: #151c2c; border-bottom: 1px solid #26324a; display: flex; gap: 12px; align-items: baseline; }
  header h1 { font-size: 16px; margin: 0; color: #8ab4ff; }
  header span { color: #6b7a99; }
  main { padding: 20px; max-width: 860px; margin: 0 auto; }
  .grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(220px, 1fr)); gap: 12px; }
  .card { background: #161e30; border: 1px solid #26324a; border-radius: 8px; padding: 12px 14px; }
  .card h2 { font-size: 12px; text-transform: uppercase; letter-spacing: .08em; color: #8ab4ff; margin: 0 0 8px; }
  table { width: 100%; border-collapse: collapse; font-size: 13px; }
  th, td { text-align: left; padding: 4px 8px; border-bottom: 1px solid #1e2940; }
  th { color: #6b7a99; font-weight: normal; }
  input, button { background: #1a2336; color: #dfe7f5; border: 1px solid #2c3a58; border-radius: 6px; padding: 6px 10px; font: inherit; }
  button { cursor: pointer; }
  button:hover { background: #22304c; }
  textarea { width: 100%; background: #1a2336; color: #dfe7f5; border: 1px solid #2c3a58; border-radius: 6px; padding: 8px; font: inherit; min-height: 64px; box-sizing: border-box; }
  .row { display: flex; gap: 8px; margin: 8px 0; align-items: center; flex-wrap: wrap; }
  .muted { color: #6b7a99; }
  .ok { color: #7ddb8a; } .bad { color: #ff7b7b; }
  pre { background: #101525; border: 1px solid #1e2940; border-radius: 8px; padding: 10px; overflow: auto; max-height: 320px; white-space: pre-wrap; }
  .bar { background: #1a2336; border-radius: 4px; height: 8px; overflow: hidden; }
  .bar > div { background: #4c8dff; height: 100%; }
</style>
</head>
<body>
<header>
  <h1>AIOS Edge AI OS</h1>
  <span id="health">…</span>
  <button onclick="refresh()">Refresh</button>
</header>
<main>
  <div class="grid">
    <div class="card"><h2>Status</h2><div id="status" class="muted">loading…</div></div>
    <div class="card"><h2>Sistema</h2><div id="system" class="muted">loading…</div></div>
    <div class="card"><h2>Inferência</h2><div id="inferstats" class="muted">loading…</div></div>
  </div>

  <div class="card" style="margin-top:12px"><h2>Avaliação (infer)</h2>
    <div class="row">
      <input id="model" placeholder="modelo (vazio = primeiro instalado)" style="flex:1">
      <input id="maxtok" type="number" value="64" min="1" max="512" style="width:90px">
    </div>
    <textarea id="prompt" placeholder="prompt…">Once upon a time,</textarea>
    <div class="row">
      <button onclick="doInfer()">Executar</button>
      <button onclick="doBench()">Benchmark (parse+soma+infer)</button>
    </div>
    <pre id="result">–</pre>
  </div>

  <div class="card" style="margin-top:12px"><h2>Modelos instalados</h2>
    <table><thead><tr><th>nome</th><th>arquitetura</th><th>tamanho</th></tr></thead><tbody id="models"><tr><td colspan="3" class="muted">…</td></tr></tbody></table>
  </div>

  <div class="card" style="margin-top:12px"><h2>Histórico de métricas</h2>
    <table><thead><tr><th>t</th><th>CPU %</th><th>RAM</th><th>req</th><th>tok</th><th>tps</th></tr></thead><tbody id="history"></tbody></table>
  </div>

  <div class="card" style="margin-top:12px"><h2>Logs</h2><pre id="logs">…</pre></div>
</main>
<script>
async function api(path, opts) {
  const r = await fetch(path, opts);
  if (!r.ok && r.status !== 500) throw new Error(path + ' -> ' + r.status);
  return r.json();
}
function $id(x) { return document.getElementById(x); }
function fmt(t) { return t == null ? '–' : Number(t).toFixed(1); }

async function refresh() {
  try {
    const h = await api('/api/health');
    $id('health').textContent = h.status + ' · v' + h.version + ' · up ' + h.uptime_s + 's';
    $id('status').innerHTML = 'serviço <span class="ok">ok</span><br>' + h.models_dir;
    const mm = await api('/api/metrics');
    const s = mm.system || {};
    $id('system').innerHTML =
      'CPU <b>' + fmt(s.cpu_percent) + '%</b> · RAM <b>' + fmt(s.mem_used_mb) + ' MB</b>' +
      ' (' + fmt(s.mem_total_mb) + ')<br>rx ' + fmt(s.net_rx_kbps) + ' kb/s · tx ' + fmt(s.net_tx_kbps) + ' kb/s' +
      '<br>cores ' + (s.parallelism ?? '–');
    const inf = mm.infer || {};
    $id('inferstats').innerHTML =
      'req <b>' + inf.requests + '</b> · erros <b>' + inf.errors + '</b> · tokens <b>' + inf.tokens + '</b>' +
      '<br>lat média <b>' + fmt(inf.avg_latency_ms) + ' ms</b> · último <b>' + fmt(inf.last_tps) + ' t/s</b>';
    const ml = await api('/api/models');
    $id('models').innerHTML = ml.models.map(m =>
      '<tr><td>' + m.name + '</td><td>' + (m.architecture || '–') + '</td><td>' + m.size + '</td></tr>').join('')
      || '<tr><td colspan="3" class="muted">nenhum modelo</td></tr>';
    const hist = (mm.history || []).slice(-30).reverse().map(p =>
      '<tr><td>' + fmt(p.ts % 100000) + '</td><td>' + fmt(p.cpu_percent) + '</td><td>' + fmt(p.mem_used_mb) + '</td><td>' + p.requests + '</td><td>' + p.tokens + '</td><td>' + fmt(p.last_tps) + '</td></tr>').join('');
    $id('history').innerHTML = hist || '<tr><td colspan="6" class="muted">aguardando amostras…</td></tr>';
    const lg = await api('/api/logs?tail=60');
    $id('logs').textContent = lg.logs.map(l => '[' + l.level + '] ' + l.msg).join('\n') || '–';
  } catch (e) {
    $id('health').textContent = 'erro: ' + e.message;
  }
}

async function doInfer() {
  const model = $id('model').value || undefined;
  const body = { prompt: $id('prompt').value, max_tokens: parseInt($id('maxtok').value) || 64 };
  if (model) body.model = model;
  $id('result').textContent = '…';
  try {
    const r = await api('/api/infer', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) });
    $id('result').textContent = '→ ' + r.text + '\n\n(' + r.tokens_per_second.toFixed(2) + ' t/s, load ' + r.load_ms + ' ms, cached ' + r.cached + ')';
    refresh();
  } catch (e) { $id('result').textContent = 'erro: ' + e.message; }
}

async function doBench() {
  const model = $id('model').value;
  $id('result').textContent = '…';
  try {
    const r = await api('/api/benchmark?tokens=16' + (model ? '&model=' + encodeURIComponent(model) : ''));
    $id('result').textContent = JSON.stringify(r, null, 2);
    refresh();
  } catch (e) { $id('result').textContent = 'erro: ' + e.message; }
}

refresh();
setInterval(refresh, 5000);
</script>
</body>
</html>"#
}
