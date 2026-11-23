/* Photo report — groups page renderer.
   Authored text is only ever written through textContent/text nodes, so a
   manifest cannot inject markup. The page reads one JSON payload and builds
   the reading surface; there is no view state to reset. */
(function () {
  'use strict';

  var data = JSON.parse(document.getElementById('photo-report-data').textContent);
  var groups = data.groups || [];

  document.getElementById('title').textContent = data.title;
  var meta = document.getElementById('meta');
  if (meta) {
    var photos = {};
    groups.forEach(function (g) {
      (g.items || []).forEach(function (item) { photos[item.photo] = true; });
    });
    meta.textContent = groups.length + ' 组 · ' + Object.keys(photos).length + ' 张';
  }

  var root = document.getElementById('groups');

  function el(tag, className, text) {
    var node = document.createElement(tag);
    if (className) node.className = className;
    if (text != null) node.textContent = text;
    return node;
  }

  /* Authored notes mark emphasis with **; the text itself is never parsed. */
  function note(text) {
    var p = el('p', 'note');
    var parts = String(text || '').split('**');
    parts.forEach(function (part, index) {
      if (!part) return;
      if (index % 2) p.appendChild(el('b', null, part));
      else p.appendChild(document.createTextNode(part));
    });
    return p;
  }

  function figure(className) {
    var f = el('figure', className || null);
    return f;
  }

  function caption(item, values) {
    var cap = el('figcaption');
    cap.appendChild(el('b', null, item.label));
    if (values && values.length) {
      cap.appendChild(document.createElement('br'));
      cap.appendChild(document.createTextNode(values.join(' / ')));
    }
    return cap;
  }

  /* strip: authored crops, painted at true scale, scaled down only when the
     column is narrower than the crop itself. */
  var strips = [];

  function strip(group) {
    var wrap = el('div', 'strip');
    var head = el('div', 'head');
    var crop = group.crop;
    head.appendChild(el('span', null, crop[2] + '×' + crop[3] + ' 裁切 · 同尺度'));
    head.appendChild(el('span', null, (group.metrics || []).join(' / ')));
    wrap.appendChild(head);

    var row = el('div', 'row');
    group.items.forEach(function (item) {
      var f = figure('fig');
      var box = el('div', 'crop');
      box.style.backgroundImage = 'url("' + item.src + '")';
      f.appendChild(box);
      f.appendChild(caption(item, item.values));
      row.appendChild(f);
      strips.push({ box: box, item: item, crop: crop, count: group.items.length });
    });
    wrap.appendChild(row);
    return wrap;
  }

  function paintStrips() {
    strips.forEach(function (entry) {
      var crop = entry.crop;
      var item = entry.item;
      var section = entry.box.closest('section');
      if (!section) return;
      var gap = section.clientWidth < 520 ? 8 : 12;
      // The section, not the row: the row shrink-wraps its tiles, so measuring
      // it here would feed tile size back into the next tile's size.
      var available = section.clientWidth;
      var columns = Math.max(1, Math.min(entry.count, Math.ceil((available + gap) / (crop[2] + gap))));
      var tile = Math.floor((available - gap * (columns - 1)) / columns);
      var scale = Math.min(1, tile / crop[2]);
      var w = crop[2] * scale;
      var h = crop[3] * scale;
      entry.box.style.width = w + 'px';
      entry.box.style.height = h + 'px';
      entry.box.style.backgroundSize = (item.width * scale) + 'px ' + (item.height * scale) + 'px';
      entry.box.style.backgroundPosition = (-crop[0] * scale) + 'px ' + (-crop[1] * scale) + 'px';
      entry.box.classList.toggle('px', scale >= 1);
    });
  }

  function split(group) {
    var f = figure();
    var box = el('div', 'split');
    var a = el('img', 'a');
    a.src = group.items[0].src;
    a.alt = group.items[0].label;
    var b = el('img', 'b');
    b.src = group.items[1].src;
    b.alt = group.items[1].label;
    var divider = el('div', 'divider');
    divider.tabIndex = 0;
    divider.setAttribute('role', 'slider');
    divider.setAttribute('aria-label', '分割位置');
    divider.setAttribute('aria-valuemin', '0');
    divider.setAttribute('aria-valuemax', '100');
    divider.setAttribute('aria-valuenow', '50');
    box.appendChild(a);
    box.appendChild(b);
    box.appendChild(divider);
    f.appendChild(box);

    var sides = el('figcaption', 'sides');
    sides.appendChild(el('span', null, group.items[0].label));
    sides.appendChild(el('span', null, group.items[1].label));
    f.appendChild(sides);
    bindDivider(box, b, divider);
    return f;
  }

  function bindDivider(box, b, divider) {
    function set(p) {
      p = Math.min(1, Math.max(0, p));
      divider.style.left = (p * 100) + '%';
      b.style.clipPath = 'inset(0 0 0 ' + (p * 100) + '%)';
      divider.setAttribute('aria-valuenow', String(Math.round(p * 100)));
    }
    var dragging = false;
    divider.addEventListener('pointerdown', function (event) {
      event.preventDefault();
      dragging = true;
      try { divider.setPointerCapture(event.pointerId); } catch (_) {}
    });
    divider.addEventListener('pointermove', function (event) {
      if (!dragging) return;
      var r = box.getBoundingClientRect();
      set((event.clientX - r.left) / r.width);
    });
    ['pointerup', 'pointercancel'].forEach(function (name) {
      divider.addEventListener(name, function () { dragging = false; });
      window.addEventListener(name, function () { dragging = false; });
    });
    divider.addEventListener('keydown', function (event) {
      var value = Number(divider.getAttribute('aria-valuenow'));
      if (event.key === 'ArrowLeft') { set((value - 2) / 100); event.preventDefault(); }
      if (event.key === 'ArrowRight') { set((value + 2) / 100); event.preventDefault(); }
      if (event.key === 'Home') { set(0); event.preventDefault(); }
      if (event.key === 'End') { set(1); event.preventDefault(); }
    });
    set(0.5);
  }

  /* pair: independent pictures at one shared height, taken from the shortest
     source so nothing is enlarged. */
  function pair(group) {
    var wrap = el('div', 'surface pair');
    var height = Math.min.apply(null, group.items.map(function (item) { return item.height; }));
    group.items.forEach(function (item) {
      var f = figure();
      var img = el('img');
      img.src = item.src;
      img.alt = item.label;
      img.style.height = height + 'px';
      img.style.width = 'auto';
      f.appendChild(img);
      f.appendChild(caption(item));
      wrap.appendChild(f);
    });
    return wrap;
  }

  function single(group) {
    var wrap = el('div', 'surface single');
    var f = figure();
    var img = el('img');
    img.src = group.items[0].src;
    img.alt = group.items[0].label;
    f.appendChild(img);
    f.appendChild(caption(group.items[0]));
    wrap.appendChild(f);
    return wrap;
  }

  groups.forEach(function (group) {
    var section = el('section');
    section.id = group.id;
    section.appendChild(el('h2', null, group.label));
    if (group.note) section.appendChild(note(group.note));
    if (group.layout === 'strip') section.appendChild(strip(group));
    else if (group.layout === 'split') section.appendChild(split(group));
    else if (group.layout === 'pair') section.appendChild(pair(group));
    else section.appendChild(single(group));
    root.appendChild(section);
  });

  paintStrips();
  window.addEventListener('resize', paintStrips);
})();
