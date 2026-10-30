import { createHero } from './hero';
import { createFlow } from './flow';
import { geometryKey } from './flow-config';
import type { FrameState, Reply, SceneId } from './protocol';

interface Surface {
  context: OffscreenCanvasRenderingContext2D;
  state: FrameState;
  renderer: ReturnType<typeof createHero> | ReturnType<typeof createFlow>;
  time: number;
  processing: string;
  geometryKey: string;
}
const surfaces = new Map<SceneId, Surface>();
let frame = 0;
let lastPaint = 0;
let lastTime = 0;
const reply = (message: Reply) => postMessage(message);
function paint(surface: Surface, sync = false) {
  const { context, state, renderer } = surface;
  const time = state.id === 'flow' && !state.playing ? -1 : surface.time;
  context.setTransform(state.dpr, 0, 0, state.dpr, 0, 0);
  context.clearRect(0, 0, state.width, state.height);
  context.save();
  const nodes = renderer(time, state.width, state.height);
  context.restore();
  if (nodes !== undefined && state.id === 'flow') {
    const key = nodes.join(',');
    if (sync || key !== surface.processing) {
      surface.processing = key;
      reply({ type: 'processing', geometryKey: surface.geometryKey, nodes });
    }
  }
}
function tick(now: number) {
  frame = requestAnimationFrame(tick);
  const elapsed = now - lastPaint;
  if (elapsed < 1000 / 60 - .5) return;
  const delta = lastTime ? Math.min((now - lastTime) / 1000, .05) : 0;
  lastTime = now;
  lastPaint = elapsed < 1000 / 60 ? now : now - elapsed % (1000 / 60);
  for (const surface of surfaces.values()) {
    if (!surface.state.playing) continue;
    surface.time += delta;
    paint(surface);
  }
}
onmessage = ({ data: state }: MessageEvent<FrameState>) => {
  try {
    const key = state.id === 'flow' ? geometryKey(state.layout, state.geometry) : '';
    let surface = surfaces.get(state.id);
    if (!surface) {
      if (!state.canvas || typeof requestAnimationFrame !== 'function') throw new Error('Canvas worker unavailable');
      const context = state.canvas.getContext('2d');
      if (!context) throw new Error('Canvas 2D unavailable');
      state.canvas.addEventListener('contextlost', () => reply({ type: 'failed' }));
      surface = {
        context, state, time: 0, processing: '', geometryKey: key,
        renderer: state.id === 'hero' ? createHero(context) : createFlow(context, state.layout, state.geometry, state.accent),
      };
      surfaces.set(state.id, surface);
    } else if (state.id === 'flow' && (key !== surface.geometryKey
      || (surface.state.id === 'flow' && state.accent !== surface.state.accent))) {
      surface.renderer = createFlow(surface.context, state.layout, state.geometry, state.accent);
      if (key !== surface.geometryKey) {
        surface.time = 0;
        surface.processing = '';
        surface.geometryKey = key;
      }
    }
    surface.state = state;
    const { canvas } = surface.context;
    const width = Math.round(state.width * state.dpr);
    const height = Math.round(state.height * state.dpr);
    if (canvas.width !== width) canvas.width = width;
    if (canvas.height !== height) canvas.height = height;
    paint(surface, true);
    if (state.canvas) reply({ type: 'ready', id: state.id });
    const playing = [...surfaces.values()].some((entry) => entry.state.playing);
    if (playing && !frame) frame = requestAnimationFrame(tick);
    if (!playing) {
      cancelAnimationFrame(frame);
      frame = lastPaint = lastTime = 0;
    }
  } catch {
    reply({ type: 'failed' });
  }
};
