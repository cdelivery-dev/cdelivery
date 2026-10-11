export type Point = readonly [number, number];
export type Box = readonly [number, number, number, number];
export const apps = ['app-before', 'app', 'app-after'] as const;
export const actors = ['developer', 'visitor'] as const;
export const nodes = ['xhealth', 'pingress', ...apps, 'insight'] as const;
export type NodeId = typeof nodes[number];
export type LayoutId = 'wide' | 'narrow';
export const speed = 85;
export const settleDistance = 32;
export const settleDuration = settleDistance * 2 / speed;
export const processing = { rise: 0.18, fade: 0.45, strength: 0.25 };
export interface Geometry {
  bounds: Box;
  cluster: Box;
  boxes: Record<typeof actors[number] | NodeId, Box>;
  radii: Record<NodeId, number>;
}
export const geometryKey = (layout: LayoutId, geometry: Geometry) => JSON.stringify([layout, geometry]);
