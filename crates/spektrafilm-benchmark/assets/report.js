
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
function selectReviewRegion(pair, region) {
  const regions = [...pair.querySelectorAll('details[data-review-region]')];
  if (!regions.length && !pair.querySelector('[data-review-full]')) return;
  pair.querySelectorAll('[data-review-full]').forEach(full => { full.hidden = Boolean(region); });
  pair.querySelectorAll('[data-review-difference]').forEach(difference => { difference.hidden = true; });
  pair.querySelectorAll('[data-difference-toggle]').forEach(button => {
    button.setAttribute('aria-pressed', 'false');
    button.setAttribute('aria-expanded', 'false');
  });
  pair.querySelectorAll('[data-review-reset]').forEach(button => {
    button.setAttribute('aria-pressed', String(!region));
  });
  regions.forEach(detail => {
    const selected = detail === region;
    detail.hidden = !selected;
    detail.open = selected;
    const disclosure = detail.previousElementSibling;
    if (disclosure && disclosure.classList.contains('disclosure-button')) {
      disclosure.hidden = !selected;
      disclosure.setAttribute('aria-expanded', String(selected));
    }
  });
  pair.querySelectorAll('[data-crop-target]').forEach(button => {
    const selected = Boolean(region && button.dataset.cropTarget === region.dataset.regionId);
    button.setAttribute('aria-pressed', String(selected));
    button.setAttribute('aria-expanded', String(selected));
  });
  pair.querySelectorAll('[data-region-locator]').forEach(locator => {
    const selected = locator.closest('details[data-region-id]') === region;
    if (selected) locator.setAttribute('aria-current', 'true');
    else locator.removeAttribute('aria-current');
  });
  if (region) {
    region.querySelectorAll('[data-zoom]').forEach(zoom => { zoom.value = '1'; });
    region.querySelectorAll('.viewport').forEach(pane => { pane.scrollLeft = 0; pane.scrollTop = 0; });
  }
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
      ? identityMatch([...pair.querySelectorAll('details[data-region-id]')], item => item.dataset.regionId === values.get('region'), '区域')
      : null;
    if (values && values.has('region') && !region) throw new Error('链接中的区域不存在');
    navigationError.hidden = true;
    navigationError.textContent = '';
    casePanels.forEach(item => { item.hidden = item !== panel; });
    caseNav.value = panel.dataset.casePanel;
    pairs.forEach(item => { item.hidden = item !== pair; });
    const pairNav = panel.querySelector('[data-pair-nav]');
    if (pairNav) pairNav.value = pair.dataset.pairPanel;
    selectReviewRegion(pair, region);
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
document.querySelectorAll('[data-crop-target], [data-review-reset]').forEach(button => {
  button.setAttribute('aria-label', button.hasAttribute('data-review-reset') ? '返回全图对照' : `查看区域：${button.textContent.trim()}`);
  button.addEventListener('click', () => {
    const pair = button.closest('[data-pair-panel]');
    const panel = pair.closest('[data-case-panel]');
    const region = button.hasAttribute('data-review-reset') ? null
      : [...pair.querySelectorAll('details[data-region-id]')].find(detail => detail.dataset.regionId === button.dataset.cropTarget);
    if (button.hasAttribute('data-crop-target') && !region) return;
    const hash = fragment(panel, pair, region);
    if (location.hash === hash) navigate();
    else location.hash = hash;
  });
});
document.querySelectorAll('[data-difference-toggle]').forEach(button => {
  const pair = button.closest('[data-pair-panel]');
  const difference = pair.querySelector('[data-review-difference]');
  button.setAttribute('aria-label', '切换差异图；数值单位、缩放与裁切见图下说明');
  button.setAttribute('aria-pressed', 'false');
  button.setAttribute('aria-expanded', 'false');
  button.disabled = button.disabled || !difference || !difference.querySelector('img');
  button.addEventListener('click', () => {
    const active = button.getAttribute('aria-pressed') !== 'true';
    selectReviewRegion(pair, null);
    difference.hidden = !active;
    pair.querySelectorAll('[data-review-full]').forEach(full => { full.hidden = active; });
    pair.querySelectorAll('[data-review-reset]').forEach(reset => { reset.setAttribute('aria-pressed', String(!active)); });
    button.setAttribute('aria-pressed', String(active));
    button.setAttribute('aria-expanded', String(active));
    // Difference is a display mode of the pair; keep Back able to restore a crop.
    const hash = fragment(pair.closest('[data-case-panel]'), pair);
    if (location.hash !== hash) {
      try {
        history.pushState(null, '', hash);
      } catch (_) {
        location.hash = hash;
      }
    }
    difference.querySelectorAll('img').forEach(image => { image.loading = 'eager'; });
    refreshViewers();
  });
});
document.querySelectorAll('[data-crop-filter]').forEach(button => {
  button.setAttribute('aria-label', `筛选区域：${button.textContent.trim()}`);
  button.addEventListener('click', () => {
    const pair = button.closest('[data-pair-panel]');
    pair.querySelectorAll('[data-crop-filter]').forEach(filter => {
      filter.setAttribute('aria-pressed', String(filter === button));
    });
    pair.querySelectorAll('[data-crop-target]').forEach(crop => {
      crop.hidden = button.dataset.cropFilter !== 'all' && crop.dataset.cropKind !== button.dataset.cropFilter;
    });
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
  splitHandle.setAttribute('aria-label', '拖动 A/B 分割线；方向键微调，Shift 加大步幅');
  splitHandle.setAttribute('aria-valuemin', '0');
  splitHandle.setAttribute('aria-valuemax', '100');
  const splitCandidates = [...viewer.querySelectorAll('.split .candidate')];
  const aligned = viewer.dataset.aligned !== 'false';
  const modeSelect = viewer.querySelector('[data-mode]');
  const alternate = viewer.querySelector('[data-alternate]');
  const modeButtons = [...viewer.querySelectorAll('[data-view-mode]')];
  const fullscreen = viewer.querySelector('[data-fullscreen]');
  viewer.tabIndex = 0;
  viewer.setAttribute('aria-label', aligned ? '图像 A/B 对照；空格切换 A/B' : '未对齐图像左右对照');
  zoom.setAttribute('aria-label', '图像显示倍率');
  modeSelect.setAttribute('aria-label', '图像查看方式');
  alternate.setAttribute('aria-label', '切换 A/B 图像');
  if (!aligned) {
    [...modeSelect.options].forEach(option => { option.disabled = option.value !== 'side'; });
    alternate.disabled = true;
    modeButtons.forEach(button => { button.disabled = true; });
    splitRange.disabled = true;
    splitHandle.disabled = true;
  }
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
      ? '适应容器 · 仅调整显示'
      : `${Number(zoom.value) * 100}% · 每个源像素对应 ${Number(zoom.value)} 个设备像素 · 最近邻显示`;
  }
  images.forEach(img => img.addEventListener('load', resize));
  zoom.addEventListener('change', resize);
  window.addEventListener('resize', resize);
  function setMode(mode) {
    viewer.dataset.mode = aligned ? mode : 'side';
    modeSelect.value = viewer.dataset.mode;
    modeButtons.forEach(button => {
      button.setAttribute('aria-label', `显示 ${button.textContent.trim()}`);
      button.setAttribute('aria-pressed', String(button.dataset.viewMode === viewer.dataset.mode));
    });
    resize();
  }
  modeSelect.addEventListener('change', event => { setMode(event.target.value); });
  modeButtons.forEach(button => { button.addEventListener('click', () => setMode(button.dataset.viewMode)); });
  function setSplit(value) {
    const position = Math.round(Math.max(0, Math.min(100, Number(value))));
    splitRange.value = String(position);
    splitCandidates.forEach(img => { img.style.clipPath = `inset(0 0 0 ${position}%)`; });
    splitHandle.style.left = `${position}%`;
    splitHandle.setAttribute('aria-valuenow', String(position));
    splitRange.setAttribute('aria-valuenow', String(position));
    splitHandle.setAttribute('aria-valuetext', `${Math.round(position)}%`);
  }
  function dragSplit(event) {
    const bounds = splitCanvas.getBoundingClientRect();
    if (bounds.width) setSplit((event.clientX - bounds.left) / bounds.width * 100);
  }
  splitRange.addEventListener('input', event => { setSplit(event.target.value); });
  splitHandle.addEventListener('pointerdown', event => {
    event.stopPropagation();
    if (!aligned || event.button !== 0 || splitPointer !== null) return;
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
  splitHandle.addEventListener('lostpointercancel', finishSplitDrag);
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
  function alternateImages() {
    if (aligned) setMode(viewer.dataset.mode === 'baseline' ? 'candidate' : 'baseline');
  }
  alternate.addEventListener('click', alternateImages);
  viewer.addEventListener('keydown', event => {
    if (event.code !== 'Space' || event.repeat || event.ctrlKey || event.metaKey || event.altKey) return;
    if (event.target.closest('button, input, select, textarea, a, [contenteditable]')) return;
    if (!aligned) return;
    event.preventDefault();
    alternateImages();
  });
  if (fullscreen) {
    fullscreen.setAttribute('aria-label', '切换图像全屏');
    fullscreen.setAttribute('aria-pressed', 'false');
    fullscreen.setAttribute('aria-expanded', 'false');
    fullscreen.disabled = !viewer.requestFullscreen || !document.fullscreenEnabled;
    fullscreen.addEventListener('click', async () => {
      try {
        if (document.fullscreenElement === viewer) await document.exitFullscreen();
        else await viewer.requestFullscreen();
      } catch (_) {
        viewer.querySelector('[data-scale-label]').textContent = '此环境无法进入全屏';
      }
    });
    document.addEventListener('fullscreenchange', () => {
      const active = document.fullscreenElement === viewer;
      fullscreen.setAttribute('aria-pressed', String(active));
      fullscreen.setAttribute('aria-expanded', String(active));
      fullscreen.textContent = active ? '退出全屏' : '全屏';
      resize();
    });
  }
  panes.forEach(pane => {
    pane.tabIndex = 0;
    pane.setAttribute('aria-label', '图像视口；可拖动或使用方向键滚动，空格切换 A/B');
    pane.addEventListener('dragstart', event => { event.preventDefault(); });
    pane.addEventListener('scroll', () => {
      if (syncing || viewer.dataset.aligned === 'false') return;
      syncing = true;
      panes.forEach(other => { if (other !== pane) { other.scrollLeft = pane.scrollLeft; other.scrollTop = pane.scrollTop; } });
      requestAnimationFrame(() => { syncing = false; });
    });
    pane.addEventListener('pointerdown', event => {
      if (event.button !== 0 || panStart || event.target.closest('button, input, select, a')) return;
      event.preventDefault();
      pane.focus({preventScroll:true});
      panStart = {pane, pointer:event.pointerId, x:event.clientX, y:event.clientY, left:pane.scrollLeft, top:pane.scrollTop};
      pane.setPointerCapture(event.pointerId);
    });
    pane.addEventListener('pointermove', event => {
      if (!panStart || panStart.pane !== pane || panStart.pointer !== event.pointerId) return;
      pane.scrollLeft = panStart.left - (event.clientX - panStart.x);
      pane.scrollTop = panStart.top - (event.clientY - panStart.y);
    });
    function finishPan(event) {
      if (!panStart || panStart.pane !== pane || panStart.pointer !== event.pointerId) return;
      panStart = null;
      if (pane.hasPointerCapture(event.pointerId)) pane.releasePointerCapture(event.pointerId);
    }
    pane.addEventListener('pointerup', finishPan);
    pane.addEventListener('pointercancel', finishPan);
    pane.addEventListener('lostpointercapture', finishPan);
  });
  setMode(viewer.dataset.mode);
});
if (casePanels.length) navigate();
