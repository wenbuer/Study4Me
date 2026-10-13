/* 致知 前端：视图切换、导入、学习、考试、指导图。 */

const tauriCore = window.__TAURI__ && window.__TAURI__.core;
const invoke = tauriCore
  ? tauriCore.invoke
  : async () => { throw new Error('Tauri 运行时未就绪，请通过桌面应用启动'); };

const state = {
  view: 'today',
  info: null,
  queue: { due: [], fresh: [], stats: {} },
  materials: [],
  preview: null,
  cards: [],
  learnIndex: 0,
  examCard: null,
  graph: 'stage',
  graphSvg: '',
  openMaterial: null,
  busy: false,
  importing: null,
  renaming: null,
  collapsed: [],
  learnQuery: '',
  learnMaterial: null,
};

const VIEW_TITLES = {
  today: '开始', import: '材料', learn: '学习', exam: '考试', insight: '指导图', settings: '设置',
};

const el = (id) => document.getElementById(id);

function escapeHtml(value) {
  return String(value == null ? '' : value)
    .replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;');
}

let toastTimer = null;
function toast(message, kind) {
  const node = el('toast');
  node.textContent = message;
  node.className = 'toast' + (kind === 'bad' ? ' is-bad' : kind === 'ok' ? ' is-ok' : '');
  node.hidden = false;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => { node.hidden = true; }, kind === 'bad' ? 5200 : 2600);
}

async function call(command, args) {
  try {
    return await invoke(command, args || {});
  } catch (error) {
    toast(typeof error === 'string' ? error : (error && error.message) || '调用失败', 'bad');
    throw error;
  }
}

function withBusy(button, label, task) {
  const original = button ? button.textContent : '';
  if (button) { button.disabled = true; button.innerHTML = '<span class="spinner"></span>' + label; }
  return Promise.resolve()
    .then(task)
    .finally(() => { if (button) { button.disabled = false; button.textContent = original; } });
}

/* ── 视图切换 ─────────────────────────────── */

function switchView(name) {
  state.view = name;
  document.querySelectorAll('.view').forEach((node) => {
    node.classList.toggle('is-active', node.dataset.view === name);
  });
  document.querySelectorAll('.nav-item').forEach((node) => {
    node.classList.toggle('is-active', node.dataset.view === name);
  });
  el('viewTitle').textContent = VIEW_TITLES[name] || name;
  if (name === 'insight') renderGraph();
  if (name === 'import') renderMaterials();
  if (name === 'learn') renderLearn();
  if (name === 'settings') loadSettings().catch(() => {});
}

/* ── 数据加载 ─────────────────────────────── */

async function refresh() {
  const results = await Promise.all([
    call('app_info'),
    call('today_queue'),
    call('list_materials'),
    call('recent_reviews', { limit: 8 }),
  ]);
  state.info = results[0];
  state.queue = results[1];
  state.materials = results[2];
  state.recent = results[3];
  state.cards = state.queue.fresh.concat(state.queue.due);
  renderChrome();
  renderToday();
  renderMaterials();
  renderPlans();
  renderLearn();
}

function renderChrome() {
  const s = state.queue.stats || {};
  el('railDue').textContent = s.dueToday || 0;
  el('railDue').className = 'rail-stat-value is-due';
  el('railStreak').textContent = s.streakDays || 0;
  el('railStreak').className = 'rail-stat-value is-streak';
  el('railModel').textContent = state.info
    ? (state.info.llmReady ? state.info.model : '未配置 API Key')
    : '连接中';
  el('topMetrics').innerHTML = [
    '已学 <b>' + (s.learned || 0) + '</b> / ' + (s.cards || 0),
    '今日到期 <b>' + (s.dueToday || 0) + '</b>',
    '平均分 <b>' + (s.avgScore || 0) + '</b>',
    '连续 <b>' + (s.streakDays || 0) + '</b> 天',
  ].map((item) => '<span>' + item + '</span>').join('');
}

/* ── 今日 ─────────────────────────────────── */

function ring(pct, number, caption) {
  const r = 54;
  const c = 2 * Math.PI * r;
  const offset = c * (1 - Math.max(0, Math.min(1, pct)));
  return '<div class="ring"><svg width="128" height="128" viewBox="0 0 128 128">' +
    '<circle class="ring-track" cx="64" cy="64" r="' + r + '" fill="none" stroke-width="8"/>' +
    '<circle class="ring-value" cx="64" cy="64" r="' + r + '" fill="none" stroke-width="8" ' +
    'stroke-dasharray="' + c.toFixed(1) + '" stroke-dashoffset="' + offset.toFixed(1) + '"/></svg>' +
    '<div class="ring-label"><span class="ring-num">' + number + '</span>' +
    '<span class="ring-cap">' + caption + '</span></div></div>';
}

