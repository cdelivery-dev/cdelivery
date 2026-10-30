type Context = OffscreenCanvasRenderingContext2D;
type Draw = (time: number, width: number, height: number) => void;

const tau = Math.PI * 2;
const fraction = (value: number) => value - Math.floor(value);
const between = (min: number, max: number) => min + Math.random() * (max - min);
const direction = () => Math.random() < .5 ? -1 : 1;
const smooth = (value: number) => {
  const progress = Math.max(0, Math.min(1, value));
  return progress * progress * (3 - 2 * progress);
};
const wind = (time: number) => Math.sin(time * .24) + .2 * Math.sin(time * .57 + .8);

function rain(context: Context): Draw {
  const droplet = new Path2D('M1.2-10 0 0');
  const ripple = new Path2D();
  const rippleSteps: [number, number][] = [];
  let startX = -25;
  let startY = -.4;
  ripple.moveTo(startX, startY);
  for (const [cx1, cy1, cx2, cy2, endX, endY] of [
    [-24, -4.3, -11, -6.2, 1, -5.5], [14, -5.8, 27, -2.7, 25, .8],
    [24, 4.9, 10, 6.5, -2, 5.3], [-15, 5.8, -27, 2.9, -25, -.4],
  ]) {
    ripple.bezierCurveTo(cx1, cy1, cx2, cy2, endX, endY);
    let previousX = startX;
    let previousY = startY;
    for (let step = 1; step <= 32; step++) {
      const t = step / 32;
      const u = 1 - t;
      const x = u ** 3 * startX + 3 * u * u * t * cx1 + 3 * u * t * t * cx2 + t ** 3 * endX;
      const y = u ** 3 * startY + 3 * u * u * t * cy1 + 3 * u * t * t * cy2 + t ** 3 * endY;
      rippleSteps.push([x - previousX, y - previousY]);
      previousX = x;
      previousY = y;
    }
    startX = endX;
    startY = endY;
  }
  ripple.closePath();
  const rippleColors = ['#bec3c8', '#b2b8be', '#a6adb4'];
  const ringDelay = .015;
  const launch = (start: number, index: number) => {
    const cycle = index % 2 ? 5.2 : 4;
    const x = (Math.floor(index / 8) + Math.random()) / 2;
    const ground = 45 - Math.random() * 32;
    const splashSize = between(.8, 1.2);
    const ringChoice = Math.random();
    const ringCount = ringChoice < .35 ? 1 : ringChoice < .85 ? 2 : 3;
    const beadCount = Math.floor(between(1, 3));
    const [min, max] = [[.24, .34], [.44, .54], [.7, .82]][ringCount - 1];
    const rippleSize = between(min, max);
    const rings = Array.from({ length: ringCount }, (_, index) => {
      const size = rippleSize * [1, .58, .32][index];
      const x = size * between(.9, 1.1) * direction();
      const angle = between(-3, 3) * Math.PI / 180;
      const y = size * between(.9, 1.1);
      const path = new Path2D();
      path.addPath(ripple, {
        a: Math.cos(angle) * x, b: Math.sin(angle) * x,
        c: -Math.sin(angle) * y, d: Math.cos(angle) * y,
      });
      const perimeter = rippleSteps.reduce((length, [dx, dy]) => length + Math.hypot(dx * x, dy * y), 0);
      const offset = between(0, perimeter);
      const segmentCount = Math.random() < .5 ? 2 : 4;
      const delay = index * ringDelay;
      const duration = .48 * cycle - delay;
      let totalLength = 0;
      const segments = Array.from({ length: segmentCount }, () => {
        const length = between(18, 34);
        const gap = length * between(.35, .65);
        const start = (between(.05, .15) - delay) / duration;
        totalLength += length;
        return { length, gap, start, end: start + between(.18, .3) };
      });
      for (const segment of segments) {
        segment.length *= perimeter / totalLength;
        segment.gap *= perimeter / totalLength;
      }
      return { path, offset, segments };
    });
    const splashDirection = direction();
    const beads = Array.from({ length: beadCount }, (_, index) => ({
      side: between(6, 12) * (index ? -splashDirection : splashDirection),
      rise: between(-14, -7), radius: between(.65, .85),
    }));
    return { start, cycle, x, ground, splashSize, rings, beads };
  };
  const drops: (ReturnType<typeof launch> | undefined)[] = [];
  const zones = [0, 1].map((side) => {
    drops[side * 8] = launch(Infinity, side * 8);
    return { side, next: between(0, .9), cursor: 0 };
  });
  return (time, width, height) => {
    context.lineJoin = 'miter';
    for (const zone of zones) {
      while (time > 0 && time >= zone.next) {
        const offset = zone.side * 8 + zone.cursor;
        drops[offset] = drops[offset + 1] = undefined;
        const count = Math.floor(between(1, 3));
        for (let index = 0; index < count; index++) {
          const drop = launch(zone.next, offset + index);
          if (index) drop.start += between(.12, .25);
          drops[offset + index] = drop;
        }
        zone.cursor = (zone.cursor + 2) % 8;
        zone.next += between(1.5, 2.2);
      }
    }
    for (const [index, drop] of drops.entries()) {
      if (!drop) continue;
      const preview = time === 0 && index % 8 === 0;
      if (!preview && (time < drop.start || time >= drop.start + drop.cycle)) continue;
      const progress = preview ? 0 : (time - drop.start) / drop.cycle;
      const y = height - drop.ground;
      context.save();
      context.translate(drop.x * width, y);
      if (progress < .452) {
        const fall = Math.min(1, progress / .45);
        context.save();
        context.translate(preview ? 8 : 16 * (1 - fall), preview ? -66 : -y * (1 - fall));
        context.globalAlpha = preview ? .9 : progress < .45
          ? .9 * Math.min(1, progress / .02) : .9 * (.452 - progress) / .002;
        context.stroke(droplet);
        context.restore();
      }
      if (preview || progress >= .45 && progress < .93) {
        for (const [index, ring] of drop.rings.entries()) {
          const delay = index * ringDelay;
          const elapsed = (progress - .45) * drop.cycle - delay;
          if (!preview && elapsed <= 0) continue;
          const phase = preview ? 0 : elapsed / (.48 * drop.cycle - delay);
          const fadeStart = .06 + index * .04;
          const spread = preview ? 1 : 1.15 * (.25 + .75 * phase * (1.2 - .2 * phase));
          const depth = drop.rings.length === 1 ? 1 : index / (drop.rings.length - 1);
          context.save();
          context.scale(spread, spread);
          context.lineWidth = (.65 + .15 * depth) / spread;
          context.strokeStyle = rippleColors[drop.rings.length - 1];
          context.globalAlpha = (.78 + .22 * depth) * (preview ? .65 : .85 * smooth(elapsed / .05)
            * (1 - smooth((phase - fadeStart) / (1 - fadeStart))));
          context.lineDashOffset = ring.offset;
          context.setLineDash(ring.segments.flatMap((segment) => {
            const gap = segment.gap * smooth((phase - segment.start) / (segment.end - segment.start));
            return [segment.length - gap, gap];
          }));
          context.stroke(ring.path);
          context.restore();
        }
      }
      if (preview || progress >= .45) {
        const impact = (progress - .45) * drop.cycle;
        context.scale(drop.splashSize, drop.splashSize);
        for (const bead of drop.beads) {
          const flight = Math.sqrt(-8 * bead.rise * drop.splashSize / 180);
          const phase = preview ? .5 : impact / flight;
          if (phase >= 1) continue;
          const vertical = 1 - 2 * phase;
          const stretch = 1 + .5 * vertical * vertical;
          const angle = Math.atan2(4 * bead.rise * vertical, 1.4 * bead.side);
          context.save();
          context.translate(1.4 * bead.side * phase, bead.rise * 4 * phase * (1 - phase));
          context.globalAlpha = preview ? .55 : .9 * smooth(impact / .05) * (1 - smooth((phase - .65) / .35));
          context.beginPath();
          context.ellipse(0, 0, bead.radius * stretch, bead.radius / stretch, angle, 0, tau);
          context.fill();
          context.restore();
        }
      }
      context.restore();
    }
  };
}

