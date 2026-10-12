"""Embedded, dependency-free assets for the offline benchmark report."""

CSS = r"""
:root { color-scheme: dark; font-family: system-ui, sans-serif; background:#12151b; color:#e6eaf0; }
body { margin:0 auto; max-width:1440px; padding:24px; overflow-wrap:anywhere; }
h1,h2,h3 { line-height:1.2; } a { color:#8dccff; } button,select,input { font:inherit; }
h1 { font-size:1.8rem; margin:0 0 10px; } h2 { font-size:1.25rem; margin:0; } p { margin:10px 0; }
button,select { background:#293242; color:inherit; border:1px solid #607089; padding:6px 10px; border-radius:4px; }
button:focus-visible,a:focus-visible,input:focus-visible,select:focus-visible { outline:3px solid #ffca69; }
section,.card { border:1px solid #394354; border-radius:8px; padding:16px; margin:20px 0; }
pre { white-space:pre-wrap; overflow-wrap:anywhere; background:#1b212b; padding:12px; }
.unavailable { padding:18px; border:1px dashed #ab8d61; color:#ffd397; }
.small,small { color:#aab6c9; font-size:.88em; }
.case-nav { position:sticky; top:0; z-index:3; background:#12151b; padding:8px 0; display:flex; gap:12px; align-items:center; border-bottom:1px solid #394354; }
.case-nav label { min-width:0; } .case-nav select { max-width:100%; }
.case-panel[hidden] { display:none; }
.supporting { margin:16px 0; }
.change-summary { background:#1b212b; padding:12px; border-left:3px solid #8dccff; }
.metric-summary { display:grid; grid-template-columns:repeat(4,minmax(120px,1fr)); gap:10px; margin:14px 0; }
.metric-summary>div { background:#1b212b; padding:10px; border-radius:6px; }
.metric-summary span,.metric-summary strong,.metric-summary small { display:block; }
.metric-summary strong { font-size:1.35rem; margin:4px 0; }
.metric-summary small { line-height:1.35; }
.grid { display:grid; grid-template-columns:repeat(auto-fit,minmax(min(100%,300px),1fr)); gap:16px; }
figure { margin:0; min-width:0; } figure>img { max-width:100%; height:auto; } figcaption { padding:6px 0; }
.controls { display:flex; flex-wrap:wrap; gap:12px; align-items:center; margin:12px 0; }
.panes { display:grid; grid-template-columns:1fr 1fr; gap:12px; }
.panes>div { min-width:0; }
.viewport { overflow:auto; height:clamp(260px,44vh,450px); border:1px solid #536078; background:#090b10; touch-action:none; overscroll-behavior:contain; }
.canvas { position:relative; width:max-content; min-width:1px; transform-origin:top left; }
.canvas img { display:block; max-width:none; image-rendering:pixelated; }
.viewer[data-zoom-mode=fit] .canvas img { image-rendering:auto; }
.viewer[data-zoom-mode=fit] .viewport { display:flex; }
.viewer[data-zoom-mode=fit] .canvas { flex:none; margin:auto; }
.split .candidate { position:absolute; inset:0; clip-path:inset(0 0 0 50%); }
.viewer[data-mode=baseline] .split .candidate { visibility:hidden; }
.viewer[data-mode=candidate] .split .candidate { clip-path:none !important; }
.split-handle { position:absolute; top:0; bottom:0; width:44px; padding:0; border:0; border-radius:0; transform:translateX(-50%); display:flex; align-items:center; justify-content:center; background:transparent; color:#fff; font-size:22px; cursor:ew-resize; touch-action:none; user-select:none; z-index:2; isolation:isolate; }
.split-handle::before { content:''; position:absolute; top:0; bottom:0; left:calc(50% - 1px); width:2px; background:#fff; box-shadow:0 0 2px #000; z-index:-1; }
.split-handle::after { content:''; position:absolute; width:34px; height:36px; border:1px solid #c6d6e9; border-radius:18px; background:#293242; box-shadow:0 1px 5px #0009; z-index:-1; }
.viewer:not([data-mode=split]) .split-handle { display:none; }
.controls input[type=range] { min-width:100px; max-width:100%; accent-color:#8dccff; }
@media(max-width:700px) { .split-handle { width:48px; } .split-handle::after { width:38px; height:40px; } .controls { gap:8px; } }
.viewer[data-mode=side] .split-pane { display:none; }
.viewer:not([data-mode=side]) .side-pane { display:none; }
.viewer:not([data-mode=side]) .panes { display:block; }
.locator { position:relative; display:inline-block; max-width:240px; }
.locator>img { display:block; width:100%; height:auto; }
.locator .rectangle { position:absolute; border:2px solid #ffda68; box-sizing:border-box; pointer-events:none; }
table { border-collapse:collapse; width:100%; } td,th { padding:8px; border-bottom:1px solid #394354; text-align:left; overflow-wrap:anywhere; }
.evidence-plot { display:block; width:min(100%,600px); height:auto; border:1px solid #394354; margin:8px 0; }
@media(max-width:700px) { body { padding:12px; } .case-nav { position:static; align-items:stretch; flex-direction:column; } .case-nav select { display:block; width:100%; box-sizing:border-box; } .panes { grid-template-columns:1fr; } .viewport { height:230px; } table { display:block; overflow:auto; } .metric-summary { grid-template-columns:repeat(2,minmax(120px,1fr)); } }
"""

JS = r"""
'use strict';
const caseNav = document.querySelector('[data-case-nav]');
function refreshViewers() {
  requestAnimationFrame(() => window.dispatchEvent(new Event('resize')));
}
if (caseNav) {
  caseNav.addEventListener('change', () => {
    document.querySelectorAll('[data-case-panel]').forEach(panel => {
      panel.hidden = panel.dataset.casePanel !== caseNav.value;
    });
    refreshViewers();
  });
}
document.querySelectorAll('details').forEach(detail => {
  detail.addEventListener('toggle', () => { if (detail.open) refreshViewers(); });
});
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
      if (syncing) return;
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
"""
