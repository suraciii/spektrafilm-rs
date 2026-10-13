
'use strict';
const caseNav = document.querySelector('[data-case-nav]');
const casePanels = [...document.querySelectorAll('[data-case-panel]')];
const navigationError = document.querySelector('[data-navigation-error]');
let navigationRevision = 0;
function refreshViewers() {
  requestAnimationFrame(() => window.dispatchEvent(new Event('resize')));
}
function fragment(panel, pair = null, region = null) {
  let hash = `#case=${encodeURIComponent(panel.dataset.caseId)}&seed=${encodeURIComponent(panel.dataset.seed)}`;
  if (pair) hash += `&pair=${encodeURIComponent(pair.dataset.pairId)}`;
  if (region) hash += `&region=${encodeURIComponent(region.dataset.regionId)}`;
  return hash;
}
// An explicit button exposes every disclosure in accessibility snapshots, including
// browsers that omit native summary controls. Native details still owns visibility.
document.querySelectorAll('details').forEach((detail, index) => {
  const summary = detail.querySelector(':scope > summary');
  if (!summary) return;
  detail.id = `disclosure-${index}`;
  const button = document.createElement('button');
  button.type = 'button';
  button.className = 'disclosure-button';
  button.textContent = summary.textContent;
  button.setAttribute('aria-controls', detail.id);
  button.setAttribute('aria-expanded', String(detail.open));
  detail.before(button);
  summary.hidden = true;
  button.addEventListener('click', () => {
    detail.open = !detail.open;
    button.setAttribute('aria-expanded', String(detail.open));
    if (detail.dataset.regionId !== undefined) {
      const panel = detail.closest('[data-case-panel]');
      const pair = detail.closest('[data-pair-panel]');
      location.hash = fragment(panel, pair, detail.open ? detail : null);
    }
    refreshViewers();
  });
  detail.addEventListener('toggle', () => {
    button.setAttribute('aria-expanded', String(detail.open));
    if (detail.open) refreshViewers();
  });
});
function parseFragment(hash) {
  const values = new Map();
  for (const field of hash.slice(1).split('&')) {
    const parts = field.split('=');
    if (parts.length !== 2) throw new Error('链接参数格式无效');
    const key = decodeURIComponent(parts[0]);
    const value = decodeURIComponent(parts[1]);
    if (!['case', 'seed', 'pair', 'region'].includes(key) || values.has(key) || !value) {
      throw new Error('链接包含未知、重复或空参数');
    }
    values.set(key, value);
  }
  if (!values.has('case') || !values.has('seed') || (values.has('region') && !values.has('pair'))) {
    throw new Error('链接需要案例、种子；区域链接还需要比较对象');
  }
  return values;
}
function identityMatch(items, predicate, label) {
  const matches = items.filter(predicate);
  if (matches.length > 1) throw new Error(`链接中的${label}身份不唯一`);
  return matches[0];
}
async function navigate() {
  const revision = ++navigationRevision;
  try {
    const values = location.hash ? parseFragment(location.hash) : null;
    const panel = values
      ? identityMatch(casePanels, item => item.dataset.caseId === values.get('case') && item.dataset.seed === values.get('seed'), '案例或种子')
      : casePanels[0];
    if (!panel) throw new Error('链接中的案例或种子不存在');
    const pairs = [...panel.querySelectorAll('[data-pair-panel]')];
    const pair = values && values.has('pair')
      ? identityMatch(pairs, item => item.dataset.pairId === values.get('pair'), '比较对象')
      : identityMatch(pairs, item => item.dataset.defaultPair === 'true', '默认比较对象');
    if (!pair) throw new Error('链接中的比较对象不存在');
    const region = values && values.has('region')
      ? identityMatch([...pair.querySelectorAll('[data-region-id]')], item => item.dataset.regionId === values.get('region'), '区域')
      : null;
    if (values && values.has('region') && !region) throw new Error('链接中的区域不存在');
    navigationError.hidden = true;
    navigationError.textContent = '';
    casePanels.forEach(item => { item.hidden = item !== panel; });
    caseNav.value = panel.dataset.casePanel;
    pairs.forEach(item => { item.hidden = item !== pair; });
    const pairNav = panel.querySelector('[data-pair-nav]');
    if (pairNav) pairNav.value = pair.dataset.pairPanel;
    const target = region || (values && values.has('pair') ? pair : panel);
    for (let ancestor = target; ancestor && ancestor !== panel; ancestor = ancestor.parentElement) {
      if (ancestor.tagName === 'DETAILS') {
        ancestor.open = true;
        ancestor.previousElementSibling.setAttribute('aria-expanded', 'true');
      }
    }
    const scrollTarget = region ? region.previousElementSibling : target;
    if (location.hash) scrollTarget.scrollIntoView({block:'start'});
    // Start only displayed images eagerly: hidden disclosures and inactive panes
    // must not delay navigation. Repeat after resize switches preview/native URLs.
    for (let pass = 0; pass < 2; pass += 1) {
      if (revision !== navigationRevision) return;
      window.dispatchEvent(new Event('resize'));
      const images = [...target.querySelectorAll('img')].filter(image => {
        if (!image.getClientRects().length) return false;
        for (let ancestor = image; ancestor; ancestor = ancestor.parentElement) {
          const style = getComputedStyle(ancestor);
          if (style.display === 'none' || style.visibility === 'hidden') return false;
        }
        return true;
      });
      await Promise.all(images.map(image => {
        image.loading = 'eager';
        if (image.complete) return Promise.resolve();
        return new Promise(resolve => {
          const finished = () => {
            image.removeEventListener('load', finished);
            image.removeEventListener('error', finished);
            resolve();
          };
          image.addEventListener('load', finished);
          image.addEventListener('error', finished);
          if (image.complete) finished();
        });
      }));
      await new Promise(resolve => requestAnimationFrame(resolve));
    }
    if (revision === navigationRevision && location.hash) scrollTarget.scrollIntoView({block:'start'});
  } catch (error) {
    casePanels.forEach(panel => { panel.hidden = true; });
    navigationError.hidden = false;
    navigationError.textContent = `无法打开报告链接：${error instanceof URIError ? 'URL 编码无效' : error.message}。请用案例选择器重新选择。`;
    if (casePanels.length) {
      const recovery = document.createElement('a');
      recovery.href = fragment(casePanels[0]);
      recovery.textContent = '打开首个案例';
      navigationError.append(' ', recovery);
    }
  }
}
if (caseNav) {
  caseNav.addEventListener('change', () => {
    const panel = casePanels.find(item => item.dataset.casePanel === caseNav.value);
    location.hash = fragment(panel);
  });
}
document.querySelectorAll('[data-pair-nav]').forEach(nav => {
  nav.addEventListener('change', () => {
    const panel = nav.closest('[data-case-panel]');
    const pair = [...panel.querySelectorAll('[data-pair-panel]')].find(item => item.dataset.pairPanel === nav.value);
    location.hash = fragment(panel, pair);
  });
});
document.querySelectorAll('.navigation-links a').forEach(link => {
  link.addEventListener('click', event => {
    if (event.button !== 0 || event.ctrlKey || event.metaKey || event.shiftKey || event.altKey) return;
    if (link.hash === location.hash) {
      event.preventDefault();
      navigate();
    }
  });
});
document.querySelectorAll('[data-copy-link]').forEach(button => {
  button.addEventListener('click', async () => {
    const link = new URL(button.dataset.copyLink, location.href).href;
    const status = document.querySelector('[data-copy-status]');
    try {
      if (navigator.clipboard && window.isSecureContext) {
        await navigator.clipboard.writeText(link);
      } else {
        const input = document.createElement('textarea');
        input.value = link;
        input.setAttribute('aria-label', '报告链接');
        document.body.append(input);
        input.select();
        const copied = document.execCommand('copy');
        input.remove();
        button.focus({preventScroll:true});
        if (!copied) throw new Error('clipboard unavailable');
      }
      status.textContent = '报告链接已复制';
    } catch (_) {
      status.textContent = `无法自动复制；请复制此链接：${link}`;
    }
  });
});
window.addEventListener('hashchange', navigate);
document.querySelectorAll('.viewer').forEach(viewer => {
  const panes = [...viewer.querySelectorAll('.viewport')];
  const images = [...viewer.querySelectorAll('.canvas img')];
  const zoom = viewer.querySelector('[data-zoom]');
  const splitRange = viewer.querySelector('[data-split]');
  const splitCanvas = viewer.querySelector('.canvas.split');
  const splitHandle = viewer.querySelector('[data-split-handle]');
  const splitCandidates = [...viewer.querySelectorAll('.split .candidate')];
  let splitPointer = null;
  let syncing = false;
  let panStart = null;
  function resize() {
    const dpr = window.devicePixelRatio || 1;
    const factor = zoom.value === 'fit' ? null : Number(zoom.value) / dpr;
    viewer.dataset.zoomMode = zoom.value === 'fit' ? 'fit' : 'native';
    images.forEach(img => {
      const desired = factor === null ? img.dataset.fit : img.dataset.native;
      if (desired && img.getAttribute('src') !== desired) {
        img.src = desired;
        return;
      }
      if (!img.naturalWidth) return;
      const pane = img.closest('.viewport');
      if (!pane.clientWidth || !pane.clientHeight) return;
      const scale = factor === null ? Math.min(1, (pane.clientWidth - 2) / img.naturalWidth, (pane.clientHeight - 2) / img.naturalHeight) : factor;
      img.style.width = `${img.naturalWidth * scale}px`;
      img.style.height = `${img.naturalHeight * scale}px`;
    });
    viewer.querySelector('[data-scale-label]').textContent = factor === null
      ? 'Fit to container (display only)'
      : `${Number(zoom.value) * 100}% · ${Number(zoom.value)} device pixels per source pixel · nearest neighbor`;
  }
  images.forEach(img => img.addEventListener('load', resize));
  zoom.addEventListener('change', resize);
  window.addEventListener('resize', resize);
  viewer.querySelector('[data-mode]').addEventListener('change', event => {
    viewer.dataset.mode = event.target.value;
    resize();
  });
  function setSplit(value) {
    const position = Math.round(Math.max(0, Math.min(100, Number(value))));
    splitRange.value = String(position);
    splitCandidates.forEach(img => { img.style.clipPath = `inset(0 0 0 ${position}%)`; });
    splitHandle.style.left = `${position}%`;
    splitHandle.setAttribute('aria-valuenow', String(position));
    splitHandle.setAttribute('aria-valuetext', `${Math.round(position)}%`);
  }
  function dragSplit(event) {
    const bounds = splitCanvas.getBoundingClientRect();
    if (bounds.width) setSplit((event.clientX - bounds.left) / bounds.width * 100);
  }
  splitRange.addEventListener('input', event => { setSplit(event.target.value); });
  splitHandle.addEventListener('pointerdown', event => {
    event.stopPropagation();
    if (event.button !== 0 || splitPointer !== null) return;
    event.preventDefault();
    splitHandle.focus({preventScroll:true});
    splitPointer = event.pointerId;
    splitHandle.setPointerCapture(event.pointerId);
    dragSplit(event);
  });
  splitHandle.addEventListener('pointermove', event => {
    if (event.pointerId !== splitPointer) return;
    event.preventDefault();
    event.stopPropagation();
    dragSplit(event);
  });
  function finishSplitDrag(event) {
    if (event.pointerId !== splitPointer) return;
    event.stopPropagation();
    splitPointer = null;
    if (splitHandle.hasPointerCapture(event.pointerId)) splitHandle.releasePointerCapture(event.pointerId);
  }
  splitHandle.addEventListener('pointerup', finishSplitDrag);
  splitHandle.addEventListener('pointercancel', finishSplitDrag);
  splitHandle.addEventListener('lostpointercapture', finishSplitDrag);
  splitHandle.addEventListener('keydown', event => {
    const step = event.shiftKey ? 10 : 1;
    let position = Number(splitRange.value);
    switch (event.key) {
      case 'ArrowLeft': case 'ArrowDown': position -= step; break;
      case 'ArrowRight': case 'ArrowUp': position += step; break;
      case 'Home': position = 0; break;
      case 'End': position = 100; break;
      default: return;
    }
    event.preventDefault();
    event.stopPropagation();
    setSplit(position);
  });
  setSplit(splitRange.value);
  viewer.querySelector('[data-alternate]').addEventListener('click', () => {
    const mode = viewer.dataset.mode === 'baseline' ? 'candidate' : 'baseline';
    viewer.dataset.mode = mode;
    viewer.querySelector('[data-mode]').value = mode;
    resize();
  });
  panes.forEach(pane => {
    pane.addEventListener('scroll', () => {
      if (syncing || viewer.dataset.aligned === 'false') return;
      syncing = true;
      panes.forEach(other => { if (other !== pane) { other.scrollLeft = pane.scrollLeft; other.scrollTop = pane.scrollTop; } });
      requestAnimationFrame(() => { syncing = false; });
    });
    pane.addEventListener('pointerdown', event => { panStart = {pane, x:event.clientX, y:event.clientY, left:pane.scrollLeft, top:pane.scrollTop}; pane.setPointerCapture(event.pointerId); });
    pane.addEventListener('pointermove', event => { if (!panStart || panStart.pane !== pane) return; pane.scrollLeft = panStart.left - (event.clientX - panStart.x); pane.scrollTop = panStart.top - (event.clientY - panStart.y); });
    pane.addEventListener('pointerup', () => { panStart = null; });
    pane.addEventListener('pointercancel', () => { panStart = null; });
  });
  resize();
});
if (casePanels.length) navigate();
