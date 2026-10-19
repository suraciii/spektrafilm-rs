(function () {
  'use strict';

  // ---------- 内嵌报告数据（禁止网络请求） ----------
  const dataElement = document.getElementById('photo-report-data');
  const errorBox = document.getElementById('photo-report-error');
  const errorText = document.getElementById('photo-report-error-text');
  const recoveryButton = document.getElementById('photo-report-recovery');

  function fatal(message) {
    errorText.textContent = `报告数据不可用：${message}。本页面无法展示照片。`;
    errorBox.hidden = false;
    recoveryButton.hidden = true;
  }
  function reportFragmentErrors(errors) {
    errorText.textContent = `链接状态无效，未应用：${errors.join('；')}。已保留默认视图，可点击“恢复默认视图”重新载入。`;
    errorBox.hidden = false;
    recoveryButton.hidden = false;
  }
  function clearFragmentErrors() {
    errorBox.hidden = true;
    errorText.textContent = '';
  }

  if (!dataElement) {
    fatal('未找到内嵌数据节点 photo-report-data');
    return;
  }
  let data = null;
  try {
    data = JSON.parse(dataElement.textContent);
  } catch (error) {
    fatal(`内嵌 JSON 解析失败（${error.message}）`);
    return;
  }
  const MODES = ['single_photo', 'two_photo', 'requested_comparisons'];
  if (!data || !MODES.includes(data.mode) || typeof data.title !== 'string' || !Array.isArray(data.photos) || data.photos.length < 1) {
    fatal('内嵌数据缺少 mode、title 或 photos');
    return;
  }

  // ---------- 索引与状态 ----------
  const photos = data.photos;
  const comparisons = Array.isArray(data.comparisons) ? data.comparisons : [];
  const photosById = new Map(photos.map(photo => [photo.id, photo]));
  const comparisonsById = new Map(comparisons.map(pair => [pair.id, pair]));
  const ZOOMS = ['fit', 'native', 'x2', 'x4'];
  const LAYOUTS = ['side', 'blink'];

  // 每个模式只接受自己的片段键；其他模式的键视为无效，绝不静默回退。
  const FRAGMENT_KEYS = {
    single_photo: ['photo', 'roi', 'zoom'],
    two_photo: ['roi', 'layout', 'zoom', 'blink'],
    requested_comparisons: ['pair', 'roi', 'layout', 'zoom', 'blink'],
  };
  const DEFAULT_ZOOM = 'fit';
  const DEFAULT_LAYOUT = 'side';

  const state = {
    pair: null,
    photo: null,
    roi: null,
    layout: DEFAULT_LAYOUT,
    zoom: DEFAULT_ZOOM,
    blink: 'left',
  };

  function photoById(id) {
    return photosById.get(id) || null;
  }
  function pairById(id) {
    return comparisonsById.get(id) || null;
  }
  function currentPair() {
    if (data.mode === 'single_photo') return null;
    if (state.pair) {
      const selected = pairById(state.pair);
      if (selected) return selected;
    }
    return comparisons[0] || null;
  }
  // 单照片模式没有对照；并排/交替顺序可用交换按钮翻转，但不改变声明的配对。
  let sideOrder = ['left', 'right'];
  function pairSides() {
    if (data.mode === 'single_photo') return [photos[0], null];
    const pair = currentPair();
    if (!pair) return [null, null];
    const bySide = { left: photoById(pair.left), right: photoById(pair.right) };
    return sideOrder.map(side => bySide[side]);
  }
  // ROI 解析由生成器按“左侧顺序后接右侧独有 ID”完成；这里只按 ID 查找。
  function roiEntries() {
    const pair = currentPair();
    if (data.mode === 'single_photo') {
      return photos[0].rois.map(roi => ({ id: roi.id, roi }));
    }
    if (!pair) return [];
    return pair.rois.map(entry => ({ id: entry.id, entry }));
  }
  function roiEntry(id) {
    return roiEntries().find(entry => entry.id === id) || null;
  }
  function selectedRoi() {
    if (!state.roi) return null;
    return roiEntry(state.roi);
  }

  // ---------- DOM 锚点 ----------
  const titleNode = document.getElementById('photo-report-title');
  const descriptionNode = document.getElementById('photo-report-description');
  const viewer = document.querySelector('.viewer');
  const pairSelectField = document.getElementById('pair-select-field');
  const pairSelect = document.getElementById('pair-select');
  const swapButton = document.getElementById('swap-button');
  const metaGrid = document.getElementById('meta-grid');
  const pairNote = document.getElementById('pair-note');
  const layoutGroup = document.getElementById('layout-group');
  const layoutButtons = [...document.querySelectorAll('[data-layout]')];
  const zoomSelect = document.getElementById('zoom-select');
  const zoomLabel = document.getElementById('zoom-label');
  const fullscreenButton = document.getElementById('fullscreen-button');
  const sidePanel = document.getElementById('side-panel');
  const sidePanes = document.getElementById('side-panes');
  const wholeNote = document.getElementById('whole-note');
  const blinkPanel = document.getElementById('blink-panel');
  const blinkButton = document.getElementById('blink-button');
  const blinkLabel = document.getElementById('blink-label');
  const blinkViewport = document.getElementById('blink-viewport');
  const blinkCanvas = document.getElementById('blink-canvas');
  const blinkImages = {
    left: document.getElementById('blink-image-left'),
    right: document.getElementById('blink-image-right'),
  };
  const roiPanel = document.getElementById('roi-panel');
  const roiHeading = document.getElementById('roi-heading');
  const roiHint = document.getElementById('roi-hint');
  const roiSelect = document.getElementById('roi-select');
  const roiClearButton = document.getElementById('roi-clear-button');
  const roiPanes = document.getElementById('roi-panes');
  const roiWarning = document.getElementById('roi-warning');
  const provenanceBody = document.getElementById('provenance-body');

  // ---------- URL 片段状态 ----------
  let lastWrittenHash = '';

  function currentHash() {
    const parts = [];
    const push = (key, value) => parts.push(`${key}=${encodeURIComponent(value)}`);
    if (data.mode === 'requested_comparisons' && state.pair) push('pair', state.pair);
    if (data.mode === 'single_photo' && state.photo) push('photo', state.photo);
    if (state.roi) push('roi', state.roi);
    push('layout', state.layout);
    push('zoom', state.zoom);
    if (state.layout === 'blink' && data.mode !== 'single_photo') push('blink', state.blink);
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
  // 片段校验必须按“候选对照”解析 ROI，而不是当前已选对照；
  // 否则切换对照时旧对照的 ROI 列表会把新对照的合法 ROI 误判为未知。
  function roiEntryFor(pairId, id) {
    const pair = data.mode === 'single_photo' ? null : pairById(pairId);
    if (data.mode === 'single_photo') {
      return photos[0].rois.find(roi => roi.id === id) ? { id } : null;
    }
    if (!pair || !id) return null;
    return pair.rois.find(entry => entry.id === id) ? { id } : null;
  }
  function parseHash(hash) {
    const errors = [];
    const values = new Map();
    const allowed = FRAGMENT_KEYS[data.mode];
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
        if (!allowed.includes(key)) {
          errors.push(`参数“${key}”在本报告模式下无效`);
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
      pair: values.has('pair') ? values.get('pair') : (currentPair() ? currentPair().id : ''),
      photo: values.has('photo') ? values.get('photo') : photos[0].id,
      roi: values.has('roi') ? values.get('roi') : '',
      layout: values.get('layout') || DEFAULT_LAYOUT,
      zoom: values.get('zoom') || DEFAULT_ZOOM,
      blink: values.get('blink') || 'left',
    };
    if (data.mode === 'requested_comparisons' && values.has('pair') && !pairById(next.pair)) {
      errors.push(`未知对照 ID“${next.pair}”`);
    }
    if (data.mode === 'single_photo' && values.has('photo') && !photoById(next.photo)) {
      errors.push(`未知照片 ID“${next.photo}”`);
    }
    if (values.has('roi') && !roiEntryFor(next.pair, next.roi)) {
      errors.push(`未知关注区域 ID“${next.roi}”`);
    }
    if (!LAYOUTS.includes(next.layout)) errors.push(`未知排布方式“${next.layout}”`);
    if (!ZOOMS.includes(next.zoom)) errors.push(`未知缩放“${next.zoom}”`);
    if (!['left', 'right'].includes(next.blink)) errors.push(`交替状态无效“${next.blink}”`);
    if (next.layout === 'blink' && data.mode === 'single_photo') {
      errors.push('单照片模式不支持交替排布');
    }
    return { errors, next };
  }
  function applyHash() {
    const { errors, next } = parseHash(location.hash);
    if (errors.length) {
      reportFragmentErrors(errors);
      state.pair = data.mode === 'requested_comparisons' ? (currentPair() ? currentPair().id : '') : null;
      state.photo = photos[0].id;
      state.roi = null;
      state.layout = DEFAULT_LAYOUT;
      state.zoom = DEFAULT_ZOOM;
      state.blink = 'left';
      renderAll();
      return;
    }
    clearFragmentErrors();
    state.pair = data.mode === 'requested_comparisons' ? next.pair : null;
    state.photo = data.mode === 'single_photo' ? next.photo : null;
    state.roi = next.roi || null;
    state.layout = next.layout;
    state.zoom = next.zoom;
    state.blink = next.blink;
    renderAll();
  }

  // ---------- 渲染 ----------
  function text(value) {
    return typeof value === 'string' ? value : '';
  }
  function photoTitle(photo) {
    return `${text(photo.label)}（${text(photo.id)}）`;
  }
  function colorText(photo) {
    const color = photo.color || {};
    const space = text(color.color_space) || 'sRGB';
    const cctf = color.cctf_decoding === false ? '已声明线性，无需 CCTF 解码' : 'CCTF 解码已启用';
    const declared = color.color_space === undefined && color.cctf_decoding === undefined ? '（默认声明）' : '';
    return `颜色空间：${space}${declared} · ${cctf}`;
  }
  function displayText(photo) {
    const display = photo.display || {};
    const bits = [];
    if (text(display.carrier)) bits.push(text(display.carrier).toUpperCase());
    if (typeof display.bit_depth === 'number') bits.push(`${display.bit_depth} 位`);
    if (text(display.color_space)) bits.push(text(display.color_space));
    return bits.length ? bits.join(' / ') : '未记录';
  }
  function dimensionText(photo) {
    return `${photo.width}×${photo.height} 像素`;
  }
  function rectText(rect) {
    const [x, y, w, h] = rect;
    return `矩形 x=${x}, y=${y}, ${w}×${h} 像素`;
  }
  function metaCard(heading, photo) {
    const card = document.createElement('div');
    card.className = 'meta-card';
    const title = document.createElement('h3');
    title.textContent = heading;
    card.appendChild(title);
    const label = document.createElement('p');
    const strong = document.createElement('strong');
    strong.textContent = photoTitle(photo);
    label.appendChild(strong);
    card.appendChild(label);
    const note = document.createElement('p');
    note.textContent = text(photo.note);
    note.hidden = !note.textContent;
    card.appendChild(note);
    const dims = document.createElement('p');
    dims.className = 'small';
    dims.textContent = dimensionText(photo);
    card.appendChild(dims);
    const color = document.createElement('p');
    color.className = 'small';
    color.textContent = colorText(photo);
    card.appendChild(color);
    const display = document.createElement('p');
    display.className = 'small';
    display.textContent = `显示副本：${displayText(photo)}`;
    card.appendChild(display);
    return card;
  }

  function renderHeader() {
    document.title = `${text(data.title)} · 照片报告`;
    titleNode.textContent = text(data.title);
    const description = text(data.description);
    descriptionNode.textContent = description;
    descriptionNode.hidden = !description;
  }

  function renderPairControls() {
    pairSelectField.hidden = data.mode !== 'requested_comparisons';
    swapButton.hidden = data.mode === 'single_photo';
    if (data.mode !== 'requested_comparisons') return;
    while (pairSelect.firstChild) pairSelect.removeChild(pairSelect.firstChild);
    for (const pair of comparisons) {
      const left = photoById(pair.left);
      const right = photoById(pair.right);
      const option = document.createElement('option');
      option.value = pair.id;
      option.textContent = text(pair.label)
        ? `${text(pair.label)}（${pair.id}）`
        : `${photoTitle(left)} ↔ ${photoTitle(right)}`;
      pairSelect.appendChild(option);
    }
    const pair = currentPair();
    if (pair) pairSelect.value = pair.id;
    pairSelect.setAttribute('aria-label', '选择声明过的对照');
  }

  function renderMeta() {
    while (metaGrid.firstChild) metaGrid.removeChild(metaGrid.firstChild);
    const [left, right] = pairSides();
    if (left) metaGrid.appendChild(metaCard(data.mode === 'single_photo' ? '照片' : '左侧', left));
    if (right) metaGrid.appendChild(metaCard('右侧', right));
    const pair = currentPair();
    const note = pair ? text(pair.note) : '';
    pairNote.textContent = note;
    pairNote.hidden = !note;
  }

  // 生成器已解析两侧可用性；一侧缺失时明确标注原因，不留空矩形。
  // 可用侧：{"availability":"available","rect","dimensions","display_artifact"}；
  // 不可用侧：{"availability":"unavailable","reason"}，无矩形。
  function roiSideState(entry, side) {
    if (data.mode === 'single_photo') return { available: true, roi: entry.roi, photo: photos[0] };
    const value = entry.entry[side];
    const pair = currentPair();
    const photoId = pair ? (side === 'left' ? pair.left : pair.right) : '';
    const photo = photoById(photoId);
    if (value && value.availability === 'available' && Array.isArray(value.rect) && photo) {
      return { available: true, rect: value.rect, dimensions: value.dimensions, photo };
    }
    const reason = value && typeof value.reason === 'string' && value.reason
      ? value.reason
      : `照片 ${photoId || '?'} 未声明此关注区域`;
    return { available: false, reason };
  }
  function sideName(side) {
    return side === 'left' ? '左侧' : '右侧';
  }

  function makeViewportPane(side, labelText, rectDescription, withLocator, ariaSide) {
    const aria = ariaSide === undefined ? sideName(side) : ariaSide;
    const pane = document.createElement('figure');
    pane.className = 'pane';
    const caption = document.createElement('figcaption');
    const strong = document.createElement('strong');
    strong.textContent = labelText;
    caption.appendChild(strong);
    const small = document.createElement('span');
    small.className = 'small';
    caption.appendChild(small);
    pane.appendChild(caption);
    const viewport = document.createElement('div');
    viewport.className = 'viewport';
    viewport.tabIndex = 0;
    viewport.dataset.viewport = side;
    viewport.setAttribute('role', 'img');
    enablePan(viewport);
    const canvas = document.createElement('div');
    canvas.className = 'canvas crop';
    canvas.dataset.canvas = side;
    const image = document.createElement('img');
    image.dataset.image = side;
    image.alt = `${aria}：${labelText}`;
    canvas.appendChild(image);
    viewport.appendChild(canvas);
    pane.appendChild(viewport);
    if (withLocator) {
      const locator = document.createElement('figure');
      locator.className = 'locator';
      const locatorImageWrap = document.createElement('div');
      locatorImageWrap.className = 'locator-image';
      const locatorImage = document.createElement('img');
      locatorImage.dataset.locatorImage = side;
      locatorImage.alt = `${aria}全图位置定位：${labelText}`;
      locatorImageWrap.appendChild(locatorImage);
      const rectangle = document.createElement('span');
      rectangle.className = 'rectangle';
      rectangle.dataset.locatorRect = side;
      locatorImageWrap.appendChild(rectangle);
      locator.appendChild(locatorImageWrap);
      const locatorCaption = document.createElement('figcaption');
      locatorCaption.className = 'small';
      locatorCaption.dataset.locatorCaption = side;
      locatorCaption.textContent = '关注区域在全图中的位置（黄框）';
      locator.appendChild(locatorCaption);
      pane.appendChild(locator);
    }
    return pane;
  }
  function makeUnavailablePane(side, labelText, reason) {
    const pane = document.createElement('figure');
    pane.className = 'pane';
    const caption = document.createElement('figcaption');
    const strong = document.createElement('strong');
    strong.textContent = labelText;
    caption.appendChild(strong);
    pane.appendChild(caption);
    const unavailable = document.createElement('p');
    unavailable.className = 'warning';
    unavailable.dataset.unavailable = side;
    unavailable.setAttribute('role', 'status');
    unavailable.textContent = reason;
    pane.appendChild(unavailable);
    return pane;
  }
  function sidePane(side, stateOf, roiId, ariaSide) {
    if (!stateOf.available) {
      return makeUnavailablePane(side, `${sideName(side)}：不可用`, stateOf.reason);
    }
    const rect = data.mode === 'single_photo' ? stateOf.roi.rect : stateOf.rect;
    const label = `${roiLabel(roiId, side)}（${roiId}） · ${photoTitle(stateOf.photo)}`;
    return makeViewportPane(side, label, `${rectText(rect)} · 原图 ${dimensionText(stateOf.photo)}`, true, ariaSide);
  }
  function roiLabel(id, side) {
    const entry = roiEntry(id);
    if (!entry) return text(id);
    const sideRoi = data.mode === 'single_photo' ? entry.roi : sideRoiOf(entry, side);
    if (sideRoi && text(sideRoi.label)) return text(sideRoi.label);
    const leftRoi = data.mode === 'single_photo' ? entry.roi : sideRoiOf(entry, 'left');
    if (leftRoi && text(leftRoi.label)) return text(leftRoi.label);
    return text(id);
  }
  function sideRoiOf(entry, side) {
    const photo = side === 'left' ? photoById(currentPair().left) : photoById(currentPair().right);
    const rois = photo && Array.isArray(photo.rois) ? photo.rois : [];
    return rois.find(roi => roi.id === entry.id) || null;
  }

  function boxOf(viewport) {
    return { w: viewport.clientWidth - 16, h: viewport.clientHeight - 16 };
  }
  function deviceFactor() {
    const dpr = window.devicePixelRatio || 1;
    const devices = state.zoom === 'native' ? 1 : Number(state.zoom.slice(1));
    return devices / dpr;
  }
  function zoomDescription(factor) {
    const percent = `${Math.round(factor * 1000) / 10}%`;
    if (state.zoom === 'fit') {
      return `适应窗口 · 显示约 ${percent} · 仅调整显示，不改变像素与矩形坐标`;
    }
    const devices = state.zoom === 'native' ? 1 : Number(state.zoom.slice(1));
    const name = state.zoom === 'native' ? '原始像素' : `${devices} 倍`;
    return `${name}：1 个源像素 = ${devices} 个设备像素 · 显示约 ${percent}（设备像素比 ${window.devicePixelRatio || 1}）`;
  }
  function fitFactor(viewport, w, h) {
    if (state.zoom !== 'fit') return deviceFactor();
    const box = boxOf(viewport);
    if (box.w <= 0 || box.h <= 0) return null;
    return Math.min(box.w / w, box.h / h);
  }
  function setImage(img, photo, factor) {
    if (img.getAttribute('src') !== photo.src) img.src = photo.src;
    img.style.width = `${photo.width * factor}px`;
    img.style.height = `${photo.height * factor}px`;
  }

  function layoutWhole() {
    const [left, right] = pairSides();
    const sides = [left, right].filter(Boolean);
    if (!sides.length) return;
    if (state.layout === 'blink' && data.mode !== 'single_photo') {
      const factor = fitFactor(blinkViewport, left.width, left.height);
      if (factor === null) return;
      for (const side of ['left', 'right']) {
        const photo = side === 'left' ? left : right;
        setImage(blinkImages[side], photo, factor);
        blinkImages[side].alt = `${sideName(side)}整图：${photoTitle(photo)}`;
      }
      blinkCanvas.style.width = `${left.width * factor}px`;
      blinkCanvas.style.height = `${left.height * factor}px`;
      applyBlink();
      zoomLabel.textContent = zoomDescription(factor);
      return;
    }
    // 并排：两侧共用同一显示比例，保持可比性；较大图像决定比例。
    const panes = [...sidePanes.querySelectorAll('[data-viewport]')];
    const box = boxOf(panes[0] || sidePanes);
    let factor;
    if (state.zoom === 'fit') {
      if (box.w <= 0 || box.h <= 0) return;
      factor = Math.min(...sides.map(photo => Math.min(box.w / photo.width, box.h / photo.height)));
    } else {
      factor = deviceFactor();
    }
    for (const side of ['left', 'right']) {
      const photo = side === 'left' ? left : right;
      if (!photo) continue;
      const img = sidePanes.querySelector(`[data-image="${side}"]`);
      if (!img) continue;
      setImage(img, photo, factor);
      img.alt = `${sideName(side)}整图：${photoTitle(photo)}`;
    }
    zoomLabel.textContent = zoomDescription(factor);
  }
  function applyBlink() {
    const [left, right] = pairSides();
    const current = state.blink === 'left' ? left : right;
    blinkImages.left.hidden = state.blink !== 'left';
    blinkImages.right.hidden = state.blink !== 'right';
    blinkButton.textContent = `交替显示：当前${sideName(state.blink)}`;
    blinkButton.setAttribute('aria-label', `交替显示左右图像；当前显示${sideName(state.blink)} ${photoTitle(current)}`);
    blinkLabel.textContent = `当前显示：${sideName(state.blink)} ${photoTitle(current)}；手动交替左右图像，仅作视觉对照`;
  }

  function layoutRoi() {
    const entry = selectedRoi();
    if (!entry) return;
    let factor = null;
    for (const side of ['left', 'right']) {
      const stateOf = roiSideState(entry, side);
      if (!stateOf.available) continue;
      const viewport = roiPanes.querySelector(`[data-viewport="${side}"]`);
      const canvas = roiPanes.querySelector(`[data-canvas="${side}"]`);
      const image = roiPanes.querySelector(`[data-image="${side}"]`);
      if (!viewport || !canvas || !image) continue;
      const rect = data.mode === 'single_photo' ? stateOf.roi.rect : stateOf.rect;
      const sideFactor = fitFactor(viewport, rect[2], rect[3]);
      if (sideFactor === null) continue;
      if (factor === null) factor = sideFactor;
      setImage(image, stateOf.photo, factor);
      image.alt = `${sideName(side)}关注区域：${roiLabel(entry.id, side)} · ${photoTitle(stateOf.photo)}`;
      image.style.left = `${-rect[0] * factor}px`;
      image.style.top = `${-rect[1] * factor}px`;
      canvas.style.width = `${rect[2] * factor}px`;
      canvas.style.height = `${rect[3] * factor}px`;
      const locatorImage = roiPanes.querySelector(`[data-locator-image="${side}"]`);
      const locatorRect = roiPanes.querySelector(`[data-locator-rect="${side}"]`);
      if (locatorImage && locatorRect) {
        if (locatorImage.getAttribute('src') !== stateOf.photo.src) locatorImage.src = stateOf.photo.src;
        locatorRect.style.left = `${100 * rect[0] / stateOf.photo.width}%`;
        locatorRect.style.top = `${100 * rect[1] / stateOf.photo.height}%`;
        locatorRect.style.width = `${100 * rect[2] / stateOf.photo.width}%`;
        locatorRect.style.height = `${100 * rect[3] / stateOf.photo.height}%`;
      }
    }
    if (factor !== null) zoomLabel.textContent = zoomDescription(factor);
  }

  function renderWholePanes() {
    while (sidePanes.firstChild) sidePanes.removeChild(sidePanes.firstChild);
    const [left, right] = pairSides();
    const single = data.mode === 'single_photo';
    const build = (side, photo) => {
      const label = single ? photoTitle(photo) : `${sideName(side)} · ${photoTitle(photo)}`;
      return makeViewportPane(side, label, `整图 ${dimensionText(photo)} · ${colorText(photo)}`, false, single ? '' : undefined);
    };
    if (single) {
      sidePanes.appendChild(build('left', left));
      return;
    }
    sidePanes.appendChild(build('left', left));
    sidePanes.appendChild(build('right', right));
  }
  function renderWholePanels() {
    const blink = state.layout === 'blink' && data.mode !== 'single_photo';
    sidePanel.hidden = blink;
    blinkPanel.hidden = !blink;
    layoutGroup.hidden = data.mode === 'single_photo';
    for (const button of layoutButtons) {
      button.setAttribute('aria-pressed', String(button.dataset.layout === state.layout));
    }
    renderWholePanes();
    if (data.mode === 'single_photo') {
      wholeNote.textContent = '单张照片检查：显示副本按源声明转换，关注区域仅为细节查看，不与任何照片对照。';
      return;
    }
    wholeNote.textContent = '两侧照片独立发布，未做配准或对齐；并排与交替仅为视觉对照，不代表像素级对应。';
  }

  function renderRoiControls() {
    const entries = roiEntries();
    roiPanel.hidden = entries.length === 0;
    if (!entries.length) {
      state.roi = null;
      return;
    }
    while (roiSelect.firstChild) roiSelect.removeChild(roiSelect.firstChild);
    const whole = document.createElement('option');
    whole.value = '';
    whole.textContent = '整图（不选择关注区域）';
    roiSelect.appendChild(whole);
    for (const entry of entries) {
      const option = document.createElement('option');
      option.value = entry.id;
      const sides = data.mode === 'single_photo'
        ? [true]
        : ['left', 'right'].map(side => roiSideState(entry, side).available);
      const shared = data.mode !== 'single_photo' && sides[0] && sides[1];
      const marker = data.mode === 'single_photo' ? '' : shared ? '双侧' : '单侧';
      option.textContent = `${roiLabel(entry.id, 'left')}（${entry.id}${marker ? ` · ${marker}` : ''}）`;
      roiSelect.appendChild(option);
    }
    roiSelect.value = state.roi || '';
    roiSelect.setAttribute('aria-label', '选择关注区域');
    roiClearButton.hidden = !state.roi;
    const pair = currentPair();
    const pairText = pair ? `对照 ${pair.id} ` : '';
    roiHint.textContent = data.mode === 'single_photo'
      ? '关注区域是细节视图，不是测量区域；选择后仍显示全图与位置。'
      : `${pairText}的关注区域：相同 ID 即两侧对应声明，矩形可不同；一侧缺失时明确标注不可用。`;
  }

  function renderRoiView() {
    while (roiPanes.firstChild) roiPanes.removeChild(roiPanes.firstChild);
    const entry = selectedRoi();
    roiWarning.hidden = true;
    roiWarning.textContent = '';
    if (!entry) {
      roiHeading.textContent = '关注区域';
      return;
    }
    roiHeading.textContent = `关注区域：${roiLabel(entry.id, 'left')}`;
    const states = {};
    for (const side of ['left', 'right']) states[side] = roiSideState(entry, side);
    const oneSided = data.mode !== 'single_photo' && states.left.available !== states.right.available;
    if (data.mode === 'single_photo') {
      roiPanes.appendChild(sidePane('left', states.left, entry.id, ''));
    } else {
      roiPanes.appendChild(sidePane('left', states.left, entry.id));
      roiPanes.appendChild(sidePane('right', states.right, entry.id));
    }
    if (oneSided) {
      const missing = states.left.available ? '右侧' : '左侧';
      roiWarning.hidden = false;
      roiWarning.textContent = `关注区域 ${entry.id} 仅在${states.left.available ? '左侧' : '右侧'}照片上声明，${missing}不可用；不做匹配、裁切或坐标推断。`;
    }
  }

  function renderProvenance() {
    while (provenanceBody.firstChild) provenanceBody.removeChild(provenanceBody.firstChild);
    for (const photo of photos) {
      const row = document.createElement('tr');
      const source = photo.source || {};
      const display = photo.display || {};
      const cells = [
        photoTitle(photo),
        text(source.path) || '未提供',
        text(source.sha256) || '未提供',
        dimensionText(photo),
        colorText(photo),
        `${text(display.path) || '未提供'} · ${displayText(photo)}`,
      ];
      for (const value of cells) {
        const cell = document.createElement('td');
        cell.textContent = value;
        row.appendChild(cell);
      }
      provenanceBody.appendChild(row);
    }
  }

  function renderAll() {
    renderHeader();
    renderPairControls();
    renderMeta();
    renderWholePanels();
    renderRoiControls();
    renderRoiView();
    renderProvenance();
    zoomSelect.value = state.zoom;
    pairSelect.value = currentPair() ? currentPair().id : '';
    layoutWhole();
    layoutRoi();
    syncHash();
  }

  // ---------- 交互 ----------
  function selectPair(id) {
    const pair = pairById(id);
    if (!pair) return;
    state.pair = pair.id;
    state.roi = null;
    renderAll();
  }
  pairSelect.addEventListener('change', () => selectPair(pairSelect.value));
  swapButton.addEventListener('click', () => {
    // 仅交换左右显示顺序，不改变声明的一对照片，也不生成新对照。
    sideOrder = [sideOrder[1], sideOrder[0]];
    renderAll();
  });
  for (const button of layoutButtons) {
    button.addEventListener('click', () => {
      state.layout = button.dataset.layout;
      renderAll();
    });
  }
  zoomSelect.addEventListener('change', () => {
    state.zoom = zoomSelect.value;
    renderAll();
  });
  blinkButton.addEventListener('click', () => {
    state.blink = state.blink === 'left' ? 'right' : 'left';
    applyBlink();
    syncHash();
  });
  roiSelect.addEventListener('change', () => {
    const id = roiSelect.value;
    state.roi = id && roiEntry(id) ? id : null;
    renderAll();
  });
  roiClearButton.addEventListener('click', () => {
    state.roi = null;
    renderAll();
  });
  recoveryButton.addEventListener('click', () => {
    clearFragmentErrors();
    state.pair = data.mode === 'requested_comparisons' ? comparisons[0].id : null;
    state.photo = photos[0].id;
    state.roi = null;
    state.layout = DEFAULT_LAYOUT;
    state.zoom = DEFAULT_ZOOM;
    state.blink = 'left';
    renderAll();
  });
  if (fullscreenButton) {
    fullscreenButton.setAttribute('aria-label', '切换图像查看全屏');
    fullscreenButton.disabled = !viewer || !viewer.requestFullscreen || !document.fullscreenEnabled;
    fullscreenButton.addEventListener('click', async () => {
      try {
        if (document.fullscreenElement === viewer) await document.exitFullscreen();
        else await viewer.requestFullscreen();
      } catch (_) {
        zoomLabel.textContent = '此环境无法进入全屏';
      }
    });
    document.addEventListener('fullscreenchange', () => {
      const active = document.fullscreenElement === viewer;
      fullscreenButton.setAttribute('aria-pressed', String(active));
      fullscreenButton.textContent = active ? '退出全屏' : '全屏';
      layoutWhole();
    });
  }

  // 视口拖动平移。
  function enablePan(viewport) {
    let pan = null;
    viewport.addEventListener('dragstart', event => event.preventDefault());
    viewport.addEventListener('pointerdown', event => {
      if (event.button !== 0 || pan || event.target.closest('button, select, a')) return;
      event.preventDefault();
      viewport.focus({ preventScroll: true });
      pan = { pointer: event.pointerId, x: event.clientX, y: event.clientY, left: viewport.scrollLeft, top: viewport.scrollTop };
      viewport.setPointerCapture(event.pointerId);
    });
    viewport.addEventListener('pointermove', event => {
      if (!pan || pan.pointer !== event.pointerId) return;
      viewport.scrollLeft = pan.left - (event.clientX - pan.x);
      viewport.scrollTop = pan.top - (event.clientY - pan.y);
    });
    const finish = event => {
      if (!pan || pan.pointer !== event.pointerId) return;
      pan = null;
      if (viewport.hasPointerCapture(event.pointerId)) viewport.releasePointerCapture(event.pointerId);
    };
    viewport.addEventListener('pointerup', finish);
    viewport.addEventListener('pointercancel', finish);
    viewport.addEventListener('lostpointercapture', finish);
  }
  enablePan(blinkViewport);

  // 并排视口同步滚动。
  let syncing = false;
  sidePanes.addEventListener('scroll', event => {
    if (syncing) return;
    const pane = event.target.closest('[data-viewport]');
    if (!pane) return;
    const side = pane.dataset.viewport;
    const other = sidePanes.querySelector(`[data-viewport="${side === 'left' ? 'right' : 'left'}"]`);
    if (!other) return;
    syncing = true;
    other.scrollLeft = pane.scrollLeft;
    other.scrollTop = pane.scrollTop;
    requestAnimationFrame(() => { syncing = false; });
  }, true);

  window.addEventListener('resize', () => {
    layoutWhole();
    layoutRoi();
  });
  window.addEventListener('hashchange', () => {
    if (location.hash === lastWrittenHash) return;
    applyHash();
  });

  // ---------- 初始化 ----------
  renderProvenance();
  if (location.hash) applyHash();
  else {
    state.pair = data.mode === 'requested_comparisons' ? comparisons[0].id : null;
    state.photo = photos[0].id;
    renderAll();
  }
})();
