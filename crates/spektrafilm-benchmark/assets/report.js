
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
