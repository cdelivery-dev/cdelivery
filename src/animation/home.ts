import { geometryKey, type Box, type Geometry } from './flow-config';
import type { FrameState, Reply, SceneId } from './protocol';

const wide = matchMedia('(width >= 800px)');
const reduced = matchMedia('(prefers-reduced-motion: reduce)');
const surfaces = Array.from(document.querySelectorAll<HTMLCanvasElement>('canvas[data-animation]'), (canvas) => ({
  canvas, parent: canvas.parentElement!, id: canvas.dataset.animation as SceneId,
  visible: false, near: false, playing: false, transferred: false, previous: '',
}));
const flow = surfaces.find((surface) => surface.id === 'flow');
const nodes = new Map(Array.from(document.querySelectorAll<HTMLElement>('[data-flow-node]'), (node) => [node.dataset.flowNode!, node]));
const boxes = new Map(Array.from(document.querySelectorAll<HTMLElement>('[data-flow-box]'), (node) => [node.dataset.flowBox!, node]));
const cluster = document.querySelector<HTMLElement>('[data-flow-cluster]');
let flowGeometryKey = '';
let worker: Worker | undefined;
let failed = false;
let painted = false;
let suspended = false;
let timeout: number | undefined;
function setProcessing(active: readonly string[], immediate = false) {
  for (const [id, node] of nodes) {
    const processing = active.includes(id);
    if (!immediate && node.hasAttribute('data-processing') === processing) continue;
    node.style.transitionProperty = immediate ? 'none' : '';
    node.toggleAttribute('data-processing', processing);
  }
}
function fallback() {
  failed = true;
  worker?.terminate();
  window.clearTimeout(timeout);
  setProcessing([], true);
  for (const surface of surfaces) {
    surface.parent.removeAttribute('data-ready');
    surface.playing = false;
    surface.canvas.hidden = true;
  }
}
function measureFlow(bounds: DOMRect): Geometry {
  const box = (element: HTMLElement): Box => {
    const { x, y, width, height } = element.getBoundingClientRect();
    return [x - bounds.x, y - bounds.y, width, height];
  };
  const measured = Object.fromEntries(Array.from(boxes, ([id, element]) => [id, box(element)])) as Geometry['boxes'];
  return {
    bounds: [0, 0, bounds.width, bounds.height], cluster: box(cluster!), boxes: measured,
    radii: Object.fromEntries(Array.from(nodes, ([id, element]) => {
      const [, , width, height] = measured[id as keyof typeof measured];
      return [id, Math.min(parseFloat(getComputedStyle(element).borderTopLeftRadius), width / 2, height / 2)];
    })) as Geometry['radii'],
  };
}
function sync() {
  if (failed || !painted) return;
  for (const surface of surfaces) {
    const enabled = surface.id === 'flow' || wide.matches;
    const needed = enabled && surface.near && !document.hidden && !suspended;
    if (!surface.transferred && !needed) continue;
    const bounds = surface.parent.getBoundingClientRect();
    const { width, height } = bounds;
    const dimensions = {
      width, height,
      dpr: Math.min(devicePixelRatio || 1, 2),
      playing: needed && surface.visible && !reduced.matches && width > 0 && height > 0,
    };
    let state: FrameState;
    if (surface.id === 'flow') {
      const style = getComputedStyle(surface.parent);
      const layout = style.getPropertyValue('--flow-layout').trim();
      if (layout !== 'wide' && layout !== 'narrow') return fallback();
      const accent = style.getPropertyValue('--color-accent').trim();
      state = { ...dimensions, id: 'flow', layout, geometry: measureFlow(bounds), accent };
    } else state = { ...dimensions, id: 'hero' };
    const key = JSON.stringify(state);
    if (key === surface.previous) continue;
    surface.previous = key;
    surface.playing = state.playing;
    if (state.id === 'flow') {
      const measured = geometryKey(state.layout, state.geometry);
      if (!state.playing || measured !== flowGeometryKey) setProcessing([], true);
      flowGeometryKey = measured;
    }
    try {
      if (!worker) {
        if (!surface.canvas.transferControlToOffscreen) return fallback();
        worker = new Worker(new URL('./animation.worker.ts', import.meta.url), { type: 'module' });
        worker.onerror = fallback;
        worker.addEventListener('messageerror', fallback);
        worker.onmessage = ({ data }: MessageEvent<Reply>) => {
          if (failed) return;
          if (data.type === 'failed') return fallback();
          if (data.type === 'processing') {
            if (!flow?.playing || data.geometryKey !== flowGeometryKey) return;
            setProcessing(data.nodes);
            return;
          }
          if (data.type === 'ready') {
            surfaces.find((entry) => entry.id === data.id)?.parent.setAttribute('data-ready', '');
            if (surfaces.filter((entry) => entry.transferred).every((entry) => entry.parent.hasAttribute('data-ready'))) window.clearTimeout(timeout);
          }
        };
      }
      if (!surface.transferred) {
        state.canvas = surface.canvas.transferControlToOffscreen();
        worker.postMessage(state, [state.canvas]);
        surface.transferred = true;
        window.clearTimeout(timeout);
        timeout = window.setTimeout(fallback, 10000);
      } else worker.postMessage(state);
    } catch {
      fallback();
      return;
    }
  }
}
const visibility = new IntersectionObserver((entries) => {
  for (const entry of entries) surfaces.find((surface) => surface.parent === entry.target)!.visible = entry.isIntersecting;
  sync();
});
const proximity = new IntersectionObserver((entries) => {
  for (const entry of entries) surfaces.find((surface) => surface.parent === entry.target)!.near = entry.isIntersecting;
  sync();
}, { rootMargin: '200px' });
const resize = new ResizeObserver(sync);
for (const surface of surfaces) {
  visibility.observe(surface.parent);
  proximity.observe(surface.parent);
  resize.observe(surface.parent);
}
for (const element of boxes.values()) resize.observe(element, { box: 'border-box' });
if (cluster) resize.observe(cluster, { box: 'border-box' });
wide.addEventListener('change', sync);
reduced.addEventListener('change', sync);
document.addEventListener('visibilitychange', sync);
window.addEventListener('resize', sync);
function watchResolution() {
  matchMedia(`(resolution: ${devicePixelRatio}dppx)`).addEventListener('change', () => {
    watchResolution();
    sync();
  }, { once: true });
}
watchResolution();
window.addEventListener('pagehide', () => { suspended = true; sync(); });
window.addEventListener('pageshow', () => { suspended = false; sync(); });
document.fonts.ready.then(sync);
requestAnimationFrame(() => requestAnimationFrame(() => { painted = true; sync(); }));
