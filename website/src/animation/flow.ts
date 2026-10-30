import { apps, processing, settleDistance, settleDuration, speed, type Box, type Geometry, type LayoutId, type NodeId, type Point } from './flow-config';

const left = ([x, y, , h]: Box): Point => [x, y + h / 2];
const right = ([x, y, w, h]: Box): Point => [x + w, y + h / 2];
const top = ([x, y, w]: Box): Point => [x + w / 2, y];
const bottom = ([x, y, w, h]: Box): Point => [x + w / 2, y + h];
const response = 0.25;
const primary = { opacity: 0.85, halo: 24, tail: 32, width: 2.2 };
const observation = { opacity: 0.55, halo: 0, tail: 32, width: 2 };
const pulse = (progress: number) => progress < 0 || progress > 1 ? 0 : Math.sin(progress * Math.PI);
const smooth = (progress: number) => {
  const value = Math.max(0, Math.min(1, progress));
  return value * value * (3 - 2 * value);
};

function route(vertices: Point[], junctions: Point[] = [], cornerRadius?: number) {
  const corners = vertices.filter(([x, y], i) => i === 0 || x !== vertices[i - 1][0] || y !== vertices[i - 1][1]);
  const points: Point[] = [corners[0]];
  for (let i = 1; i < corners.length - 1; i++) {
    const [x, y] = corners[i];
    const incoming: Point = [x - corners[i - 1][0], y - corners[i - 1][1]];
    const outgoing: Point = [corners[i + 1][0] - x, corners[i + 1][1] - y];
    const turn = incoming[0] * outgoing[1] - incoming[1] * outgoing[0];
    if (turn === 0) continue;
    if (junctions.some(([jx, jy]) => x === jx && y === jy)) {
      points.push([x, y]);
      continue;
    }
    const radius = cornerRadius ?? Math.min(12, Math.hypot(...incoming) / 2, Math.hypot(...outgoing) / 2);
    const start: Point = [x - Math.sign(incoming[0]) * radius, y - Math.sign(incoming[1]) * radius];
    const center: Point = [start[0] + Math.sign(outgoing[0]) * radius, start[1] + Math.sign(outgoing[1]) * radius];
    const angle = Math.atan2(start[1] - center[1], start[0] - center[0]);
    points.push(start);
    for (let step = 1; step <= 8; step++) {
      const at = angle + Math.sign(turn) * Math.PI / 2 * step / 8;
      points.push([center[0] + Math.cos(at) * radius, center[1] + Math.sin(at) * radius]);
    }
  }
  points.push(corners[corners.length - 1]);
  const shape = new Path2D();
  const distances = [0];
  shape.moveTo(...points[0]);
  for (let i = 1; i < points.length; i++) {
    distances.push(distances[i - 1] + Math.hypot(points[i][0] - points[i - 1][0], points[i][1] - points[i - 1][1]));
    shape.lineTo(...points[i]);
  }
  const length = distances[distances.length - 1];
  return {
    shape, length,
    point(distance: number): Point {
      if (distance <= 0) return points[0];
      if (distance >= length) return points[points.length - 1];
      let low = 1;
      while (distances[low] < distance) low++;
      const progress = (distance - distances[low - 1]) / (distances[low] - distances[low - 1]);
      return [points[low - 1][0] + (points[low][0] - points[low - 1][0]) * progress,
        points[low - 1][1] + (points[low][1] - points[low - 1][1]) * progress];
    },
    segment(from: number, to: number) {
      const section = new Path2D();
      section.moveTo(...this.point(from));
      for (let i = 1; i < points.length; i++) {
        if (distances[i] > from && distances[i] < to) section.lineTo(...points[i]);
      }
      section.lineTo(...this.point(to));
      return section;
    },
  };
}