function snow(context: Context): Draw {
  const arms = ['M0 0V-2.2M-.48-1.4 0-.92 .48-1.4', 'M0 0V-2.2M-.4-1.18 0-.78 .4-1.18M-.28-1.65 0-1.37 .28-1.65', 'M0 0V-2.2M-.65-1.85 0-1.25 .65-1.85'].map((shape) => new Path2D(shape));
  const flakes = Array.from({ length: 80 }, () => {
    const size = between(.5, 5.5);
    const fall = between(8.8, 12.3);
    const hold = between(.15, .3);
    const fade = between(1.8, 2.6);
    const shape = size <= 1.5 ? -1 : size <= 3 ? 0 : Math.floor(between(0, 3));
    const sprite = shape < 0 ? null : new OffscreenCanvas(32, 32);
    if (sprite) {
      const brush = sprite.getContext('2d');
      if (!brush) throw new Error('Snow sprite context unavailable');
      brush.translate(16, 16);
      brush.scale(4, 4);
      brush.strokeStyle = '#b2b8be';
      brush.lineWidth = (shape === 2 ? .5 : .32) * between(.85, 1);
      brush.lineCap = 'round';
      for (let side = 0; side < 6; side++) {
        brush.stroke(arms[shape]);
        brush.rotate(tau / 6);
      }
    }
    return {
      x: Math.random(), phase: Math.random(), ground: between(13, 30), fall, hold, fade, period: fall + hold + fade,
      size: (size <= 3 ? size : 3 + (size - 3) * 2.2) / 5,
      angle: between(0, tau), omega: between(35, 65) * Math.PI / 180 * direction(), spinPhase: between(0, tau),
      drift: between(-3, 3), sway: between(1, 3), breeze: between(3, 8), sprite, opacity: between(.75, .9),
    };
  });
  return (time, width, height) => {
    for (const flake of flakes) {
      const clock = flake.phase + time / flake.period;
      const age = fraction(clock) * flake.period;
      const elapsed = Math.min(age, flake.fall);
      const landing = Math.max(0, Math.min(1, (elapsed - flake.fall + .25) / .25));
      const motion = elapsed - .125 * landing * landing;
      const falling = motion / (flake.fall - .125);
      const movingTime = (Math.floor(clock) - flake.phase) * flake.period + motion;
      const melt = smooth((age - flake.fall - flake.hold) / flake.fade);
      const x = flake.x * width + flake.drift * falling
        + wind(movingTime) * flake.breeze + Math.sin(movingTime * .7 + flake.phase * tau) * flake.sway;
      const y = falling * (height - flake.ground);
      const size = flake.size * (1 - .12 * melt);
      context.globalAlpha = flake.opacity * Math.min(1, age / .5) * (1 - melt);
      context.save();
      context.translate(x, y);
      context.rotate(flake.angle + flake.omega * (motion + .3 * (Math.sin(motion * .5 + flake.spinPhase) - Math.sin(flake.spinPhase))));
      context.scale(size, size);
      if (flake.sprite) context.drawImage(flake.sprite, -4, -4, 8, 8);
      else {
        context.beginPath();
        context.arc(0, 0, 2.5, 0, tau);
        context.fill();
      }
      context.restore();
    }
  };
}

