import type { Geometry, LayoutId, NodeId } from './flow-config';

export type SceneId = 'hero' | 'flow';
export type FrameState = {
  width: number;
  height: number;
  dpr: number;
  playing: boolean;
  canvas?: OffscreenCanvas;
} & ({ id: 'hero' } | { id: 'flow'; layout: LayoutId; geometry: Geometry; accent: string });
export type Reply = { type: 'ready'; id: SceneId } | { type: 'failed' }
  | { type: 'processing'; geometryKey: string; nodes: NodeId[] };
