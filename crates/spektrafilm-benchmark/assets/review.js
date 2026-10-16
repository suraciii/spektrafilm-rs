(function () {
  'use strict';

  // ---------- 报告数据（内嵌 JSON，禁止网络请求） ----------
  const dataElement = document.getElementById('review-data');
  const hashError = document.getElementById('chart-hash-error');
  function fatal(message) {
    hashError.hidden = false;
    hashError.textContent = `报告数据不可用：${message}。本页面无法展示图像对照。`;
  }
  if (!dataElement) {
    fatal('未找到内嵌数据节点 review-data');
    return;
  }
  let data = null;
  try {
    data = JSON.parse(dataElement.textContent);
  } catch (error) {
    fatal(`内嵌 JSON 解析失败（${error.message}）`);
    return;
  }
  if (!data || !Array.isArray(data.images) || data.images.length < 2 || !Array.isArray(data.charts)) {
    fatal('内嵌数据缺少 charts 或 images（图像至少两个）');
    return;
  }

  // ---------- 几何与语义工具 ----------
  const charts = new Map(data.charts.map(chart => [chart.id, chart]));
  const images = data.images;
  const imagesById = new Map(images.map(image => [image.id, image]));
  const patchCache = new Map();

  function columnLabel(column) {
    let label = '';
    let n = column;
    while (n > 0) {
      const rest = (n - 1) % 26;
      label = String.fromCharCode(65 + rest) + label;
      n = Math.floor((n - 1) / 26);
    }
    return label;
  }
  // 行优先遍历；标签 = Excel 风格列字母 + 从 1 开始的行号（如 A1…I5）。
  function patchList(image) {
    const chart = charts.get(image.chart);
    if (!chart || !chart.grid) return [];
    if (patchCache.has(image.chart)) return patchCache.get(image.chart);
    const grid = chart.grid;
    const list = [];
    for (let row = 1; row <= grid.rows; row += 1) {
      for (let column = 1; column <= grid.columns; column += 1) {
        list.push({
          label: columnLabel(column) + row,
          row,
          column,
          x: grid.origin[0] + (column - 1) * (grid.patch_size[0] + grid.gap[0]),
          y: grid.origin[1] + (row - 1) * (grid.patch_size[1] + grid.gap[1]),
          w: grid.patch_size[0],
          h: grid.patch_size[1],
        });
      }
    }
    patchCache.set(image.chart, list);
    return list;
  }
  function patchFor(image, label) {
    return patchList(image).find(patch => patch.label === label) || null;
  }
  function chartLabel(image) {
    const chart = charts.get(image.chart);
    return chart && chart.label ? chart.label : image.chart;
  }
  function imageById(id) {
    return imagesById.get(id) || null;
  }

  // ---------- 状态 ----------
  const defaults = data.default_pair && imageById(data.default_pair.left) && imageById(data.default_pair.right)
    ? data.default_pair
    : { left: images[0].id, right: images[1].id };
  const state = {
    left: defaults.left,
    right: defaults.right,
    mode: 'whole',
    zoom: 'fit',
    layout: 'side',
    split: 50,
    blink: 'left',
    patch: null,
    patchLeft: null,
    patchRight: null,
  };

  function sameChart() {
    return imageById(state.left).chart === imageById(state.right).chart;
  }
  // 分割/交替只按声明的共同色卡启用；同色卡图像尺寸由生成端保证一致，不在此比较尺寸。
  function sameLayout() {
    return sameChart();
  }
  // 各色块参数只在自己的声明网格内有效；跨色卡不推断同名对应。
  function normalizePatches() {
    const left = imageById(state.left);
    const right = imageById(state.right);
    if (sameChart()) {
      if (!patchFor(left, state.patch)) {
        const list = patchList(left);
        state.patch = list.length ? list[0].label : null;
      }
    } else {
      if (!patchFor(left, state.patchLeft)) {
        const list = patchList(left);
        state.patchLeft = list.length ? list[0].label : null;
      }
      if (!patchFor(right, state.patchRight)) {
        const list = patchList(right);
        state.patchRight = list.length ? list[0].label : null;
      }
    }
  }

  // ---------- DOM 锚点 ----------
  const titleNode = document.getElementById('review-title');
  const descriptionNode = document.getElementById('review-description');
  const viewer = document.querySelector('[data-viewer]');
  const leftSelect = document.querySelector('[data-left-select]');
  const rightSelect = document.querySelector('[data-right-select]');
  const swapButton = document.querySelector('[data-swap]');
  const chartWarning = document.querySelector('[data-chart-warning]');
  const modeButtons = [...document.querySelectorAll('[data-view-mode]')];
  const zoomSelect = document.querySelector('[data-zoom]');
  const zoomLabelNode = document.querySelector('[data-zoom-label]');
  const fullscreenButton = document.querySelector('[data-fullscreen]');
  const wholePanel = document.querySelector('[data-whole-panel]');
  const patchPanel = document.querySelector('[data-patch-panel]');
  const layoutButtons = [...document.querySelectorAll('[data-layout]')];
  const wholeNote = document.querySelector('[data-whole-note]');
  const splitPanel = document.querySelector('[data-split-panel]');
  const splitRange = document.querySelector('[data-split-range]');
  const splitLabel = document.querySelector('[data-split-label]');
  const splitDivider = document.querySelector('[data-split-divider]');
  const blinkPanel = document.querySelector('[data-blink-panel]');
  const blinkButton = document.querySelector('[data-blink]');
  const blinkLabel = document.querySelector('[data-blink-label]');
  const patchSharedControls = document.querySelector('[data-patch-shared-controls]');
  const patchSplitControls = document.querySelector('[data-patch-split-controls]');
  const patchSelect = document.querySelector('[data-patch-select]');
  const patchLeftSelect = document.querySelector('[data-patch-left-select]');
  const patchRightSelect = document.querySelector('[data-patch-right-select]');
  const sheetHint = document.querySelector('[data-sheet-hint]');
  const provenanceBody = document.querySelector('[data-provenance-body]');

  function sideNodes(side) {
    return {
      metaLabel: document.querySelector(`[data-meta-label="${side}"]`),
      metaChart: document.querySelector(`[data-meta-chart="${side}"]`),
      metaDescription: document.querySelector(`[data-meta-description="${side}"]`),
      metaColor: document.querySelector(`[data-meta-color="${side}"]`),
      caption: document.querySelector(`[data-caption="${side}"]`),
      viewport: document.querySelector(`[data-viewport="${side}"]`),
      image: document.querySelector(`[data-image="${side}"]`),
      patchLabel: document.querySelector(`[data-patch-label="${side}"]`),
      patchRect: document.querySelector(`[data-patch-rect="${side}"]`),
      patchViewport: document.querySelector(`[data-patch-viewport="${side}"]`),
      patchCanvas: document.querySelector(`[data-patch-canvas="${side}"]`),
      patchImage: document.querySelector(`[data-patch-image="${side}"]`),
      locatorImage: document.querySelector(`[data-locator-image="${side}"]`),
      locatorRect: document.querySelector(`[data-locator-rect="${side}"]`),
      locatorCaption: document.querySelector(`[data-locator-caption="${side}"]`),
    };
  }
  const nodes = { left: sideNodes('left'), right: sideNodes('right') };
  const splitImages = {
    left: document.querySelector('[data-split-image="left"]'),
    right: document.querySelector('[data-split-image="right"]'),
  };
  const blinkImages = {
    left: document.querySelector('[data-blink-image="left"]'),
    right: document.querySelector('[data-blink-image="right"]'),
  };
  const sheets = {
    shared: document.querySelector('[data-contact-sheet="shared"]'),
    left: document.querySelector('[data-contact-sheet="left"]'),
    right: document.querySelector('[data-contact-sheet="right"]'),
  };

  // ---------- 哈希状态（语义 ID；异常链接只报告、不替换图像） ----------
  const HASH_KEYS = ['left', 'right', 'mode', 'zoom', 'layout', 'split', 'blink', 'patch', 'patch_left', 'patch_right'];
  const MODES = ['whole', 'patch'];
  const ZOOMS = ['fit', 'native', 'x2', 'x4', 'x8', 'x16'];
  const LAYOUTS = ['side', 'split', 'blink'];
  let lastWrittenHash = '';

  function currentHash() {
    const parts = [];
    const push = (key, value) => parts.push(`${key}=${encodeURIComponent(value)}`);
    push('left', state.left);
    push('right', state.right);
    push('mode', state.mode);
    push('zoom', state.zoom);
    if (state.mode === 'whole') {
      push('layout', state.layout);
      if (state.layout === 'split') push('split', String(state.split));
      if (state.layout === 'blink') push('blink', state.blink);
    } else if (sameChart()) {
      if (state.patch) push('patch', state.patch);
    } else {
      if (state.patchLeft) push('patch_left', state.patchLeft);
      if (state.patchRight) push('patch_right', state.patchRight);
    }
    return `#${parts.join('&')}`;
  }
  function syncHash() {
    const hash = currentHash();
    lastWrittenHash = hash;
    if (location.hash === hash) return;
    try {
      history.replaceState(null, '', hash);
    } catch (_) {
      location.hash = hash;
    }
  }
  function parseHash(hash) {
    const errors = [];
    const values = new Map();
    if (hash.length > 1) {
      for (const field of hash.slice(1).split('&')) {
        if (!field) continue;
        const equal = field.indexOf('=');
        const rawKey = equal < 0 ? field : field.slice(0, equal);
        const rawValue = equal < 0 ? '' : field.slice(equal + 1);
        let key = rawKey;
        let value = rawValue;
        try {
          key = decodeURIComponent(rawKey);
          value = decodeURIComponent(rawValue);
        } catch (_) {
          errors.push(`URL 编码无效（“${field}”）`);
          continue;
        }
        if (!HASH_KEYS.includes(key)) {
          errors.push(`未知参数“${key}”`);
          continue;
        }
        if (values.has(key)) {
          errors.push(`参数“${key}”重复`);
          continue;
        }
        if (!value) {
          errors.push(`参数“${key}”缺少值`);
          continue;
        }
        values.set(key, value);
      }
    }
    const next = {
      left: values.get('left') || state.left,
      right: values.get('right') || state.right,
      mode: values.get('mode') || state.mode,
      zoom: values.get('zoom') || state.zoom,
      layout: values.get('layout') || state.layout,
      split: values.has('split') ? values.get('split') : String(state.split),
      blink: values.get('blink') || state.blink,
      patch: values.has('patch') ? values.get('patch') : state.patch,
      patchLeft: values.has('patch_left') ? values.get('patch_left') : state.patchLeft,
      patchRight: values.has('patch_right') ? values.get('patch_right') : state.patchRight,
    };
    for (const key of ['left', 'right']) {
      if (!imageById(next[key])) errors.push(`未知图像 ID“${next[key]}”`);
    }
    if (!MODES.includes(next.mode)) errors.push(`未知显示模式“${next.mode}”`);
    if (!ZOOMS.includes(next.zoom)) errors.push(`未知缩放“${next.zoom}”`);
    if (!LAYOUTS.includes(next.layout)) errors.push(`未知排布方式“${next.layout}”`);
    if (!/^\d+$/.test(next.split) || Number(next.split) > 100) errors.push(`分割位置无效“${next.split}”`);
    if (!['left', 'right'].includes(next.blink)) errors.push(`交替状态无效“${next.blink}”`);
    const left = imageById(next.left);
    const right = imageById(next.right);
    if (left && right) {
      const shared = left.chart === right.chart;
      if (shared) {
        if (values.has('patch_left') || values.has('patch_right')) {
          errors.push('参数 patch_left/patch_right 仅适用于不同色卡的图像组合');
        }
        if (values.has('patch') && !patchFor(left, next.patch)) {
          errors.push(`色块“${next.patch}”不在色卡 ${left.chart} 的声明网格内`);
        }
      } else {
        if (values.has('patch')) {
          errors.push('参数 patch 仅适用于同一色卡的图像组合');
        }
        if (values.has('patch_left') && !patchFor(left, next.patchLeft)) {
          errors.push(`左侧色块“${next.patchLeft}”不在色卡 ${left.chart} 的声明网格内`);
        }
        if (values.has('patch_right') && !patchFor(right, next.patchRight)) {
          errors.push(`右侧色块“${next.patchRight}”不在色卡 ${right.chart} 的声明网格内`);
        }
        if (next.layout === 'split' || next.layout === 'blink') {
          errors.push('分割/交替仅适用于同一色卡的图像组合');
        }
      }
    }
    return { errors, next };
  }
  function reportHashErrors(errors) {
    hashError.hidden = false;
    hashError.textContent = `报告链接状态无效，未应用：${errors.join('；')}。请使用选择器重新选择图像与色块。`;
  }
  function applyHash() {
    const { errors, next } = parseHash(location.hash);
    if (errors.length) {
      reportHashErrors(errors);
      renderViews();
      return;
    }
    hashError.hidden = true;
    hashError.textContent = '';
    Object.assign(state, {
      left: next.left,
      right: next.right,
      mode: next.mode,
      zoom: next.zoom,
      layout: next.layout,
      split: Number(next.split),
      blink: next.blink,
      patch: next.patch,
      patchLeft: next.patchLeft,
      patchRight: next.patchRight,
    });
    normalizePatches();
    renderAll();
  }

  // ---------- 渲染 ----------
  function boxOf(viewport) {
    return { w: viewport.clientWidth - 16, h: viewport.clientHeight - 16 };
  }
  function deviceFactor() {
    const dpr = window.devicePixelRatio || 1;
    const devices = state.zoom === 'native' ? 1 : Number(state.zoom.slice(1));
    return devices / dpr;
  }
  function zoomDescription(factor) {
    const dpr = window.devicePixelRatio || 1;
    const percent = `${Math.round(factor * 1000) / 10}%`;
    if (state.zoom === 'fit') {
      return `适应窗口 · 显示约 ${percent} · 仅调整显示，不改变像素数据`;
    }
    const devices = state.zoom === 'native' ? 1 : Number(state.zoom.slice(1));
    const name = state.zoom === 'native' ? '原始像素' : `${devices} 倍`;
    return `${name}：1 个源像素 = ${devices} 个设备像素 · 显示约 ${percent}（设备像素比 ${dpr}）`;
  }
  function fitOrDeviceFactor(viewport, w, h) {
    if (state.zoom === 'fit') {
      const box = boxOf(viewport);
      if (box.w <= 0 || box.h <= 0) return null;
      return Math.min(box.w / w, box.h / h);
    }
    return deviceFactor();
  }

  function renderHeader() {
    const title = typeof data.title === 'string' && data.title ? data.title : '色卡视觉检查报告';
    document.title = `${title} · 色卡视觉检查报告`;
    titleNode.textContent = title;
    const description = typeof data.description === 'string' ? data.description : '';
    descriptionNode.textContent = description;
    descriptionNode.hidden = !description;
  }

  function renderSelectors() {
    for (const image of images) {
      const text = `${image.label}（${image.id} · ${chartLabel(image)}）`;
      for (const select of [leftSelect, rightSelect]) {
        const option = document.createElement('option');
        option.value = image.id;
        option.textContent = text;
        select.appendChild(option);
      }
    }
    leftSelect.value = state.left;
    rightSelect.value = state.right;
    leftSelect.setAttribute('aria-label', '左侧图像选择');
    rightSelect.setAttribute('aria-label', '右侧图像选择');
  }

  function renderMeta() {
    for (const side of ['left', 'right']) {
      const image = imageById(state[side]);
      const node = nodes[side];
      node.metaLabel.textContent = `${side === 'left' ? '左侧' : '右侧'}：${image.label}（${image.id}）`;
      node.caption.textContent = `${image.label}（${image.id}）`;
      node.metaChart.textContent = `色卡：${chartLabel(image)}（${image.chart}）`;
      node.metaDescription.textContent = image.description ? image.description : '';
      node.metaDescription.hidden = !image.description;
      const cctf = image.cctf_decoding ? 'CCTF 解码已启用' : '无需 CCTF 解码（已声明线性）';
      const display = image.display;
      const displayBits = [];
      if (display && typeof display === 'object') {
        if (typeof display.carrier === 'string') displayBits.push(display.carrier.toUpperCase());
        if (typeof display.bit_depth === 'number') displayBits.push(`${display.bit_depth} 位`);
        if (typeof display.color_space === 'string') displayBits.push(`${display.color_space} 载体`);
      }
      const displayText = displayBits.length ? ` · 显示副本：${displayBits.join(' / ')}（仅显示载体）` : '';
      node.metaColor.textContent = `颜色空间：${image.color_space} · ${cctf}${displayText} · ${image.width}×${image.height} 像素`;
    }
    const left = imageById(state.left);
    const right = imageById(state.right);
    if (left.chart !== right.chart) {
      chartWarning.hidden = false;
      chartWarning.textContent = `当前左右图像分属不同色卡（${chartLabel(left)} / ${chartLabel(right)}），仅作并列视觉对照。色块窗口按各自声明网格独立裁切，相同编号不代表同一物理色块。`;
    } else {
      chartWarning.hidden = true;
      chartWarning.textContent = '';
    }
  }

  function setSizedImage(img, image, factor) {
    if (img.getAttribute('src') !== image.src) img.src = image.src;
    img.style.width = `${image.width * factor}px`;
    img.style.height = `${image.height * factor}px`;
  }

  let lastZoomDescription = '';
  function layoutImages() {
    viewer.dataset.zoomMode = state.zoom === 'fit' ? 'fit' : 'native';
    const left = imageById(state.left);
    const right = imageById(state.right);
    if (state.mode === 'whole') {
      if (state.layout === 'side') {
        // 两侧共用同一显示比例，保持可比性；较大图像决定比例。
        const box = boxOf(nodes.left.viewport);
        if (box.w <= 0 || box.h <= 0) return;
        const factor = state.zoom === 'fit'
          ? Math.min(box.w / left.width, box.h / left.height, box.w / right.width, box.h / right.height)
          : deviceFactor();
        for (const side of ['left', 'right']) {
          const image = imageById(state[side]);
          const img = nodes[side].image;
          setSizedImage(img, image, factor);
          img.alt = `${side === 'left' ? '左侧' : '右侧'}整图：${image.label}`;
        }
        lastZoomDescription = zoomDescription(factor);
      } else if (state.layout === 'split' && sameLayout()) {
        const factor = fitOrDeviceFactor(document.querySelector('[data-split-viewport]'), left.width, left.height);
        if (factor === null) return;
        setSizedImage(splitImages.left, left, factor);
        setSizedImage(splitImages.right, right, factor);
        splitImages.left.alt = `左侧整图：${left.label}`;
        splitImages.right.alt = `右侧整图：${right.label}`;
        const canvas = document.querySelector('[data-split-canvas]');
        canvas.style.width = `${left.width * factor}px`;
        canvas.style.height = `${left.height * factor}px`;
        applySplit();
        lastZoomDescription = zoomDescription(factor);
      } else if (state.layout === 'blink' && sameLayout()) {
        const factor = fitOrDeviceFactor(document.querySelector('[data-blink-viewport]'), left.width, left.height);
        if (factor === null) return;
        setSizedImage(blinkImages.left, left, factor);
        setSizedImage(blinkImages.right, right, factor);
        blinkImages.left.alt = `左侧整图：${left.label}`;
        blinkImages.right.alt = `右侧整图：${right.label}`;
        const canvas = document.querySelector('[data-blink-canvas]');
        canvas.style.width = `${left.width * factor}px`;
        canvas.style.height = `${left.height * factor}px`;
        applyBlink();
        lastZoomDescription = zoomDescription(factor);
      }
    } else {
      const patches = {
        left: sameChart() ? patchFor(left, state.patch) : patchFor(left, state.patchLeft),
        right: sameChart() ? patchFor(right, state.patch) : patchFor(right, state.patchRight),
      };
      for (const side of ['left', 'right']) {
        const image = imageById(state[side]);
        const patch = patches[side];
        const node = nodes[side];
        if (!patch) continue;
        const factor = fitOrDeviceFactor(node.patchViewport, patch.w, patch.h);
        if (factor === null) continue;
        if (node.patchImage.getAttribute('src') !== image.src) node.patchImage.src = image.src;
        node.patchImage.alt = `${side === 'left' ? '左侧' : '右侧'}色块 ${patch.label}：${image.label}`;
        node.patchImage.style.width = `${image.width * factor}px`;
        node.patchImage.style.height = `${image.height * factor}px`;
        node.patchImage.style.left = `${-patch.x * factor}px`;
        node.patchImage.style.top = `${-patch.y * factor}px`;
        node.patchCanvas.style.width = `${patch.w * factor}px`;
        node.patchCanvas.style.height = `${patch.h * factor}px`;
        node.patchLabel.textContent = `${side === 'left' ? '左侧' : '右侧'}色块 ${patch.label} · ${image.label}`;
        node.patchRect.textContent = `窗口矩形 x=${patch.x}, y=${patch.y}, ${patch.w}×${patch.h} 像素 · 原图 ${image.width}×${image.height}`;
        if (node.locatorImage.getAttribute('src') !== image.src) node.locatorImage.src = image.src;
        node.locatorImage.alt = `${side === 'left' ? '左侧' : '右侧'}全图位置定位`;
        node.locatorRect.style.left = `${100 * patch.x / image.width}%`;
        node.locatorRect.style.top = `${100 * patch.y / image.height}%`;
        node.locatorRect.style.width = `${100 * patch.w / image.width}%`;
        node.locatorRect.style.height = `${100 * patch.h / image.height}%`;
        node.locatorCaption.textContent = `${patch.label} 在全图中的位置（黄框）`;
        if (side === 'left') lastZoomDescription = zoomDescription(factor);
      }
    }
    zoomLabelNode.textContent = lastZoomDescription;
  }

  function applySplit() {
    const position = Math.max(0, Math.min(100, Math.round(state.split)));
    splitRange.value = String(position);
    splitImages.right.style.clipPath = `inset(0 0 0 ${position}%)`;
    splitDivider.style.left = `${position}%`;
    splitLabel.textContent = `分割位置 ${position}%：左侧 ${imageById(state.left).label} 显示 0–${position}%，右侧 ${imageById(state.right).label} 显示 ${position}–100%（同一声明色卡布局，仅视觉对照）`;
  }
  function applyBlink() {
    const current = state.blink === 'left' ? imageById(state.left) : imageById(state.right);
    blinkImages.left.hidden = state.blink !== 'left';
    blinkImages.right.hidden = state.blink !== 'right';
    blinkButton.textContent = `交替显示：当前${state.blink === 'left' ? '左侧' : '右侧'}`;
    blinkButton.setAttribute('aria-label', `交替显示 A/B；当前显示${state.blink === 'left' ? '左侧' : '右侧'}图像 ${current.label}`);
    blinkLabel.textContent = `当前显示：${state.blink === 'left' ? '左侧' : '右侧'} ${current.label}（${current.id}）；手动交替左右图像（同一声明色卡布局，仅视觉对照）`;
  }

  function renderWholePatchPanels() {
    const whole = state.mode === 'whole';
    const patch = !whole;
    wholePanel.hidden = !whole;
    patchPanel.hidden = whole;
    for (const button of modeButtons) {
      button.setAttribute('aria-pressed', String(button.dataset.viewMode === state.mode));
    }
    const aligned = sameLayout();
    if (!aligned && state.layout !== 'side') state.layout = 'side';
    for (const button of layoutButtons) {
      const layout = button.dataset.layout;
      button.setAttribute('aria-pressed', String(state.layout === layout));
      button.disabled = !aligned && layout !== 'side';
      button.title = button.disabled ? '分割/交替仅适用于同一声明色卡的图像' : '';
    }
    splitPanel.hidden = !(whole && state.layout === 'split' && aligned);
    blinkPanel.hidden = !(whole && state.layout === 'blink' && aligned);
    document.querySelector('[data-whole-panes]').hidden = !(whole && state.layout === 'side');
    if (whole) {
      wholeNote.textContent = aligned
        ? '分割/交替按声明的共同色卡布局显示，仅为视觉对照，不代表物理或像素级对应。'
        : '当前图像分属不同色卡，仅支持并排整图对照。';
    }
    patchSharedControls.hidden = !(patch && sameChart());
    patchSplitControls.hidden = !(patch && !sameChart());
    sheetHint.textContent = sameChart()
      ? '点击任意色块可同步选择左右色块；每格包含左右两个固定窗口。'
      : '左右色块独立选择；相同编号不代表同一物理色块。';
    sheets.shared.hidden = !sameChart();
    sheets.left.hidden = sameChart();
    sheets.right.hidden = sameChart();
  }

  function fillPatchSelect(select, image, selected) {
    while (select.firstChild) select.removeChild(select.firstChild);
    for (const patch of patchList(image)) {
      const option = document.createElement('option');
      option.value = patch.label;
      option.textContent = `${patch.label}（x=${patch.x}, y=${patch.y}, ${patch.w}×${patch.h}）`;
      select.appendChild(option);
    }
    if (selected) select.value = selected;
    select.disabled = patchList(image).length === 0;
  }
  function renderPatchControls() {
    const left = imageById(state.left);
    const right = imageById(state.right);
    if (sameChart()) {
      fillPatchSelect(patchSelect, left, state.patch);
      patchSelect.setAttribute('aria-label', '色块窗口选择（左右同步）');
    } else {
      fillPatchSelect(patchLeftSelect, left, state.patchLeft);
      fillPatchSelect(patchRightSelect, right, state.patchRight);
      patchLeftSelect.setAttribute('aria-label', '左侧色块窗口选择');
      patchRightSelect.setAttribute('aria-label', '右侧色块窗口选择');
    }
  }

  function makeThumb(image, patch, altText) {
    const box = document.createElement('span');
    box.className = 'thumb';
    // 缩略窗口按较长边 64 像素定界，保证任意窗口在窄屏不溢出。
    const scale = 64 / Math.max(patch.w, patch.h);
    box.style.width = `${patch.w * scale}px`;
    box.style.height = `${patch.h * scale}px`;
    const img = document.createElement('img');
    img.src = image.src;
    img.alt = altText;
    img.loading = 'lazy';
    img.style.width = `${image.width * scale}px`;
    img.style.height = `${image.height * scale}px`;
    img.style.left = `${-patch.x * scale}px`;
    img.style.top = `${-patch.y * scale}px`;
    box.appendChild(img);
    return box;
  }
  function sheetSelectionValue(sheet) {
    if (sheet === sheets.shared) return state.patch;
    if (sheet === sheets.left) return state.patchLeft;
    return state.patchRight;
  }
  function updateSheetSelection() {
    for (const key of ['shared', 'left', 'right']) {
      const sheet = sheets[key];
      if (sheet.hidden) continue;
      const selected = sheetSelectionValue(sheet);
      for (const cell of sheet.querySelectorAll('[data-patch]')) {
        cell.setAttribute('aria-pressed', String(cell.dataset.patch === selected));
      }
    }
  }
  function buildSheet(sheet, side) {
    while (sheet.firstChild) sheet.removeChild(sheet.firstChild);
    const paired = side === 'shared';
    const primary = paired ? imageById(state.left) : imageById(side === 'left' ? state.left : state.right);
    const partner = paired ? imageById(state.right) : null;
    for (const patch of patchList(primary)) {
      const cell = document.createElement('button');
      cell.type = 'button';
      cell.className = 'sheet-cell';
      cell.dataset.patch = patch.label;
      const label = document.createElement('strong');
      label.className = 'sheet-label';
      label.textContent = `色块 ${patch.label}`;
      const rect = document.createElement('span');
      rect.className = 'sheet-rect';
      rect.textContent = `x=${patch.x}, y=${patch.y}, ${patch.w}×${patch.h}`;
      const thumbs = document.createElement('span');
      thumbs.className = 'sheet-thumbs';
      const appendThumb = (tagText, image, altText) => {
        const wrap = document.createElement('span');
        wrap.className = 'sheet-thumb';
        const tag = document.createElement('span');
        tag.textContent = tagText;
        wrap.appendChild(tag);
        wrap.appendChild(makeThumb(image, patch, altText));
        thumbs.appendChild(wrap);
      };
      if (partner) {
        appendThumb('左', primary, `左侧 ${primary.label} 色块 ${patch.label}`);
        appendThumb('右', partner, `右侧 ${partner.label} 色块 ${patch.label}`);
        cell.setAttribute('aria-label', `选择色块 ${patch.label}：左侧 ${primary.label} 与右侧 ${partner.label}`);
      } else {
        appendThumb(side === 'left' ? '左侧' : '右侧', primary, `${side === 'left' ? '左侧' : '右侧'} ${primary.label} 色块 ${patch.label}`);
        cell.setAttribute('aria-label', `选择${side === 'left' ? '左侧' : '右侧'}色块 ${patch.label}`);
      }
      cell.appendChild(label);
      cell.appendChild(rect);
      cell.appendChild(thumbs);
      sheet.appendChild(cell);
    }
  }
  let sheetsKey = '';
  function renderSheets() {
    const key = `${state.left}|${state.right}`;
    if (key === sheetsKey) {
      updateSheetSelection();
      return;
    }
    sheetsKey = key;
    if (sameChart()) {
      buildSheet(sheets.shared, 'shared');
    } else {
      buildSheet(sheets.left, 'left');
      buildSheet(sheets.right, 'right');
    }
    updateSheetSelection();
  }

  function renderProvenance() {
    while (provenanceBody.firstChild) provenanceBody.removeChild(provenanceBody.firstChild);
    for (const image of images) {
      const row = document.createElement('tr');
      const source = image.source || {};
      const cells = [
        `${image.label}（${image.id}）`,
        `${chartLabel(image)}（${image.chart}）`,
        `${image.color_space} · ${image.cctf_decoding ? 'CCTF 解码已启用' : '无需 CCTF 解码（已声明线性）'}`,
        `${image.width}×${image.height} 像素`,
        source.path ? source.path : '未提供',
        source.sha256 ? source.sha256 : '未提供',
      ];
      for (const text of cells) {
        const cell = document.createElement('td');
        cell.textContent = text;
        row.appendChild(cell);
      }
      provenanceBody.appendChild(row);
    }
  }

  function renderViews() {
    renderMeta();
    renderWholePatchPanels();
    renderPatchControls();
    renderSheets();
    layoutImages();
    leftSelect.value = state.left;
    rightSelect.value = state.right;
    zoomSelect.value = state.zoom;
  }

  function renderAll() {
    renderViews();
    hashError.hidden = true;
    hashError.textContent = '';
    syncHash();
  }

  // ---------- 交互 ----------
  function selectImage(side, id) {
    const image = imageById(id);
    if (!image) return;
    state[side] = id;
    normalizePatches();
    renderAll();
  }
  leftSelect.addEventListener('change', () => selectImage('left', leftSelect.value));
  rightSelect.addEventListener('change', () => selectImage('right', rightSelect.value));
  swapButton.addEventListener('click', () => {
    const left = state.left;
    state.left = state.right;
    state.right = left;
    const patchLeft = state.patchLeft;
    state.patchLeft = state.patchRight;
    state.patchRight = patchLeft;
    state.blink = state.blink === 'left' ? 'right' : 'left';
    normalizePatches();
    renderAll();
  });
  for (const button of modeButtons) {
    button.addEventListener('click', () => {
      state.mode = button.dataset.viewMode;
      renderAll();
    });
  }
  zoomSelect.addEventListener('change', () => {
    state.zoom = zoomSelect.value;
    renderAll();
  });
  for (const button of layoutButtons) {
    button.addEventListener('click', () => {
      if (button.disabled) return;
      state.layout = button.dataset.layout;
      renderAll();
    });
  }
  splitRange.addEventListener('input', () => {
    state.split = Number(splitRange.value);
    applySplit();
    syncHash();
  });
  blinkButton.addEventListener('click', () => {
    state.blink = state.blink === 'left' ? 'right' : 'left';
    applyBlink();
    syncHash();
  });
  function selectPatch(field, label) {
    const image = imageById(field === 'patchRight' ? state.right : state.left);
    if (!patchFor(image, label)) return;
    state[field] = label;
    renderAll();
  }
  patchSelect.addEventListener('change', () => selectPatch('patch', patchSelect.value));
  patchLeftSelect.addEventListener('change', () => selectPatch('patchLeft', patchLeftSelect.value));
  patchRightSelect.addEventListener('change', () => selectPatch('patchRight', patchRightSelect.value));
  for (const key of ['shared', 'left', 'right']) {
    sheets[key].addEventListener('click', event => {
      const cell = event.target.closest('[data-patch]');
      if (!cell) return;
      const label = cell.dataset.patch;
      if (key === 'shared') selectPatch('patch', label);
      else if (key === 'left') selectPatch('patchLeft', label);
      else selectPatch('patchRight', label);
    });
  }
  document.addEventListener('keydown', event => {
    if (state.mode !== 'patch' || !sameChart()) return;
    if (event.target.closest('button, input, select, textarea, a, [contenteditable]')) return;
    if (event.key !== 'ArrowLeft' && event.key !== 'ArrowRight') return;
    const list = patchList(imageById(state.left));
    const index = list.findIndex(patch => patch.label === state.patch);
    if (index < 0) return;
    const step = event.key === 'ArrowRight' ? 1 : -1;
    const next = list[(index + step + list.length) % list.length];
    event.preventDefault();
    selectPatch('patch', next.label);
  });
  if (fullscreenButton) {
    fullscreenButton.setAttribute('aria-label', '切换图像查看全屏');
    fullscreenButton.disabled = !viewer.requestFullscreen || !document.fullscreenEnabled;
    fullscreenButton.addEventListener('click', async () => {
      try {
        if (document.fullscreenElement === viewer) await document.exitFullscreen();
        else await viewer.requestFullscreen();
      } catch (_) {
        zoomLabelNode.textContent = '此环境无法进入全屏';
      }
    });
    document.addEventListener('fullscreenchange', () => {
      const active = document.fullscreenElement === viewer;
      fullscreenButton.setAttribute('aria-pressed', String(active));
      fullscreenButton.textContent = active ? '退出全屏' : '全屏';
      layoutImages();
    });
  }

  // 视口拖动与左右同步滚动。
  let syncing = false;
  const panes = [...document.querySelectorAll('.viewport')];
  for (const pane of panes) {
    pane.addEventListener('dragstart', event => event.preventDefault());
    pane.addEventListener('scroll', () => {
      if (syncing || state.mode !== 'whole' || state.layout !== 'side') return;
      const side = pane.dataset.viewport;
      if (!side) return;
      const other = nodes[side === 'left' ? 'right' : 'left'].viewport;
      syncing = true;
      other.scrollLeft = pane.scrollLeft;
      other.scrollTop = pane.scrollTop;
      requestAnimationFrame(() => { syncing = false; });
    });
  }
  for (const pane of panes) {
    let pan = null;
    pane.addEventListener('pointerdown', event => {
      if (event.button !== 0 || pan || event.target.closest('button, input, select, a')) return;
      event.preventDefault();
      pane.focus({ preventScroll: true });
      pan = { pointer: event.pointerId, x: event.clientX, y: event.clientY, left: pane.scrollLeft, top: pane.scrollTop };
      pane.setPointerCapture(event.pointerId);
    });
    pane.addEventListener('pointermove', event => {
      if (!pan || pan.pointer !== event.pointerId) return;
      pane.scrollLeft = pan.left - (event.clientX - pan.x);
      pane.scrollTop = pan.top - (event.clientY - pan.y);
    });
    const finish = event => {
      if (!pan || pan.pointer !== event.pointerId) return;
      pan = null;
      if (pane.hasPointerCapture(event.pointerId)) pane.releasePointerCapture(event.pointerId);
    };
    pane.addEventListener('pointerup', finish);
    pane.addEventListener('pointercancel', finish);
    pane.addEventListener('lostpointercapture', finish);
  }

  for (const side of ['left', 'right']) {
    nodes[side].image.addEventListener('load', layoutImages);
    nodes[side].patchImage.addEventListener('load', layoutImages);
  }
  splitImages.left.addEventListener('load', layoutImages);
  blinkImages.left.addEventListener('load', layoutImages);
  window.addEventListener('resize', layoutImages);
  window.addEventListener('hashchange', () => {
    if (location.hash === lastWrittenHash) return;
    applyHash();
  });

  // ---------- 初始化 ----------
  renderHeader();
  renderSelectors();
  renderProvenance();
  normalizePatches();
  if (location.hash) applyHash();
  else renderAll();
})();
