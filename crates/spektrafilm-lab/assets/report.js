'use strict';
// The page is a magnifying glass over photographs. There is one surface per
// view, a divider to wipe between reference and variant, and direct gestures:
// wheel zooms at the pointer, drag pans, space shows the other side.
const panels = [...document.querySelectorAll('[data-case-panel]')];
const casesBox = document.querySelector('[data-cases]');

function active(index, updateHash) {
  if (!panels.length) return;
  const target = Math.max(0, Math.min(panels.length - 1, index));
  panels.forEach((panel, i) => { panel.hidden = i !== target; });
  if (casesBox) {
    casesBox.querySelectorAll('[data-case-option]').forEach((option, i) => {
      option.classList.toggle('active', i === target);
      option.setAttribute('aria-current', String(i === target));
    });
    const name = casesBox.querySelector('[data-case-name]');
    const chosen = casesBox.querySelectorAll('[data-case-option]')[target];
    if (name && chosen) name.textContent = chosen.textContent.trim();
  }
  // A panel that was hidden when it was set up measured a zero-width stage.
  // Now that it has a box, let every viewer re-fit.
  if (!panels[target].hidden) window.dispatchEvent(new Event('resize'));
  if (updateHash) {
    const panel = panels[target];
    const view = panel.querySelector('.views button[aria-pressed=true]');
    const hash = `#case=${encodeURIComponent(panel.dataset.caseId)}${view ? `&view=${encodeURIComponent(view.dataset.view)}` : ''}`;
    if (location.hash !== hash) history.replaceState(null, '', hash);
  }
}