function cardRow(card) {
  const dot = !card.learned ? 'dot-new'
    : card.due <= today() ? 'dot-due'
    : card.reps > 0 ? 'dot-review' : 'dot-fresh';
  return '<div class="row" data-card="' + card.id + '" data-mode="exam">' +
    '<span class="dot ' + dot + '"></span>' +
    '<div class="row-main"><div class="row-title">' + escapeHtml(card.title) + '</div>' +
    '<div class="row-sub">' + escapeHtml(card.materialTitle) + '</div></div>' +
    '<div class="row-side">' + (card.learned ? '到期 ' + card.due : '未学') + '</div></div>';
}

function today() {
  const d = new Date();
  const m = String(d.getMonth() + 1).padStart(2, '0');
  const day = String(d.getDate()).padStart(2, '0');
  return d.getFullYear() + '-' + m + '-' + day;
}

function renderToday() {
  const s = state.queue.stats || {};
  const due = state.queue.due || [];
  const fresh = state.queue.fresh || [];
  const pct = s.cards ? s.learned / s.cards : 0;

  let eyebrow = '进度';
  let title = '';
  let copy = '';
  let actions = '';

  if (due.length > 0) {
    eyebrow = '记忆曲线提醒';
    title = '今天有 ' + due.length + ' 张卡到期';
    copy = '这些卡正好落在记忆即将衰减的位置，先清掉它们，再去学新内容。';
    actions = '<button class="btn btn-primary" data-action="review-due">开始复习</button>' +
      '<button class="btn" data-action="go-import">补新材料</button>';
  } else if (fresh.length > 0) {
    eyebrow = '今天没有到期卡';
    title = '可以推进 ' + fresh.length + ' 个新知识点';
    copy = '学完当天就考一次，第一次答题的结果决定后面几周的复习节奏。';
    actions = '<button class="btn btn-primary" data-action="learn-first">开始学习</button>' +
      '<button class="btn" data-action="go-import">导入材料</button>';
  } else {
    eyebrow = '还没有内容';
    title = '先给 致知 一份材料';
    copy = '网页链接、PDF、Word 或直接粘贴文本，模型会拆成知识点卡片与考题。';
    actions = '<button class="btn btn-primary" data-action="go-import">去导入</button>';
  }

  el('todayFocus').innerHTML =
    '<div class="focus"><div>' +
    '<div class="focus-eyebrow">' + eyebrow + '</div>' +
    '<h3 class="focus-title">' + title + '</h3>' +
    '<p class="focus-copy">' + copy + '</p>' +
    '<div class="focus-actions">' + actions + '</div>' +
    '</div>' + ring(pct, (s.learned || 0) + '/' + (s.cards || 0), '已学知识点') + '</div>';

  el('dueHint').textContent = due.length ? '点击任意一张开始' : '今天很轻松';
  el('dueList').innerHTML = due.length
    ? due.map(cardRow).join('')
    : '<div class="empty">没有到期卡片。晚点再来，或者去学新知识点。</div>';

  const stages = ['入门', '铺开', '巩固', '精通'];
  const active = stages.indexOf(s.stage || '未开始');
  const nodes = stages.map((name, i) => {
    const cls = i === active ? 'stage-node is-active' : (active > i ? 'stage-node is-passed' : 'stage-node');
    return '<div class="' + cls + '">' + name + '</div>';
  }).join('');
  el('stagePanel').innerHTML =
    '<div class="stage-track">' + nodes + '</div>' +
    '<div class="bar"><span style="width:' + Math.round((s.stageProgress || 0) * 100) + '%"></span></div>' +
    '<div class="metric-grid">' +
    metric(s.cards || 0, '知识点总数') + metric(s.reviewsTotal || 0, '累计练习') +
    metric(s.activeDays || 0, '学习天数') + metric(s.avgScore || 0, '答题平均分') +
    '</div>';

  const recent = state.recent || [];
  el('recentList').innerHTML = recent.length
    ? recent.map((r) => '<div class="row" data-card="' + r.cardId + '" data-mode="exam">' +
        '<div class="row-main"><div class="row-title">' + escapeHtml(r.cardTitle) + '</div>' +
        '<div class="row-sub">' + escapeHtml(r.ratedAt.slice(5, 16)) + '</div></div>' +
        '<div class="row-side">' + (r.score == null ? ratingLabel(r.rating) : r.score + ' 分') + '</div></div>').join('')
    : '<div class="empty">还没有练习记录。</div>';
}

function metric(value, label) {
  return '<div class="metric"><div class="metric-value">' + value + '</div>' +
    '<div class="metric-label">' + label + '</div></div>';
}

function ratingLabel(rating) {
  return { 1: '忘记', 2: '有点难', 3: '记得', 4: '太简单' }[rating] || '已复习';
}

/* ── 材料 ─────────────────────────────────── */


/* 内容到手之后：切分单元 → 补考点，全过程进度都写在材料库那一行上。 */
async function importItem(item) {
  state.importing = { label: '正在切分学习单元…', source: item.title };
  renderMaterials();
  const out = await call('create_material', { item: item });
  state.importing = { label: '已切分 ' + out.units + ' 个单元 · 正在补考点…', source: item.title };
  renderMaterials();
  const result = await polishUnits(out.materialId);
  state.importing = null;
  if (result.error) {
    toast('已导入 ' + out.units + ' 个单元，但考点没生成：' + result.error, 'bad');
  } else {
    toast('导入完成：' + out.units + ' 个学习单元 · ' + result.polished + ' 份考点', 'ok');
  }
  await refresh();
  switchView('learn');
}