function kite(context: Context): Draw {
  const cloudStyles = [
    { color: '#d4d9de', width: .48, opacity: .6 },
    { color: '#cdd3d8', width: .6, opacity: .7 },
    { color: '#c4cbd2', width: .8, opacity: .82 },
  ];
  const clouds = [
    { top: 18.7, path: new Path2D('M-21.8 6.4C-30.4 5.8-29.1-6.8-20.7-7.2C-20.1-13.9-14.6-18.7-8.8-18.1C-3.9-18.4 .1-14.6 1.9-9.3M1.2-8.3C7.1-13.3 15.4-10.6 18.7-5.7C20.3-3.4 21.1-.7 20.2 1.6C28.5 1.1 29.8 10.8 21.6 11.7M19.2 12Q9.4 11.1-.8 12.2T-16.8 11.6') },
    { top: 20.2, path: new Path2D('M-26.1 7.1C-31.5 4.9-29-4.5-22.3-3.9C-22.7-10.8-13.5-14.3-8.6-8.4M-9.2-7.3C-7.8-18.4 6.8-20.2 11.4-9.6C19.2-12.3 26.2-6.5 24.1 .5M23.5 .1C31.5 .4 30.5 9.6 23.4 10.3Q14.3 10.8 5.7 10M1.2 10.4Q-9.4 9.6-20.7 10.8') },
    { top: 23.2, path: new Path2D('M-16.2 9.7C-24.5 8.8-25.6-2.7-17.1-4.8C-18.7-10.7-12.9-14.1-8.2-11.7M-8.7-10.8C-10.1-19.5-.8-23.2 5.8-18C9.1-15.5 9.6-11.8 9.1-8.8C17.4-11.1 23.2-4.6 20.2 2.1M19.5 1.8C26.5 3.6 25 11.4 18.3 12.1Q11.1 12.8 3.6 11.9M.1 12.2Q-6.7 11.4-12.8 12') },
  ].map((cloud, index) => {
    const duration = between(72, 96);
    return { ...cloud, start: -duration * (index + between(0, .5)) / 3,
      duration, size: between(.45, 1.35), y: Math.random() };
  });
  const outline = [
    [8.8, -11.8, 16.3, -.9], [9.3, 12.3, -1.1, 24.1],
    [-10.4, 12.9, -16.2, 1.1], [-7.9, -11.2, .8, -20.2],
  ];
  const outlineWidth = .7;
  const radius = Math.max(...outline.flatMap(([cx, cy, x, y]) => [Math.hypot(cx, cy), Math.hypot(x, y)])) + outlineWidth / 2 + 1;
  const body = new Path2D();
  body.moveTo(.8, -20.2);
  for (const [cx, cy, x, y] of outline) body.quadraticCurveTo(cx, cy, x, y);
  body.closePath();
  const shade = new Path2D('M.8-20.2Q-1.3-10.3 0 0Q1.2 11.4-1.1 24.1Q-10.4 12.9-16.2 1.1Q-7.9-11.2 .8-20.2Z');
  const ribs = new Path2D('M.8-20.2Q-1.3-10.3 0 0Q1.2 11.4-1.1 24.1M-16.2 1.1Q-8.6-1.1 0 0Q8.7 1.2 16.3-.9');
  const phase = between(0, tau);
  const duration = between(72, 96);
  const originX = between(.68, .72);
  const originY = between(.05, .15);
  return (time, width, height) => {
    const angle = time * tau * 3 / duration + phase;
    context.strokeStyle = '#c8cdd2';
    for (const cloud of clouds) {
      while (time >= cloud.start + cloud.duration) {
        cloud.start += cloud.duration;
        cloud.duration = between(72, 96);
        cloud.y = Math.random();
        cloud.size = between(.45, 1.35);
      }
      const progress = (time - cloud.start) / cloud.duration;
      const style = cloudStyles[cloud.size < .75 ? 0 : cloud.size < 1.05 ? 1 : 2];
      const minY = cloud.top * cloud.size + style.width / 2 + 4;
      const y = minY + Math.max(0, height * .35 - minY) * cloud.y;
      context.save();
      context.strokeStyle = style.color;
      context.translate(width + 40 - progress * (width + 80), y);
      context.scale(cloud.size, cloud.size);
      context.globalAlpha = style.opacity * Math.min(1, progress * 8, (1 - progress) * 8);
      context.lineWidth = style.width / cloud.size;
      context.stroke(cloud.path);
      context.restore();
    }
    const breeze = Math.sin(angle);
    const sway = Math.sin(angle * 2 + .8);
    const lag = Math.sin(angle - .18);
    const x = width * originX + 44 * breeze + 6 * sway;
    const amplitude = Math.min(30, Math.max(0, height / 2 - radius));
    const baselineY = Math.max(radius + amplitude, Math.min(height - radius - amplitude, height * originY));
    const y = baselineY + amplitude * (.8 * sway + .2 * breeze);
    const anchorY = height * .78;
    context.save();
    context.translate(x, y);
    context.rotate((-18 + 7 * breeze + 2 * sway) * Math.PI / 180);
    context.globalAlpha = 1;
    context.fillStyle = '#fff';
    context.fill(body);
    context.fillStyle = '#f3f5f6';
    context.fill(shade);
    context.strokeStyle = '#bcc2c7';
    context.lineWidth = outlineWidth;
    context.stroke(body);
    context.globalAlpha = .8;
    context.lineWidth = .5;
    context.stroke(ribs);
    context.restore();
    const sag = Math.min(18, (width - x) * .08) * (1 - .2 * lag);
    context.globalAlpha = .7;
    context.lineWidth = .55;
    context.beginPath();
    context.moveTo(x, y);
    context.quadraticCurveTo((x + width) / 2, (y + anchorY) / 2 + sag, width, anchorY);
    context.stroke();
  };
}

