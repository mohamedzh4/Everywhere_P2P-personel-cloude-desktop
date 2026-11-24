// Page motion: the opening loader and the orbit with its two squares
// (wine = your PC, slate = this browser).

const reduced = matchMedia('(prefers-reduced-motion: reduce)').matches;

/** Plays once per browser tab; later visits in the same tab skip it. */
export function runLoader() {
  const el = document.getElementById('loader');
  if (!el) return;
  let seen = false;
  try { seen = sessionStorage.getItem('everywhere.loaded') === '1'; sessionStorage.setItem('everywhere.loaded', '1'); } catch (_) {}
  if (reduced || seen) { el.classList.add('done'); return; }
  el.addEventListener('animationend', (e) => { if (e.animationName === 'loader-bg') el.classList.add('done'); });
  // Safety net if animations are throttled (background tab).
  setTimeout(() => el.classList.add('done'), 3500);
}

/**
 * The two squares travel the ellipse. Idle: opposite each other, drifting.
 * Connecting: they speed up and close in. Connected: they meet.
 */
export function startOrbit() {
  const svg = document.getElementById('orbit');
  if (!svg) return { set() {} };
  const [pc, web] = svg.querySelectorAll('rect');
  const cx = 500, cy = 260, rx = 488, ry = 246, tilt = -0.12;
  let mode = 'idle', angle = 0.6, gap = Math.PI, last = performance.now();
  const place = (r, a) => {
    const x = rx * Math.cos(a), y = ry * Math.sin(a);
    const px = cx + x * Math.cos(tilt) - y * Math.sin(tilt);
    const py = cy + x * Math.sin(tilt) + y * Math.cos(tilt);
    r.setAttribute('x', (px - 5).toFixed(1));
    r.setAttribute('y', (py - 5).toFixed(1));
  };
  svg.querySelector('ellipse').setAttribute('transform', `rotate(${(tilt * 180 / Math.PI).toFixed(2)} ${cx} ${cy})`);
  const frame = (now) => {
    const dt = Math.min(0.05, (now - last) / 1000);
    last = now;
    const speed = mode === 'connecting' ? 0.9 : mode === 'connected' ? 0.25 : 0.06;
    const target = mode === 'idle' ? Math.PI : mode === 'connecting' ? Math.PI * 0.35 : 0.06;
    angle += speed * dt;
    gap += (target - gap) * Math.min(1, dt * (mode === 'idle' ? 0.8 : 2.2));
    place(pc, angle);
    place(web, angle + gap);
    if (!reduced) requestAnimationFrame(frame);
  };
  if (reduced) { place(pc, angle); place(web, angle + gap); } else requestAnimationFrame(frame);
  return { set(m) { mode = m; if (reduced) { gap = m === 'idle' ? Math.PI : 0.06; place(pc, angle); place(web, angle + gap); } } };
}