/* 给还没有考点的单元补考点；返回补了几份、以及第一处失败原因。 */
async function polishUnits(materialId) {
  let polished = 0;
  let error = '';
  for (;;) {
    const step = await call('polish_units_step', { materialId: materialId, limit: 6 });
    polished += step.polished;
    if (step.error && !error) error = step.error;
    if (state.importing) {
      state.importing.label = '正在补考点 ' + polished + '/' + (polished + step.remaining) + '…';
      renderMaterials();
    }
    if (step.done || step.polished === 0) break;
  }
  return { polished: polished, error: error };
}


function renderMaterials() {
  const list = state.materials || [];
  el('materialHint').textContent = list.length ? list.length + ' 份材料' : '还没有材料';

  const busy = state.importing
    ? '<div class="row is-busy"><span class="spinner"></span>' +
      '<div class="row-main"><div class="row-title">' + escapeHtml(state.importing.label) + '</div>' +
      '<div class="row-sub">' + escapeHtml(state.importing.source) + '</div></div></div>'
    : '';

  const rows = list.map((m) => {
    const renaming = state.renaming === m.id;
    return '<div class="row" data-material="' + m.id + '">' +
      '<div class="row-main">' +
      (renaming
        ? '<input class="inline-input" data-rename="' + m.id + '" value="' + escapeHtml(m.title) + '" />'
        : '<div class="row-title">' + escapeHtml(m.title) + '</div>') +
      '<div class="row-sub">' + m.sourceKind +
      (m.pendingCount ? ' · 待补考点 ' + m.pendingCount : '') +
      ' · ' + m.chars + ' 字 · ' + m.cardCount + ' 个单元 · ' +
      escapeHtml(m.createdAt.slice(0, 10)) + '</div></div>' +
      (m.pendingCount
        ? '<button class="btn btn-slim" data-act="polish" data-id="' + m.id + '">补考点</button>'
        : '') +
      '<button class="btn btn-slim" data-act="rename" data-id="' + m.id + '">重命名</button>' +
      '<button class="btn btn-slim" data-act="export" data-id="' + m.id + '">导出</button>' +
      '<button class="btn btn-slim" data-act="del" data-id="' + m.id + '">删除</button></div>';
  }).join('');

  el('materialList').innerHTML = busy + (list.length
    ? rows
    : (state.importing ? '' : '<div class="empty">还没有材料。左边导入第一份吧。</div>'));

  if (state.renaming) {
    const input = document.querySelector('[data-rename]');
    if (input) {
      input.focus();
      input.select();
      input.addEventListener('keydown', (event) => {
        if (event.key === 'Enter') commitRename(input);
        if (event.key === 'Escape') { state.renaming = null; renderMaterials(); }
      });
      input.addEventListener('blur', () => commitRename(input));
    }
  }
}

async function commitRename(input) {
  const id = Number(input.dataset.rename);
  const title = input.value.trim();
  if (state.renaming === null) return;
  state.renaming = null;
  if (!title) { renderMaterials(); return; }
  input.disabled = true;
  await call('rename_material', { materialId: id, title: title });
  toast('已重命名', 'ok');
  await refresh();
}