function stars(context: Context): Draw {
  const star = new Path2D('M-.3-5Q.7-3.2 1.4-1.7L5-1.2Q3.5.3 2.3 1.2L3 4.6Q1.3 3.7-.2 2.6L-3.5 4.2-2.5.8Q-4-.5-4.8-1.9L-1.6-2.1Z');
  const place = () => ({ x: Math.random(), y: between(.05, .5), size: between(.6, .86), angle: between(0, tau), width: between(.45, .95) });
  const points = Array.from({ length: 36 }, () => {
    const period = between(10, 16);
    return { ...place(), start: -Math.random() * period, period };
  });
  const flight = () => {
    const side = direction();
    const x = between(.45, .9);
    const y = between(.05, 1 / 3);
    return { x: side < 0 ? x : 1 - x, y, dx: side * between(.25, .4), dy: between(.65, .85) - y,
      duration: between(2, 3), wait: between(1, 3) };
  };
  const meteor = { ...flight(), start: between(1, 2) };
  return (time, width, height) => {
    for (const point of points) {
      while (time >= point.start + point.period) {
        point.start += point.period;
        Object.assign(point, place());
      }
      const progress = (time - point.start) / point.period;
      const fading = smooth((progress - .1) / .8);
      if (progress >= .9) continue;
      context.save();
      context.translate(width * point.x, height * point.y);
      context.rotate(point.angle);
      const size = point.size * (1 - .9 * fading);
      context.scale(size, size);
      context.globalAlpha = .9 * smooth(progress / .1) * (1 - fading);
      context.lineWidth = point.width;
      context.stroke(star);
      context.restore();
    }
    while (time >= meteor.start + meteor.duration + meteor.wait) {
      meteor.start += meteor.duration + meteor.wait;
      Object.assign(meteor, flight());
    }
    const progress = (time - meteor.start) / meteor.duration;
    if (progress < 0 || progress >= 1) return;
    const x = width * meteor.x;
    const y = height * meteor.y;
    const dx = width * meteor.dx;
    const dy = height * meteor.dy;
    const distance = Math.hypot(dx, dy);
    const size = 1 - .7 * smooth(progress);
    const tailSize = 1 - .2 * smooth(progress);
    const appearing = smooth(progress / .12);
    const fade = .85 * appearing * (1 - smooth((progress - .3) / .7));
    for (let segment = 8; segment > 0; segment--) {
      const back = Math.max(0, progress - segment * 6 * tailSize / distance);
      const front = Math.max(0, progress - (segment - 1) * 6 * tailSize / distance);
      const fading = smooth((progress - .3 + segment * .018) / (.7 + segment * .018));
      context.globalAlpha = .55 * appearing * (1 - fading) * (1 - segment / 9) ** 2;
      context.lineWidth = (.3 + .5 * (1 - segment / 9)) * tailSize;
      context.beginPath();
      context.moveTo(x + dx * back, y + dy * back);
      context.lineTo(x + dx * front, y + dy * front);
      context.stroke();
    }
    context.translate(x + dx * progress, y + dy * progress);
    context.rotate(Math.atan2(dy, dx));
    context.scale(size, size);
    context.globalAlpha = fade;
    context.lineWidth = .7;
    context.stroke(star);
  };
}

export function createHero(context: Context): Draw {
  const scenes = [rain, snow, kite, stars];
  const draw = scenes[Math.floor(Math.random() * scenes.length)](context);
  return (time, width, height) => {
    context.strokeStyle = context.fillStyle = '#b2b8be';
    context.lineWidth = .8;
    context.lineCap = context.lineJoin = 'round';
    draw(Math.max(0, time), width, height);
  };
}