function fromHash() {
  const values = new Map();
  for (const part of location.hash.replace(/^#/, '').split('&')) {
    const [key, value] = part.split('=');
    if (key && value !== undefined) values.set(key, decodeURIComponent(value));
  }
  const index = panels.findIndex(panel => panel.dataset.caseId === values.get('case'));
  return { index, view: values.get('view') };
}

// Bootstrap owns the case menu (open, close, placement, keyboard). The page only
// has to say what a choice means.
if (casesBox) {
  casesBox.querySelector('[data-case-menu]').addEventListener('click', event => {
    const option = event.target.closest('[data-case-option]');
    if (!option) return;
    active(Number(option.dataset.caseOption), true);
    // Bootstrap closes the menu but leaves focus on <body>; a keyboard reader
    // would restart from the top of the page.
    casesBox.querySelector('[data-case-button]').focus();
  });
}

for (const panel of panels) {
  const views = [...panel.querySelectorAll('button[data-view]')];
  const stages = [...panel.querySelectorAll('[data-stage]')];
  const variants = [...panel.querySelectorAll('[data-variant]')];
  const note = panel.querySelector('[data-view-note]');
  const rail = panel.querySelector('.views');
  const syncRail = () => rail?.toggleAttribute('data-scroll',
    rail.scrollHeight > rail.clientHeight + 1 || rail.scrollWidth > rail.clientWidth + 1);
  syncRail();
  const state = { scale: null, pan: { x: 0, y: 0 }, split: 50, view: 0 };

  // A view is a surface (the print plus its bar); the stage inside it carries
  // the crop. Both the index and the visibility live on the surface.
  const wrapper = stage => stage.closest('[data-surface]') || stage;
  const visible = () => stages.filter(stage => !wrapper(stage).hidden);
  const geometry = stage => {
    const image = stage.querySelector('[data-plane=reference]');
    const [x, y, width, height] = (stage.dataset.crop || '')
      .split(',')
      .map(Number);
    return {
      x: x || 0,
      y: y || 0,
      width: width || image.naturalWidth || 1,
      height: height || image.naturalHeight || 1,
    };
  };
  const fitScale = stage => {
    const g = geometry(stage);
    return Math.min(stage.clientWidth / g.width, stage.clientHeight / g.height);
  };
  // Only scales that never shrink below fit. A 300px frame in a 620px stage
  // starts at 2.07x, so 100% is not an option there; a 13px window starts at
  // 47x and has no larger step at all, which is why the chip can be inert.
  const steps = stage => {
    const fit = fitScale(stage);
    const all = [fit, 1, 2, 4].filter(value => value >= fit * 0.999);
    return [...new Set(all.map(value => Math.round(value * 1000) / 1000))];
  };

  function clamp(stage, g) {
    const width = g.width * state.scale;
    const height = g.height * state.scale;
    state.pan.x = width <= stage.clientWidth
      ? (stage.clientWidth - width) / 2
      : Math.min(0, Math.max(stage.clientWidth - width, state.pan.x));
    state.pan.y = height <= stage.clientHeight
      ? (stage.clientHeight - height) / 2
      : Math.min(0, Math.max(stage.clientHeight - height, state.pan.y));
  }

  function paint() {
    for (const stage of visible()) {
      const g = geometry(stage);
      if (state.scale === null) state.scale = fitScale(stage);
      clamp(stage, g);
      const native = state.scale >= 0.999;
      for (const plane of stage.querySelectorAll('.plane')) {
        plane.style.transform = `translate(${state.pan.x - g.x * state.scale}px,${state.pan.y - g.y * state.scale}px) scale(${state.scale})`;
        plane.style.imageRendering = native ? 'pixelated' : 'auto';
        const want = native ? plane.dataset.native : plane.dataset.fit;
        if (want && plane.getAttribute('src') !== want) plane.src = want;
      }
      const fit = Math.abs(state.scale - fitScale(stage)) < 0.005;
      stage.classList.toggle('zoomed', !fit);
      const chip = wrapper(stage).querySelector('[data-scale]');
      if (chip) {
        chip.textContent = fit ? '适应' : `${Math.round(state.scale * 100)}%`;
        chip.disabled = steps(stage).length < 2;
      }
    }
  }

  function split(value) {
    state.split = Math.max(0, Math.min(100, value));
    for (const plane of panel.querySelectorAll('.plane.variant')) {
      plane.style.clipPath = `inset(0 0 0 ${state.split}%)`;
    }
    for (const divider of panel.querySelectorAll('[data-divider]')) {
      divider.style.left = `${state.split}%`;
      divider.setAttribute('aria-valuenow', String(Math.round(state.split)));
    }
  }

  function showView(index) {
    state.view = index;
    stages.forEach(stage => {
      wrapper(stage).hidden = Number(wrapper(stage).dataset.view) !== index;
    });
    panel.querySelectorAll('.stages-pair').forEach(pair => {
      pair.hidden = Number(pair.dataset.view) !== index;
    });
    views.forEach((button, i) => button.setAttribute('aria-pressed', String(i === index)));
    state.scale = fitScale(visible()[0] || stages[0]);
    state.pan = { x: 0, y: 0 };
    paint();
    if (note) {
      const text = views[index] ? views[index].dataset.note || '' : '';
      note.textContent = text;
      note.hidden = !text;
    }
    const current = views[index];
    const hash = `#case=${encodeURIComponent(panel.dataset.caseId)}${current ? `&view=${encodeURIComponent(current.dataset.view)}` : ''}`;
    if (location.hash !== hash) history.replaceState(null, '', hash);
  }

  function showVariant(index) {
    const chip = variants[index];
    if (!chip) return;
    for (const plane of panel.querySelectorAll('.plane.variant')) {
      plane.src = chip.dataset.fit;
      plane.dataset.fit = chip.dataset.fit;
      plane.dataset.native = chip.dataset.native;
    }
    variants.forEach((button, i) => button.setAttribute('aria-selected', String(i === index)));
    for (const tag of panel.querySelectorAll('.edge-right')) tag.textContent = chip.textContent.trim();
    state.scale = fitScale(visible()[0] || stages[0]);
    paint();
  }

  for (const button of views) {
    button.addEventListener('click', () => showView(Number(button.dataset.view)));
  }
  for (const button of variants) {
    button.addEventListener('click', () => showVariant(Number(button.dataset.variant)));
  }

  for (const stage of stages) {
    stage.addEventListener('wheel', event => {
      event.preventDefault();
      const box = stage.getBoundingClientRect();
      const factor = Math.exp(-event.deltaY * 0.0018);
      // Never below fit; a tiny crop already sits above the 16x ceiling.
      const next = Math.min(
        Math.max(fitScale(stage), state.scale * factor),
        Math.max(16, fitScale(stage))
      );
      // Keep the source pixel under the pointer where it is.
      const px = event.clientX - box.left;
      const py = event.clientY - box.top;
      state.pan.x = px - ((px - state.pan.x) * next) / state.scale;
      state.pan.y = py - ((py - state.pan.y) * next) / state.scale;
      state.scale = next;
      paint();
    }, { passive: false });

    // Drag pans; two fingers pinch around their midpoint, the same gesture the
    // rest of the phone already taught the reader.
    const pointers = new Map();
    let pan = null;
    let pinch = null;
    stage.addEventListener('pointerdown', event => {
      if (event.target.closest('[data-divider], button')) return;
      pointers.set(event.pointerId, { x: event.clientX, y: event.clientY });
      try { stage.setPointerCapture(event.pointerId); } catch (_) { /* synthetic pointer */ }
      if (pointers.size === 2) {
        const [a, b] = [...pointers.values()];
        pinch = { distance: Math.hypot(a.x - b.x, a.y - b.y) || 1, scale: state.scale };
        pan = null;
        return;
      }
      if (event.button !== 0) return;
      pan = { id: event.pointerId, x: event.clientX, y: event.clientY, px: state.pan.x, py: state.pan.y };
    });
    stage.addEventListener('pointermove', event => {
      if (pointers.has(event.pointerId)) {
        pointers.set(event.pointerId, { x: event.clientX, y: event.clientY });
      }
      if (pinch && pointers.size === 2) {
        const [a, b] = [...pointers.values()];
        const box = stage.getBoundingClientRect();
        const midX = (a.x + b.x) / 2 - box.left;
        const midY = (a.y + b.y) / 2 - box.top;
        const fit = fitScale(stage);
        const distance = Math.hypot(a.x - b.x, a.y - b.y) || 1;
        const next = Math.min(
          Math.max(fit, pinch.scale * (distance / pinch.distance)),
          Math.max(16, fit)
        );
        const k = next / state.scale;
        state.pan.x = midX - (midX - state.pan.x) * k;
        state.pan.y = midY - (midY - state.pan.y) * k;
        state.scale = next;
        paint();
        return;
      }
      if (!pan || pan.id !== event.pointerId) return;
      state.pan.x = pan.px + (event.clientX - pan.x);
      state.pan.y = pan.py + (event.clientY - pan.y);
      paint();
    });
    const end = event => {
      pointers.delete(event.pointerId);
      if (pointers.size < 2) pinch = null;
      if (pan && pan.id === event.pointerId) pan = null;
    };
    stage.addEventListener('pointerup', end);
    stage.addEventListener('pointercancel', end);
    stage.addEventListener('lostpointercapture', end);
    stage.addEventListener('dblclick', event => {
      if (event.target.closest('[data-divider], button')) return;
      const list = steps(stage);
      const current = list.indexOf(state.scale);
      state.scale = current === list.length - 1 || current === -1 ? fitScale(stage) : list[Math.max(0, current + 1)];
      state.pan = { x: 0, y: 0 };
      paint();
    });

    const chip = wrapper(stage).querySelector('[data-scale]');
    if (chip) {
      chip.addEventListener('click', () => {
        const list = steps(stage);
        const current = list.indexOf(state.scale);
        state.scale = current === list.length - 1 || current === -1 ? fitScale(stage) : list[current + 1];
        state.pan = { x: 0, y: 0 };
        paint();
      });
    }
    const fullscreen = wrapper(stage).querySelector('[data-fullscreen]');
    if (fullscreen) {
      fullscreen.addEventListener('click', async () => {
        if (document.fullscreenElement === stage) await document.exitFullscreen();
        else await stage.requestFullscreen();
      });
    }
  }

  // One set of handlers for every stage's divider. The first version bound the
  // first divider only, so the eight crop views showed a slider that ignored
  // both the pointer and the arrow keys.
  const stagesRoot = panel.querySelector('.stages');
  let wipe = null;
  function setWipe(divider, clientX) {
    const box = divider.parentElement.getBoundingClientRect();
    split(((clientX - box.left) / box.width) * 100);
    for (const stage of stages) stage.dataset.touched = 'true';
  }
  stagesRoot.addEventListener('pointerdown', event => {
    const divider = event.target.closest('[data-divider]');
    if (!divider) return;
    wipe = { id: event.pointerId, divider };
    try { divider.setPointerCapture(event.pointerId); } catch (_) { /* synthetic pointer */ }
    event.preventDefault();
  });
  stagesRoot.addEventListener('pointermove', event => {
    if (!wipe || wipe.id !== event.pointerId) return;
    setWipe(wipe.divider, event.clientX);
  });
  const endWipe = event => { if (wipe && wipe.id === event.pointerId) wipe = null; };
  stagesRoot.addEventListener('pointerup', endWipe);
  stagesRoot.addEventListener('pointercancel', endWipe);
  stagesRoot.addEventListener('lostpointercapture', endWipe);
  stagesRoot.addEventListener('keydown', event => {
    const divider = event.target.closest('[data-divider]');
    if (!divider) return;
    const step = event.shiftKey ? 10 : 2;
    if (event.key === 'ArrowLeft') setWipe(divider, divider.getBoundingClientRect().left - step);
    else if (event.key === 'ArrowRight') setWipe(divider, divider.getBoundingClientRect().left + step);
    else if (event.key === 'Home') split(0);
    else if (event.key === 'End') split(100);
    else return;
    event.preventDefault();
  });

  panel.addEventListener('keydown', event => {
    if (event.ctrlKey || event.metaKey || event.altKey) return;
    const typing = event.target.closest('input, select, textarea, a[href], [contenteditable]');
    // Hold Space to see the other side, wherever focus is; Enter still
    // activates a focused control.
    if (event.code === 'Space' && stagesRoot.querySelector('[data-divider]') && !typing) {
      event.preventDefault();
      if (event.repeat) return;
      panel.dataset.restore = String(state.split);
      split(0);
      return;
    }
    if (event.target.closest('button, [contenteditable]')) return;
    if (event.key === 'f') {
      state.scale = fitScale(visible()[0] || stages[0]);
      state.pan = { x: 0, y: 0 };
      paint();
      event.preventDefault();
    }
  });
  panel.addEventListener('keyup', event => {
    if (event.code === 'Space' && panel.dataset.restore) {
      split(Number(panel.dataset.restore));
    }
  });

  window.addEventListener('resize', () => {
    if (panel.hidden) return;
    syncRail();
    state.scale = fitScale(visible()[0] || stages[0]);
    state.pan = { x: 0, y: 0 };
    paint();
  });

  split(state.split);
  paint();
}

const start = fromHash();
active(start.index >= 0 ? start.index : 0, false);
if (start.index >= 0 && start.view) {
  const panel = panels[start.index];
  const button = [...panel.querySelectorAll('button[data-view]')].find(
    item => item.dataset.view === start.view || item.textContent.trim() === start.view
  );
  if (button) button.click();
}