/* 正文开头自带的一级标题常与卡片标题一字不差，去掉以免同一句显示两遍 */
function stripLeadTitle(markdown, title) {
  const norm = (text) => String(text || '')
    .replace(/[#*_>\u0060\s《》「」【】]/g, '')
    .toLowerCase();
  const want = norm(title);
  const lines = String(markdown || '').split('\n');
  let head = 0;
  while (head < lines.length && !lines[head].trim()) head += 1;
  if (!want || head >= lines.length) return markdown;
  const first = lines[head].trim().replace(/^#{1,6}\s*/, '');
  if (norm(first) !== want) return markdown;
  lines.splice(head, 1);
  return lines.join('\n');
}

/* ── 轻量 Markdown 渲染（学习页用，无第三方依赖）───────── */

function renderMarkdown(source) {
  const FENCE = '\u0060\u0060\u0060';
  const CODE = '\u0060([^\u0060]+)\u0060';
  const inline = (text) => escapeHtml(text)
    .replace(/\*\*([^*]+)\*\*/g, '<b>$1</b>')
    .replace(new RegExp(CODE, 'g'), '<code>$1</code>')
    .replace(/\[([^\]]+)\]\(([^)]+)\)/g, '<a href="$2" target="_blank" rel="noreferrer">$1</a>');

  const lines = String(source || '').split('\n');
  const html = [];
  let inCode = false;
  let code = [];
  let list = null;
  let table = [];

  const closeList = () => { if (list) { html.push('</' + list + '>'); list = null; } };
  const closeTable = () => {
    if (!table.length) return;
    const rows = table.filter((row) => !/^\|[\s:|-]+\|$/.test(row));
    html.push('<table>');
    rows.forEach((row, index) => {
      const cells = row.replace(/^\||\|$/g, '').split('|').map((cell) => cell.trim());
      const tag = index === 0 ? 'th' : 'td';
      html.push('<tr>' + cells.map((cell) => '<' + tag + '>' + inline(cell) + '</' + tag + '>').join('') + '</tr>');
    });
    html.push('</table>');
    table = [];
  };

  for (const raw of lines) {
    const line = raw.trimEnd();
    if (line.trim().startsWith(FENCE)) {
      if (inCode) {
        html.push('<pre><code>' + escapeHtml(code.join('\n')) + '</code></pre>');
        code = [];
        inCode = false;
      } else {
        closeList();
        closeTable();
        inCode = true;
      }
      continue;
    }
    if (inCode) { code.push(raw); continue; }

    const trimmed = line.trim();
    if (/^\|.*\|$/.test(trimmed)) { closeList(); table.push(trimmed); continue; }
    closeTable();

    const heading = trimmed.match(/^(#{1,6})\s+(.*)$/);
    if (heading) {
      closeList();
      const level = Math.min(heading[1].length + 1, 6);
      html.push('<h' + level + '>' + inline(heading[2]) + '</h' + level + '>');
      continue;
    }
    if (/^>\s?/.test(trimmed)) {
      closeList();
      html.push('<blockquote>' + inline(trimmed.replace(/^>\s?/, '')) + '</blockquote>');
      continue;
    }
    const bullet = line.match(/^\s*[-*+]\s+(.*)$/);
    if (bullet) {
      if (list !== 'ul') { closeList(); html.push('<ul>'); list = 'ul'; }
      html.push('<li>' + inline(bullet[1]) + '</li>');
      continue;
    }
    const numbered = line.match(/^\s*\d+[.)]\s+(.*)$/);
    if (numbered) {
      if (list !== 'ol') { closeList(); html.push('<ol>'); list = 'ol'; }
      html.push('<li>' + inline(numbered[1]) + '</li>');
      continue;
    }
    if (!trimmed) { closeList(); continue; }
    closeList();
    html.push('<p>' + inline(trimmed) + '</p>');
  }
  if (inCode && code.length) {
    html.push('<pre><code>' + escapeHtml(code.join('\n')) + '</code></pre>');
  }
  closeList();
  closeTable();
  return html.join('\n');
}

/* ── 学习计划 ─────────────────────────────── */

async function renderPlans() {
  const box = el('planBlock');
  let plans = [];
  try {
    plans = await call('list_plans');
  } catch (error) {
    box.innerHTML = '<div class="empty">读取学习计划失败</div>';
    return;
  }

  if (!plans.length) {
    const options = (state.materials || [])
      .map((m) => '<option value="' + m.id + '">' + escapeHtml(m.title) + '（' + m.cardCount + ' 单元）</option>')
      .join('');
    box.innerHTML =
      '<div class="block-head"><h2>安排学习计划</h2></div>' +
      '<div class="plan-sub">选一份素材库、定个期限，系统会把单元按天排好，每天照着学就行。</div>' +
      '<div class="plan-form">' +
      '<label>素材库<select id="planMaterial">' + (options || '<option value="">还没有材料</option>') + '</select></label>' +
      '<label>计划天数<input id="planDays" type="number" min="1" max="365" value="14" /></label>' +
      '<button class="btn btn-primary" id="createPlanBtn">生成计划</button>' +
      '</div>';
    const button = el('createPlanBtn');
    if (button) button.addEventListener('click', createPlan);
    return;
  }

  const current = await call('plan_today', { planId: plans[0].id });
  const plan = current ? current.plan : plans[0];
  const percent = plan.unitsTotal ? Math.round((plan.learned / plan.unitsTotal) * 100) : 0;
  const units = current ? current.dayUnits : [];
  const pending = units.filter((card) => !card.learned);
  const listBody = units.length
    ? '<div class="list list-compact" style="margin-top:14px">' + pending.slice(0, 6).map((card) =>
        '<div class="row is-static"><div class="row-main"><div class="row-title">' + escapeHtml(card.title) +
        '</div><div class="row-sub">' + escapeHtml(card.section || '') + '</div></div></div>').join('') +
      (pending.length > 6 ? '<div class="empty">还有 ' + (pending.length - 6) + ' 个</div>' : '') + '</div>'
    : '<div class="empty" style="margin-top:12px">今天这一段已经学完了</div>';

  box.innerHTML =
    '<div class="plan-head"><div>' +
    '<div class="plan-title">' + escapeHtml(plan.materialTitle) + '</div>' +
    '<div class="plan-sub">第 ' + current.dayIndex + ' / ' + plan.days + ' 天 · 每天 ' +
    plan.perDay + ' 个单元 · 今天还剩 ' + pending.length + ' 个 · 全库还剩 ' +
    current.remainingUnits + ' 个未学</div></div>' +
    '<div class="row-side">' + percent + '%</div></div>' +
    '<div class="bar" style="margin-top:12px"><span style="width:' + percent + '%"></span></div>' +
    listBody +
    '<div class="plan-actions">' +
    '<button class="btn btn-primary" data-action="plan-study" data-id="' + plan.id + '">开始今天的学习</button>' +
    '<button class="btn" data-action="plan-delete" data-id="' + plan.id + '">删除计划</button>' +
    '</div>';
}

async function createPlan() {
  const materialId = Number((el('planMaterial') || {}).value || 0);
  const days = Number((el('planDays') || {}).value || 0);
  if (!materialId) { toast('先去材料页导入一份材料', 'bad'); return; }
  if (!days || days < 1) { toast('填一个计划天数', 'bad'); return; }
  await call('create_plan', { materialId: materialId, days: days });
  toast('计划已生成：' + days + ' 天', 'ok');
  await refresh();
}

/* ── 学习 ─────────────────────────────────── */

function renderLearn() {
  // 素材库切换器
  const select = el('learnMaterial');
  if (select) {
    const html = ['<option value="">全部素材库</option>'].concat(
      (state.materials || []).map((m) =>
        '<option value="' + m.id + '">' + escapeHtml(m.title) + '</option>')).join('');
    if (select.dataset.rendered !== html) {
      select.innerHTML = html;
      select.dataset.rendered = html;
    }
    select.value = state.learnMaterial ? String(state.learnMaterial) : '';
  }

  const query = (state.learnQuery || '').trim().toLowerCase();
  let fresh = state.queue.fresh || [];
  if (state.learnMaterial) {
    fresh = fresh.filter((card) => card.materialId === state.learnMaterial);
  }
  if (query) {
    fresh = fresh.filter((card) =>
      (card.title || '').toLowerCase().includes(query) ||
      (card.section || '').toLowerCase().includes(query));
  }

  if (!fresh.length) {
    el('learnQueue').innerHTML = '<div class="empty">' +
      (query ? '没有匹配「' + escapeHtml(state.learnQuery) + '」的单元' : '这里还没有待学单元') + '</div>';
    el('learnCard').innerHTML = '<div class="empty">导入材料后，这里会按章节列出全部学习单元</div>';
    return;
  }

  // 按章节分组，章节标题可点开收起
  const groups = [];
  const index = {};
  fresh.forEach((card, position) => {
    const key = card.section || '未分类';
    if (!index[key]) { index[key] = { name: key, items: [] }; groups.push(index[key]); }
    index[key].items.push({ card: card, position: position });
  });

  el('learnQueue').innerHTML = groups.map((group) => {
    const folded = state.collapsed.indexOf(group.name) >= 0;
    return '<div class="unit-group">' +
      '<div class="unit-group-title" data-section="' + escapeHtml(group.name) + '">' +
      '<span><span class="chev">' + (folded ? '▶' : '▼') + '</span>' +
      escapeHtml(group.name) + '</span><span>' + group.items.length + '</span></div>' +
      (folded ? '' : group.items.map((entry) =>
        '<div class="row' + (entry.position === state.learnIndex ? ' is-active' : '') +
        '" data-learn="' + entry.card.id + '">' +
        '<div class="row-main"><div class="row-title">' + escapeHtml(entry.card.title) +
        '</div></div></div>').join('')) +
      '</div>';
  }).join('');

  const current = fresh[state.learnIndex] || fresh[0];
  const points = (current.keyPoints || []).length
    ? '<div class="unit-points"><div class="grade-sub">读完检查这几条</div><ul class="grade-list">' +
      current.keyPoints.map((point) => '<li>' + escapeHtml(point) + '</li>').join('') + '</ul></div>'
    : '';
  const quiz = current.question
    ? '<div class="unit-question"><div class="grade-sub">读完自测</div><p>' +
      escapeHtml(current.question) + '</p></div>'
    : '<div class="unit-question"><div class="grade-sub">读完自测</div><p class="block-note">' +
      '这一节的考点还在生成中，稍后刷新即可。</p></div>';

  el('learnCard').innerHTML =
    '<div class="card-meta">' + escapeHtml(current.section || '') + ' · ' +
    escapeHtml(current.materialTitle) + ' · 第 ' + current.seq + ' 单元</div>' +
    '<h3 class="card-title">' + escapeHtml(current.title) + '</h3>' +
    '<div class="unit-body">' +
      renderMarkdown(stripLeadTitle(current.content || current.summary, current.title)) + '</div>' +
    points + quiz +
    '<div class="card-actions">' +
    '<button class="btn btn-primary" data-action="learn-to-exam" data-id="' + current.id + '">学完去考试</button>' +
    '<button class="btn" data-action="learn-next">下一个单元</button></div>';
}

/* ── 考试 ─────────────────────────────────── */

async function loadExamCard(cardId, mode) {
  let card = state.cards.find((c) => c.id === cardId);
  if (!card) {
    const all = await call('list_cards', {});
    state.cards = all;
    card = all.find((c) => c.id === cardId);
  }
  if (!card) { toast('找不到这张卡', 'bad'); return; }
  state.examCard = card;
  state.examMode = mode || 'exam';
  el('examHeading').textContent = card.title;
  el('examSource').textContent = card.materialTitle + ' · 到期 ' + card.due + ' · 已复习 ' + card.reps + ' 次';
  el('examQuestion').textContent = card.question || '（这张卡没有题目，可先学一遍再自评）';
  el('examAnswer').value = '';
  el('gradeBlock').hidden = true;
  switchView('exam');
}

async function submitGrade() {
  const card = state.examCard;
  if (!card) { toast('先选一道题', 'bad'); return; }
  const answer = el('examAnswer').value.trim();
  if (!answer) { toast('先写下你的回答', 'bad'); return; }
  await withBusy(el('gradeBtn'), '评判中', async () => {
    const result = await call('grade_answer', { cardId: card.id, answer: answer });
    const updated = await call('submit_review', {
      cardId: card.id, rating: result.rating, score: result.score, answer: answer, mode: 'exam',
    });
    el('gradeBlock').hidden = false;
    el('scorePill').textContent = result.score + ' / 10 · ' + ratingLabel(result.rating);
    const missed = (result.missed || []).map((m) => '<li>' + escapeHtml(m) + '</li>').join('');
    el('gradeBody').innerHTML =
      '<p class="grade-text">' + escapeHtml(result.feedback).replace(/\n/g, '<br>') + '</p>' +
      (missed ? '<div class="grade-sub">遗漏要点</div><ul class="grade-list">' + missed + '</ul>' : '') +
      (result.suggestion ? '<div class="grade-sub">参考回答</div><p class="grade-text">' +
        escapeHtml(result.suggestion) + '</p>' : '') +
      '<div class="grade-sub">下次复习 ' + updated.due + '（' + Math.max(1, Math.round(
        (new Date(updated.due) - new Date(today())) / 86400000)) + ' 天后）</div>';
    toast('已排到 ' + updated.due, 'ok');
    await refresh();
  });
}

async function submitSelfRating(rating) {
  const card = state.examCard;
  if (!card) { toast('先选一道题', 'bad'); return; }
  const updated = await call('submit_review', { cardId: card.id, rating: rating, mode: 'self' });
  toast('自评已记录，下次复习 ' + updated.due, 'ok');
  el('selfNote').textContent = '下次复习：' + updated.due;
  await refresh();
}

/* ── 指导图 ───────────────────────────────── */

async function renderGraph() {
  const canvas = el('graphCanvas');
  canvas.innerHTML = '<div class="empty"><span class="spinner"></span>正在生成图表</div>';
  try {
    const command = {
      knowledge: 'render_knowledge_map',
      plan: 'render_review_plan',
      stage: 'render_stage_guide',
    }[state.graph];
    const svg = await call(command, state.graph === 'knowledge' ? { materialId: null } : {});
    state.graphSvg = svg;
    canvas.innerHTML = svg;
  } catch (error) {
    canvas.innerHTML = '<div class="empty">图表生成失败：' + escapeHtml(String(error)) + '</div>';
  }
}

async function exportGraph() {
  if (!state.graphSvg) { toast('先生成一张图', 'bad'); return; }
  const path = await call('export_svg', { name: state.graph, svg: state.graphSvg });
  toast('已导出：' + path, 'ok');
  await call('reveal_path', { path: path });
}

/* ── 设置 ─────────────────────────────────── */

async function loadSettings() {
  const s = await call('get_settings');
  state.settings = s;
  el('sApiBase').value = s.apiBase || '';
  el('sApiKey').value = s.apiKey || '';
  el('sModel').value = s.model || '';
  el('sProxyUrl').value = s.proxyUrl || '';
  el('sNoProxy').value = s.noProxy || '';
  setProxyMode(s.proxyMode || 'auto');
  el('dataDir').textContent = (state.info && state.info.dataDir) || '';
  await loadAutostart();
}

/* 开机自启：状态以注册表为准，切换后立刻落盘。 */
async function loadAutostart() {
  try {
    setAutostartSeg(await call('autostart_on'));
  } catch (error) {
    setAutostartSeg(false);
  }
}

function setAutostartSeg(on) {
  document.querySelectorAll('#autostartSeg .seg-item').forEach((button) => {
    button.classList.toggle('is-active', (button.dataset.auto === 'on') === !!on);
  });
}

function setProxyMode(mode) {
  document.querySelectorAll('#proxyMode .seg-item').forEach((button) => {
    button.classList.toggle('is-active', button.dataset.mode === mode);
  });
}

function currentProxyMode() {
  const active = document.querySelector('#proxyMode .seg-item.is-active');
  return active ? active.dataset.mode : 'auto';
}

function collectSettings() {
  return {
    apiBase: el('sApiBase').value.trim(),
    apiKey: el('sApiKey').value.trim(),
    model: el('sModel').value.trim(),
    proxyMode: currentProxyMode(),
    proxyUrl: el('sProxyUrl').value.trim(),
    noProxy: el('sNoProxy').value.trim(),
  };
}

async function saveSettings(silent) {
  const info = await call('save_settings', { settings: collectSettings() });
  state.info = info;
  renderChrome();
  if (!silent) {
    el('saveHint').textContent = '已保存 · ' + new Date().toLocaleTimeString();
    toast('设置已保存，接口已重建', 'ok');
  }
  return info;
}

function showResult(id, result) {
  const node = el(id);
  node.hidden = false;
  node.className = 'result' + (result.ok ? ' is-ok' : ' is-bad');
  node.textContent = result.message + (result.detail ? '\n' + result.detail : '');
}

async function detectProxy() {
  const info = await call('detect_proxy');
  const node = el('proxyResult');
  node.hidden = false;
  node.className = 'result' + (info.effective ? ' is-ok' : '');
  node.textContent = [
    '环境变量代理：' + (info.envProxy || '无'),
    'Windows 系统代理：' + (info.systemProxy || '未启用'),
    '本地代理端口：' + (info.openPorts.length
      ? info.openPorts.map((port) => '127.0.0.1:' + port).join('、')
      : '未发现'),
    '当前实际使用：' + (info.effective || '直连'),
  ].join('\n');
  if (!el('sProxyUrl').value.trim() && info.suggested) {
    el('sProxyUrl').value = info.suggested;
    toast('已填入检测到的代理地址，点保存生效', 'ok');
  }
}

/* ── 事件绑定 ─────────────────────────────── */

function bind() {
  el('nav').addEventListener('click', (event) => {
    const button = event.target.closest('.nav-item');
    if (button) switchView(button.dataset.view);
  });

  document.addEventListener('click', async (event) => {
    const row = event.target.closest('[data-card]');
    const learnRow = event.target.closest('[data-learn]');
    const materialAct = event.target.closest('[data-act]');
    const action = event.target.closest('[data-action]');
    const graphTab = event.target.closest('[data-graph]');

    if (row && !event.target.closest('button')) { loadExamCard(Number(row.dataset.card), row.dataset.mode); return; }
    if (learnRow) {
      const idx = (state.queue.fresh || []).findIndex((c) => c.id === Number(learnRow.dataset.learn));
      if (idx >= 0) { state.learnIndex = idx; renderLearn(); }
      return;
    }
    if (materialAct) {
      const id = Number(materialAct.dataset.id);
      const act = materialAct.dataset.act;
      if (act === 'del') {
        if (!confirm('删除这份材料及其全部学习单元？')) return;
        await call('delete_material', { materialId: id });
        toast('已删除', 'ok');
        await refresh();
      } else if (act === 'rename') {
        state.renaming = id;
        renderMaterials();
      } else if (act === 'polish') {
        state.importing = { label: '正在补考点…', source: '单元已切分，只差考点' };
        renderMaterials();
        const result = await polishUnits(id);
        state.importing = null;
        toast(result.error
          ? '补考点失败：' + result.error
          : '已补 ' + result.polished + ' 份考点', result.error ? 'bad' : 'ok');
        await refresh();
      } else if (act === 'export') {
        await withBusy(materialAct, '导出中', async () => {
          const path = await call('export_material', { materialId: id });
          toast('已导出：' + path, 'ok');
          await call('reveal_path', { path: path });
        });
      }
      return;
    }
    if (graphTab) {
      state.graph = graphTab.dataset.graph;
      document.querySelectorAll('.tab').forEach((t) => t.classList.toggle('is-active', t === graphTab));
      renderGraph();
      return;
    }
    if (action) {
      const name = action.dataset.action;
      if (name === 'review-due' || name === 'learn-first') {
        const first = name === 'review-due' ? (state.queue.due || [])[0] : (state.queue.fresh || [])[0];
        if (first) loadExamCard(first.id, 'exam');
      } else if (name === 'go-import') {
        switchView('import');
      } else if (name === 'learn-next') {
        state.learnIndex = Math.min(state.learnIndex + 1, (state.queue.fresh || []).length - 1);
        renderLearn();
      } else if (name === 'plan-study') {
        const planId = Number(action.dataset.id);
        const current = await call('plan_today', { planId: planId });
        const first = current && current.dayUnits.find((card) => !card.learned);
        switchView('learn');
        if (first) {
          const at = (state.queue.fresh || []).findIndex((card) => card.id === first.id);
          if (at >= 0) state.learnIndex = at;
          renderLearn();
        }
      } else if (name === 'plan-delete') {
        await call('delete_plan', { planId: Number(action.dataset.id) });
        toast('计划已删除', 'ok');
        await refresh();
      } else if (name === 'learn-to-exam') {
        const id = Number(action.dataset.id);
        await call('mark_learned', { cardId: id, learned: true });
        await refresh();
        loadExamCard(id, 'exam');
      }
    }
  });

  const runSmartImport = (button) => withBusy(button, '导入中', async () => {
    const query = el('urlInput').value.trim();
    if (!query) { toast('先粘贴链接，或者写一个想学的主题', 'bad'); return; }
    state.importing = {
      label: query.startsWith('http') ? '正在抓取网页…' : '正在搜索资料…',
      source: query,
    };
    renderMaterials();
    try {
      const item = await call('ingest_query', { text: query });
      el('urlInput').value = '';
      await importItem(item);
    } catch (error) {
      state.importing = null;
      renderMaterials();
      toast('导入失败。如果这个站点需要代理，去「设置」页配置代理后重试。', 'bad');
    }
  });

  el('urlBtn').addEventListener('click', (event) => runSmartImport(event.currentTarget));
  el('urlInput').addEventListener('keydown', (event) => {
    if (event.key === 'Enter') { event.preventDefault(); runSmartImport(el('urlBtn')); }
  });

  el('textBtn').addEventListener('click', (event) => withBusy(event.currentTarget, '处理中', async () => {
    const body = el('textInput').value;
    const title = el('textTitle').value.trim() || '粘贴文本 ' + today();
    state.importing = { label: '正在切分学习单元…', source: title };
    renderMaterials();
    try {
      const item = await call('ingest_text', { title: title, text: body });
      el('textInput').value = '';
      await importItem(item);
    } catch (error) {
      state.importing = null;
      renderMaterials();
      toast('导入失败：' + String(error), 'bad');
    }
  }));

  const dropzone = el('dropzone');
  const fileInput = el('fileInput');
  dropzone.addEventListener('click', () => fileInput.click());
  dropzone.addEventListener('dragover', (event) => { event.preventDefault(); dropzone.classList.add('is-over'); });
  dropzone.addEventListener('dragleave', () => dropzone.classList.remove('is-over'));
  dropzone.addEventListener('drop', (event) => {
    event.preventDefault();
    dropzone.classList.remove('is-over');
    if (event.dataTransfer.files.length) handleFiles(event.dataTransfer.files);
  });
  fileInput.addEventListener('change', () => {
    if (fileInput.files.length) handleFiles(fileInput.files);
  });

  el('learnQueue').addEventListener('click', (event) => {
    const header = event.target.closest('.unit-group-title');
    if (!header) return;
    const name = header.dataset.section;
    const at = state.collapsed.indexOf(name);
    if (at >= 0) state.collapsed.splice(at, 1); else state.collapsed.push(name);
    renderLearn();
  });

  el('learnMaterial').addEventListener('change', (event) => {
    state.learnMaterial = Number(event.target.value) || null;
    state.learnIndex = 0;
    renderLearn();
  });

  el('learnSearch').addEventListener('input', (event) => {
    state.learnQuery = event.target.value;
    state.learnIndex = 0;
    renderLearn();
  });

  el('gradeBtn').addEventListener('click', submitGrade);
  el('rateGrid').addEventListener('click', (event) => {
    const button = event.target.closest('[data-rating]');
    if (button) submitSelfRating(Number(button.dataset.rating));
  });
  el('exportBtn').addEventListener('click', exportGraph);

  el('saveSettingsBtn').addEventListener('click', (event) =>
    withBusy(event.currentTarget, '保存中', () => saveSettings(false)));

  el('detectBtn').addEventListener('click', (event) =>
    withBusy(event.currentTarget, '检测中', detectProxy));

  document.querySelectorAll('[data-reveal]').forEach((button) => {
    button.addEventListener('click', () => {
      const input = el(button.dataset.reveal);
      const hidden = input.type === 'password';
      input.type = hidden ? 'text' : 'password';
      button.textContent = hidden ? '隐藏' : '显示';
    });
  });

  el('proxyMode').addEventListener('click', (event) => {
    const button = event.target.closest('.seg-item');
    if (button) setProxyMode(button.dataset.mode);
  });

  el('autostartSeg').addEventListener('click', async (event) => {
    const button = event.target.closest('.seg-item');
    if (!button) return;
    const want = button.dataset.auto === 'on';
    try {
      const on = await call('set_autostart', { enabled: want });
      setAutostartSeg(on);
      toast(on ? '已设为开机自启' : '已关闭开机自启', 'ok');
    } catch (error) {
      toast('设置失败：' + String(error), 'bad');
      await loadAutostart();
    }
  });

  el('testLlmBtn').addEventListener('click', (event) =>
    withBusy(event.currentTarget, '测试中', async () => {
      await saveSettings(true);
      showResult('llmResult', await call('test_llm'));
    }));

  el('testProxyBtn').addEventListener('click', (event) =>
    withBusy(event.currentTarget, '测试中', async () => {
      await saveSettings(true);
      showResult('proxyTestResult', await call('test_fetch', { url: 'https://www.google.com/' }));
    }));

  document.addEventListener('keydown', (event) => {
    if ((event.ctrlKey || event.metaKey) && event.key === 'Enter') {
      event.preventDefault();
      if (state.view === 'exam') submitGrade();
    }
  });
}

bind();

refresh().catch((error) => {
  el('todayFocus').innerHTML =
    '<div class="empty">初始化失败：' + escapeHtml(String(error)) + '</div>';
});

async function handleFiles(files) {
  const file = files[0];
  if (!file) return;
  state.importing = { label: '正在解析文件…', source: file.name };
  renderMaterials();
  try {
    const buffer = new Uint8Array(await file.arrayBuffer());
    const item = await call('ingest_bytes', {
      name: file.name, bytes: Array.from(buffer),
    });
    await importItem(item);
  } catch (error) {
    state.importing = null;
    renderMaterials();
    toast('导入失败：' + String(error), 'bad');
  }
}