export function createFlow(context: OffscreenCanvasRenderingContext2D, layoutId: LayoutId, geometry: Geometry, accent: string) {
  const translucent = (alpha: number) => `color-mix(in srgb, ${accent} ${alpha * 100}%, transparent)`;
  const narrow = layoutId === 'narrow';
  const { bounds, boxes, cluster, radii } = geometry;
  const edgeRight = bounds[0] + bounds[2];
  const edgeBottom = bounds[1] + bounds[3];
  const lane = narrow ? (bounds[0] + cluster[0]) / 2 : (right(boxes.xhealth)[0] + cluster[0]) / 2;
  const inlet = narrow ? bottom : right;
  const crossbar = (Math.max(bottom(boxes.xhealth)[1], bottom(boxes.pingress)[1]) + cluster[1]) / 2;
  const appInlets = apps.map((id) => left(boxes[id]));
  const junctions: Point[] = narrow
    ? [[bottom(boxes.xhealth)[0], crossbar], ...appInlets.slice(0, 2).map(([, y]): Point => [lane, y])]
    : appInlets.map(([, y]) => [lane, y]);
  const branch = (start: Point, end: Point) => route(narrow
    ? [start, [start[0], crossbar], [lane, crossbar], [lane, end[1]], end]
    : [start, [lane, start[1]], [lane, end[1]], end], junctions);
  const connect = (from: Box, to: Box) => route(narrow ? [bottom(from), top(to)] : [right(from), left(to)]);
  const submit = connect(boxes.developer, boxes.xhealth);
  const request = connect(boxes.visitor, boxes.pingress);
  const releases = appInlets.map((end) => branch(inlet(boxes.xhealth), end));
  const serving = branch(inlet(boxes.pingress), appInlets[1]);
  const tracks = [submit, request, ...releases, serving];
  const observationLane = (right(cluster)[0] + (narrow ? edgeRight : boxes.insight[0])) / 2;
  const insightInlet = left(boxes.insight);
  const insightX = top(boxes.insight)[0];
  const collection = (bottom(cluster)[1] + boxes.insight[1]) / 2;
  const outerLeft = (bounds[0] + Math.min(boxes.xhealth[0], cluster[0], boxes.insight[0])) / 2;
  const outerRight = (Math.max(right(boxes.pingress)[0], right(cluster)[0], right(boxes.insight)[0]) + edgeRight) / 2;
  const outerTop = (bounds[1] + Math.min(boxes.xhealth[1], cluster[1], boxes.insight[1])) / 2;
  const outerBottom = (Math.max(bottom(boxes.pingress)[1], bottom(cluster)[1], bottom(boxes.insight)[1]) + edgeBottom) / 2;
  const observationJunctions: Point[] = narrow
    ? appInlets.slice(1).map(([, y]) => [observationLane, y]) : [[observationLane, insightInlet[1]]];
  const observations = apps.map((id) => {
    const start = right(boxes[id]);
    return route(narrow
      ? [start, [observationLane, start[1]], [observationLane, collection], [insightX, collection], top(boxes.insight)]
      : [start, [observationLane, start[1]], [observationLane, insightInlet[1]], insightInlet], observationJunctions);
  });
  observations.push(narrow
    ? route([right(boxes.pingress), [outerRight, right(boxes.pingress)[1]], [outerRight, insightInlet[1]], right(boxes.insight)])
    : route([bottom(boxes.pingress), [bottom(boxes.pingress)[0], outerBottom], [insightX, outerBottom], bottom(boxes.insight)]));
  observations.push(narrow
    ? route([left(boxes.xhealth), [outerLeft, left(boxes.xhealth)[1]], [outerLeft, insightInlet[1]], insightInlet])
    : route([top(boxes.xhealth), [top(boxes.xhealth)[0], outerTop], [insightX, outerTop], top(boxes.insight)]));
  const paths = [...observations, ...tracks];
  const light = new OffscreenCanvas(48, 28);
  const lightContext = light.getContext('2d');
  if (!lightContext) throw new Error('Flow particle canvas unavailable.');
  for (const [offset, width, height, opacity] of [[-3, 24, 14, 0.4], [0, 8, 5, 0.9]]) {
    lightContext.setTransform(width, 0, 0, height, 30 + offset * 2, 14);
    const gradient = lightContext.createRadialGradient(0, 0, 0, 0, 0, 1);
    for (const [at, strength] of [[0, 1], [0.2, 0.85], [0.5, 0.35], [0.75, 0.08], [1, 0]]) {
      gradient.addColorStop(at, translucent(opacity * strength));
    }
    lightContext.fillStyle = gradient;
    lightContext.fillRect(-1, -1, 2, 2);
  }
  const flight = (path: ReturnType<typeof route>, start: number, target: NodeId, minimum = 0) => {
    const box = boxes[target];
    const [x, y, width, height] = box;
    const terminal = apps.some((app) => app === target);
    const vertical = narrow && !terminal;
    const enter = vertical ? top(box) : left(box);
    const exit = vertical ? bottom(box) : right(box);
    const borders = (vertical ? [x, x + width] : [y, y + height]).map((edge, side) => ({
      path: route(vertical ? [enter, [edge, y], [edge, y + height], exit]
        : [enter, [enter[0], edge], [exit[0], edge], exit], [], radii[target]),
      clip: (vertical
        ? [side ? x + width / 2 : x - primary.halo, y - primary.halo, width / 2 + primary.halo, height + primary.halo * 2]
        : [x - primary.halo, side ? y + height / 2 : y - primary.halo, width + primary.halo * 2, height / 2 + primary.halo]) as Box,
    }));
    const arrival = start + path.length / speed;
    const end = arrival + (borders[0].path.length + (terminal ? settleDistance : 0)) / speed;
    return { path, borders, start, arrival, end, minimum, terminal, target };
  };
  const verified = flight(submit, 0.3, 'xhealth');
  const released = releases.map((path, index) => flight(path, verified.end, apps[index],
    index === 0 ? 0 : releases[index - 1].length - Math.abs(appInlets[index - 1][0] - lane)));
  const routed = flight(request, Math.max(...released.map(({ end }) => end)) + 0.7, 'pingress');
  const completed = flight(serving, routed.end, 'app');
  const flights = [verified, ...released, routed, completed];
  const duration = completed.end + 0.9;

  const particle = (path: ReturnType<typeof route>, distance: number, style: typeof primary) => {
    const point = path.point(distance);
    const before = path.point(distance - 2);
    const after = path.point(distance + 2);
    const scale = style.halo / primary.halo;
    context.save();
    context.globalAlpha = style.opacity;
    context.translate(...point);
    context.rotate(Math.atan2(after[1] - before[1], after[0] - before[0]));
    context.scale(scale, scale);
    context.drawImage(light, -15, -7, 24, 14);
    context.restore();
  };
  const trail = (path: ReturnType<typeof route>, distance: number, style: typeof primary, minimum = 0, limit = path.length) => {
    const from = Math.max(minimum, distance - style.tail);
    const to = Math.min(path.length, distance);
    const end = Math.min(to, limit);
    context.globalAlpha = style.opacity;
    if (end > from) {
      const gradient = context.createLinearGradient(...path.point(from), ...path.point(to));
      for (let step = 0; step <= 4; step++) {
        const at = step / 4;
        const strength = 1 - (distance - from - (to - from) * at) / style.tail;
        gradient.addColorStop(at, translucent(strength * strength));
      }
      context.strokeStyle = gradient;
      context.lineWidth = style.width;
      context.lineCap = end < to || (minimum > 0 && from === minimum) ? 'butt' : 'round';
      context.stroke(path.segment(from, end));
    }
  };
  const glow = (id: NodeId, intensity: number) => {
    if (intensity <= 0) return;
    const [x, y, width, height] = boxes[id];
    context.strokeStyle = accent;
    for (let band = 3; band >= 0; band--) {
      context.globalAlpha = intensity * (band === 0 ? 0.55 : 0.035);
      context.lineWidth = band === 0 ? 1.6 : 2 + band * 3;
      context.beginPath();
      context.roundRect(x, y, width, height, radii[id]);
      context.stroke();
    }
  };

  return (time: number) => {
    const active: NodeId[] = [];
    context.lineCap = 'round';
    context.lineJoin = 'round';
    context.lineWidth = 1.5;
    context.strokeStyle = '#e1e6ef';
    for (const path of paths) context.stroke(path.shape);
    if (time >= 0) {
      context.strokeStyle = accent;
      const cycle = time % duration;
      for (const { path, borders, start, arrival, end, minimum, terminal, target } of flights) {
        if (cycle < start || cycle >= end + (terminal ? 0 : Math.max(primary.tail / speed, processing.fade))) continue;
        const distance = (cycle - start) * speed;
        if (cycle < arrival + primary.tail / speed) trail(path, distance, primary, minimum);
        if (cycle < arrival) {
          if (minimum === 0 || cycle > start + minimum / speed) particle(path, distance, primary);
          continue;
        }
        if (cycle < end - (terminal ? settleDuration : 0)) active.push(target);
        const progress = terminal ? Math.max(0, Math.min(1, (cycle - end + settleDuration) / settleDuration)) : 0;
        const scale = 1 - smooth(progress);
        glow(target, processing.strength * smooth((cycle - arrival) / processing.rise)
          * (terminal ? scale : 1 - smooth((cycle - end) / processing.fade)));
        if (scale <= 0) continue;
        const along = progress > 0
          ? borders[0].path.length - settleDistance * (1 - progress) ** 3 * (1 + progress) : (cycle - arrival) * speed;
        const style = progress > 0 ? {
          opacity: primary.opacity * (1 - smooth((progress - 0.6) / 0.4)),
          halo: primary.halo * scale, tail: primary.tail * scale, width: primary.width * scale,
        } : primary;
        for (const border of borders) {
          context.save();
          context.beginPath();
          context.rect(...border.clip);
          context.clip();
          trail(border.path, along, style);
          if (cycle < end) particle(border.path, along, style);
          context.restore();
        }
      }
      let insightGlow = 0;
      observations.forEach((path, index) => {
        const spacing = 280;
        const distance = (time * 44 + (index < apps.length ? 0 : index * 67)) % spacing;
        const limit = !narrow && index === 2 ? path.length - (insightInlet[0] - observationLane) : path.length;
        for (let at = distance; at < path.length + observation.tail; at += spacing) {
          trail(path, at, observation, 0, limit);
          insightGlow = Math.max(insightGlow, pulse((at - path.length) / (44 * response)));
        }
      });
      glow('insight', insightGlow * 0.18);
    }
    return active;
  };
}
