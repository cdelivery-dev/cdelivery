(() => {
  "use strict";

  const timezoneKey = "cdelivery:observability:timezone:v1";
  const quantityRules = new Intl.PluralRules(document.documentElement.lang);
  const quantity = (value, forms) => {
    const singular = Number.isSafeInteger(value)
      && value >= 0
      && quantityRules.select(value) === "one";
    return singular ? forms.one : forms.other;
  };
  const timezoneSelect = document.querySelector("[data-observability-timezone]");
  const browserZone = Intl.DateTimeFormat().resolvedOptions().timeZone;
  const wallOptions = {
    hourCycle: "h23",
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  };
  let zone;
  let wallFormat;
  try {
    zone = timezoneSelect ? localStorage.getItem(timezoneKey) || undefined : undefined;
    wallFormat = new Intl.DateTimeFormat("en-CA", { ...wallOptions, timeZone: zone });
  } catch {
    zone = undefined;
    try { localStorage.removeItem(timezoneKey); } catch {}
    wallFormat = new Intl.DateTimeFormat("en-CA", wallOptions);
  }

  const wall = (date) =>
    Object.fromEntries(wallFormat.formatToParts(date).map((part) => [part.type, part.value]));

  const wallEpoch = (ms) => {
    const p = wall(new Date(ms));
    return Date.UTC(
      Number(p.year), Number(p.month) - 1, Number(p.day),
      Number(p.hour), Number(p.minute), Number(p.second),
    );
  };

  const localInstant = (value) => {
    const date = new Date(value);
    if (Number.isNaN(date.valueOf())) return "";
    const p = wall(date);
    return `${p.year}-${p.month}-${p.day}T${p.hour}:${p.minute}`;
  };

  const compatibleInstant = (value) => {
    const target = Date.parse(`${value}Z`);
    if (Number.isNaN(target)) return Number.NaN;
    const offsets = new Set([-48, -24, 0, 24, 48].map((hours) => {
      const sample = target + hours * 3_600_000;
      return wallEpoch(sample) - sample;
    }));
    return [...offsets]
      .map((offset) => target - offset)
      .map((at) => ({ at, delta: wallEpoch(at) - target }))
      .filter(({ delta }) => delta >= 0)
      .sort((left, right) => left.delta - right.delta || left.at - right.at)[0]?.at
      ?? Number.NaN;
  };

  const instant = (value) => {
    const ms = zone ? compatibleInstant(value) : new Date(value).getTime();
    return Number.isNaN(ms) ? "" : new Date(ms).toISOString();
  };

  const formatInstant = (value) => {
    const date = new Date(value);
    if (Number.isNaN(date.valueOf())) return value;
    return date.toLocaleString(document.documentElement.lang, zone ? { timeZone: zone } : undefined);
  };

  const zoneLabel = (name) => {
    const offset = (new Intl.DateTimeFormat("en", { timeZone: name, timeZoneName: "longOffset" })
      .formatToParts(new Date())
      .find((part) => part.type === "timeZoneName")?.value ?? "GMT")
      .replace("GMT", "");
    return `${offset || "+00:00"} ${name}`;
  };

  const initTimezone = (select) => {
    if (browserZone && zone !== browserZone) select.options[0].textContent = zoneLabel(browserZone);
    if (!zone && browserZone === "UTC") select.options[1].remove();
    if (zone && zone !== "UTC") select.append(new Option(zoneLabel(zone), zone));
    select.value = zone ?? "";
  };

  const zoneOptions = (select) => {
    if (select.dataset.filled || !Intl.supportedValuesOf) return;
    select.dataset.filled = "true";
    for (const name of Intl.supportedValuesOf("timeZone")) {
      if (name === "UTC" || name === browserZone || name === select.value) continue;
      select.append(new Option(zoneLabel(name), name));
    }
  };

  const saveTimezone = (select) => {
    try {
      if (select.value) localStorage.setItem(timezoneKey, select.value);
      else localStorage.removeItem(timezoneKey);
      location.reload();
    } catch {
      select.value = zone ?? "";
    }
  };

  const timeRange = (start, end, range) => ({
    start: localInstant(start),
    end: localInstant(end),
    range,
    instant,
    submit() {
      this.$nextTick(() => this.$root.requestSubmit());
    },
    preset(token) {
      this.range = token;
      this.submit();
    },
    custom() {
      this.range = null;
      this.submit();
    },
  });

  const chartFillAlpha = 0.12;
  const statFillAlpha = 0.18;

  const bytes = (value) => {
    const units = ["B", "KiB", "MiB", "GiB", "TiB"];
    let unit = 0;
    let scaled = value;
    while (scaled >= 1024 && unit < units.length - 1) {
      scaled /= 1024;
      unit += 1;
    }
    return `${scaled.toFixed(scaled >= 10 || unit === 0 ? 0 : 1)} ${units[unit]}`;
  };

  const duration = (value) => {
    const minute = 60;
    const hour = 60 * minute;
    const day = 24 * hour;
    if (value >= day) return `${(value / day).toFixed(1)} d`;
    if (value >= hour) return `${(value / hour).toFixed(1)} h`;
    if (value >= minute) return `${(value / minute).toFixed(1)} min`;
    if (value >= 1 || value === 0) return `${value.toFixed(value >= 10 || value === 0 ? 0 : 1)} s`;
    if (value >= 0.001) return `${(value * 1000).toFixed(1)} ms`;
    return `${(value * 1_000_000).toFixed(0)} µs`;
  };

  const decimal = (value) => value.toFixed(4).replace(/\.?0+$/, "");

  const percentDigits = (scaled, span) => {
    const plain = scaled !== 0 && Math.abs(scaled) < 0.1 ? 2 : 1;
    if (!(span > 0)) return plain;
    return Math.min(6, Math.max(plain, Math.ceil(-Math.log10(span)) + 1));
  };

  const percentValue = (value, capacity, graph) => {
    const scaled = value * 100 / capacity;
    const range = graph?.yAxisRange?.();
    const span = range ? Math.abs(range[1] - range[0]) * 100 / capacity : 0;
    return `${scaled.toFixed(percentDigits(scaled, span))}%`;
  };

  const percent = (value, ...args) => percentValue(value, 1, args[2]);

  const formats = Object.freeze({
    bytes,
    "bytes-rate": (value) => `${bytes(value)}/s`,
    cores: decimal,
    percent,
    duration,
    events: (value) => `${value.toFixed(3)}/s`,
    count: (value) => value.toFixed(value < 10 ? 1 : 0),
  });

  const unitFormat = (unit, coreUnit) => {
    const format = formats[unit] ?? decimal;
    if (unit !== "cores" || !coreUnit) return format;
    return (value) => `${format(value)} ${coreUnit}`;
  };

  const detailFormat = (unit, coreUnit) =>
    ["bytes", "percent", "cores"].includes(unit) ? unitFormat(unit, coreUnit) : null;

  const summary = (format, total, capacity, rootData) => {
    const used = [rootData.totalLabel, format(total)].filter(Boolean).join(" ");
    if (!(capacity > 0)) return used;
    return `${used} / ${format(capacity)} (${percent(total / capacity)})`;
  };

  let cachedPalette;

  const paletteColors = () => {
    const style = getComputedStyle(document.body);
    const colors = [];
    for (let i = 1; i <= 10; i += 1) {
      const color = style.getPropertyValue(`--chart-${i}`).trim();
      if (color) colors.push(color);
    }
    const stat = style.getPropertyValue("--chart-stat").trim();
    const neutral = style.getPropertyValue("--chart-neutral").trim();
    const faint = style.getPropertyValue("--faint").trim();
    const key = [style.color, style.backgroundColor, ...colors, stat, neutral, faint].join("\0");
    if (cachedPalette?.key === key) return cachedPalette;
    const probe = document.createElement("span");
    probe.hidden = true;
    document.body.append(probe);
    const resolve = (color) => {
      probe.style.color = color;
      return getComputedStyle(probe).color;
    };
    cachedPalette = {
      key,
      colors: colors.map(resolve),
      stat: resolve(stat),
      neutral: resolve(neutral),
      faint: resolve(faint),
    };
    probe.remove();
    return cachedPalette;
  };

  const stackedLowerBound = (graph) => {
    const [start, end] = graph.xAxisRange();
    const rows = graph.numRows();
    const columns = graph.numColumns();
    let min = Infinity;

    for (let row = 0; row < rows; row += 1) {
      const before = graph.getValue(Math.max(0, row - 1), 0);
      const after = graph.getValue(Math.min(rows - 1, row + 1), 0);
      if (after < start || before > end) continue;

      for (let column = columns - 1; column > 0; column -= 1) {
        const value = graph.getValue(row, column);
        if (!Number.isFinite(value)) continue;
        min = Math.min(min, value);
        break;
      }
    }

    return Number.isFinite(min) ? min * 0.95 : null;
  };

  const createGraph = (chart, options, statLabels, rootData, initialWindow) => {
    const rows = JSON.parse(chart.dataset.metric);
    const seriesLabels = chart.dataset.series ? JSON.parse(chart.dataset.series) : null;
    const seriesKinds = chart.dataset.seriesKinds ? JSON.parse(chart.dataset.seriesKinds) ?? [] : [];
    const minMax = !seriesLabels && !chart.dataset.statLabels;
    const plot = document.createElement("div");
    plot.className = "metric-chart-plot";
    const legend = document.createElement("div");
    legend.className = "dygraph-legend";
    chart.replaceChildren(plot, legend);
    const format = formats[chart.dataset.unit] ?? decimal;
    const interactionModel = xInteraction(initialWindow, plot);
    const themed = () => {
      const fontSize = parseFloat(getComputedStyle(chart).fontSize);
      const { colors, stat, neutral, faint } = paletteColors();
      let series = [colors[0] ?? neutral];
      if (minMax) series = [stat];
      else if (seriesLabels) {
        series = seriesLabels.map((_, i) => colors[i % colors.length] ?? neutral);
        series = series.map((color, index) => {
          if (seriesKinds[index] === "Rest") return neutral;
          if (seriesKinds[index] === "Host") return stat;
          return color;
        });
      }
      return {
        colors: series,
        axisLineColor: faint,
        axisLabelFontSize: fontSize,
        axes: {
          x: { axisLabelWidth: 6 * fontSize, pixelsPerLabel: 6 * fontSize },
          y: { axisLabelWidth: 5.5 * fontSize },
        },
      };
    };
    const axes = {
      x: {
        drawGrid: false,
      },
      y: {
        axisTickSize: 0,
        drawGrid: false,
        valueFormatter: format,
        axisLabelFormatter: format,
      },
    };
    if (zone) {
      axes.x.axisLabelFormatter = (date, granularity) => {
        const p = wall(date);
        const time = `${p.hour}:${p.minute}`;
        if (granularity >= Dygraph.Granularity.DAILY) {
          const day = `${p.year}-${p.month}-${p.day}`;
          return time === "00:00" ? day : `${day} ${time}`;
        }
        return granularity < Dygraph.Granularity.MINUTELY ? `${time}:${p.second}` : time;
      };
      axes.x.valueFormatter = (ms) => {
        const p = wall(new Date(ms));
        return `${p.year}-${p.month}-${p.day} ${p.hour}:${p.minute}:${p.second}`;
      };
    }
    const sharedOptions = {
      ...options,
      resizable: "passive",
      plugins: [{
        activate(graph) {
          return {
            layout: () => {
              const appearance = themed();
              if (graph.getOption("stackedGraph")) {
                appearance.valueRange = [stackedLowerBound(graph), null];
              }
              graph.updateOptions(appearance, true);
            },
          };
        },
      }],
      interactionModel,
      plotter: [
        Dygraph.Plotters.fillPlotter,
        Dygraph.Plotters.errorPlotter,
        chart.dataset.filesystems === "true" ? Dygraph.Plotters.linePlotter : Dygraph.smoothPlotter,
      ],
      drawPoints: false,
      drawGapEdgePoints: true,
      labelsDiv: legend,
      legend: "follow",
    };
    if (seriesLabels) {
      const stacked = chart.dataset.stacked === "true";
      const percentage = stacked && ["cores", "bytes"].includes(chart.dataset.unit);
      const formatSeries = unitFormat(chart.dataset.unit, rootData.coreUnit);
      const rowsAt = new Map(rows.map((row) => [row.at * 1000, row]));
      if (percentage) {
        axes.y.valueFormatter = percent;
        axes.y.axisLabelFormatter = percent;
      }
      const extraRows = stacked ? 1 : 0;
      chart.style.setProperty(
        "--metric-legend-rows",
        String(1 + seriesLabels.length + extraRows),
      );
      const formatSeriesDetail = detailFormat(chart.dataset.detailUnit, rootData.coreUnit);
      const extraText = (extra) => {
        if (!extra || !formatSeriesDetail) return "";
        const details = [
          [extra[0], chart.dataset.detailLabel],
          [extra[1], chart.dataset.totalLabel],
        ]
          .filter(([value]) => value != null)
          .map(([value, label]) =>
            label ? `${label} ${formatSeriesDetail(value)}` : formatSeriesDetail(value))
          .join(" / ");
        return details ? ` (${details})` : "";
      };
      const graphOptions = {
        ...sharedOptions,
        labels: ["time", ...seriesLabels],
        connectSeparatedPoints: chart.dataset.filesystems === "true",
        highlightSeriesOpts: { strokeWidth: 2 },
        highlightSeriesBackgroundAlpha: 1,
        ...(stacked
          ? {
              stackedGraph: true,
              stackedGraphNaNFill: "none",
              fillGraph: true,
              fillAlpha: 0.08,
              strokeWidth: 0.75,
            }
          : {}),
        legendFormatter: (data) => {
          const lines = [data.xHTML];
          const row = rowsAt.get(data.x);
          const capacity = row?.total;
          const share = (value) =>
            stacked && capacity > 0 ? ` (${percent(value / capacity)})` : "";
          let total = null;
          for (const [index, item] of data.series.entries()) {
            if (item.y == null || Number.isNaN(item.y)) continue;
            const value = percentage ? row.v[index] : item.y;
            total = (total ?? 0) + value;
            const text = `${item.labelHTML} ${percentage ? formatSeries(value) : item.yHTML}${share(value)}${extraText(row?.e?.[index])}`;
            lines.push(`${item.dashHTML} ${item.isHighlighted ? `<b>${text}</b>` : text}`);
          }
          if (stacked && total !== null) {
            lines.push(summary(formatSeries, total, capacity, rootData));
          }
          return lines.filter(Boolean).join("<br>");
        },
        axes,
      };
      const graph = new Dygraph(
        plot,
        rows.map((row) => [
          new Date(row.at * 1000),
          ...(row.v ?? seriesLabels.map(() => null)).map((value) => {
            if (value === false) return Number.NaN;
            if (value == null || !percentage) return value;
            return row.total > 0 ? value / row.total : null;
          }),
        ]),
        graphOptions,
      );
      return { graph, themed };
    }
    const labels = chart.dataset.statLabels ? chart.dataset.statLabels.split(",") : statLabels;
    const formatDetail = detailFormat(chart.dataset.detailUnit, rootData.coreUnit);
    const detailLabel = chart.dataset.detailLabel;
    const totalLabel = chart.dataset.totalLabel;
    const replicasLabel = rootData.replicasLabel;
    const replicasAt = replicasLabel && rows.some((row) => row.replicas != null)
      ? new Map(rows.map((row) => [row.at * 1000, row.replicas]))
      : null;
    chart.style.setProperty("--metric-legend-rows", replicasAt ? "5" : "4");
    const detailText = (value, label) => {
      if (value == null || !formatDetail) return null;
      const formatted = formatDetail(value);
      return label ? `${label} ${formatted}` : formatted;
    };
    const graphOptions = {
      labels: ["time", "value"],
      ...sharedOptions,
      customBars: true,
      fillAlpha: minMax ? statFillAlpha : chartFillAlpha,
      connectSeparatedPoints: false,
      legendFormatter: ({ x, xHTML, series }) => {
        const replicas = replicasAt?.get(x);
        return [
          xHTML,
          series[0]?.yHTML,
          replicas != null ? `${replicasLabel} ${replicas}` : null,
        ].filter(Boolean).join("<br>");
      },
      axes: {
        ...axes,
        y: {
          ...axes.y,
          valueFormatter: (value, _options, _name, graph, row, column) => {
            const bar = graph.getValue(row, column);
            if (!Array.isArray(bar) || bar.includes(null)) return format(value);
            const detail = rows[row]?.detail;
            const total = rows[row]?.total;
            const valueAt = (edge, index) => {
              const formatted = format(edge);
              if (index !== 1) return formatted;
              const details = [
                detailText(detail, detailLabel),
                detailText(total, totalLabel),
              ]
                .filter(Boolean)
                .join(" / ");
              return details ? `${formatted} (${details})` : formatted;
            };
            if (bar[0] === bar[2]) return valueAt(bar[1], 1);
            return bar
              .map((edge, index) => `${labels[index]} ${valueAt(edge, index)}`)
              .join("<br>");
          },
        },
      },
    };
    const graph = new Dygraph(
      plot,
      rows.map((row) => [new Date(row.at * 1000), row.v]),
      graphOptions,
    );
    return { graph, themed };
  };

  const xInteraction = (initialWindow, plot) => {
    const selection = document.createElement("div");
    selection.className = "metric-chart-selection";
    selection.hidden = true;
    const clear = (context) => {
      context.active = null;
      selection.hidden = true;
    };
    const move = (event, graph, context) => {
      const active = context.active;
      if (!active || event.pointerId !== active.pointerId) return null;
      const [x, y] = graph.eventToDomCoords(event);
      const deltaX = x - active.startX;
      const deltaY = y - active.startY;
      if (active.horizontal == null && Math.max(Math.abs(deltaX), Math.abs(deltaY)) >= 10) {
        active.horizontal = Math.abs(deltaX) > Math.abs(deltaY);
      }
      if (!active.horizontal) return [x, y, null];
      const area = graph.getArea();
      const endX = Math.min(area.x + area.w, Math.max(area.x, x));
      const left = Math.min(active.startX, endX);
      if (!selection.isConnected) plot.append(selection);
      selection.style.left = `${left}px`;
      selection.style.top = `${area.y}px`;
      selection.style.width = `${Math.abs(endX - active.startX)}px`;
      selection.style.height = `${area.h}px`;
      selection.hidden = false;
      return [x, y, endX];
    };
    const cancel = (event, _graph, context) => {
      if (event.pointerId !== context.active?.pointerId) return;
      context.tap = null;
      clear(context);
    };
    return {
      pointerdown(event, graph, context) {
        if (context.active || !event.isPrimary || event.button !== 0) return;
        const [x, y] = graph.eventToDomCoords(event);
        const area = graph.getArea();
        if (x < area.x || x > area.x + area.w || y < area.y || y > area.y + area.h) return;
        context.active = {
          pointerId: event.pointerId,
          startX: x,
          startY: y,
          horizontal: null,
        };
        event.currentTarget.setPointerCapture(event.pointerId);
      },
      pointermove: move,
      pointerup(event, graph, context) {
        const active = context.active;
        if (!active || event.pointerId !== active.pointerId) return;
        const [x, y, endX] = move(event, graph, context)
          ?? [active.startX, active.startY, null];
        if (active.horizontal && endX != null) {
          const left = Math.min(active.startX, endX);
          const right = Math.max(active.startX, endX);
          if (right - left >= 10) {
            graph.clearSelection();
            graph.updateOptions({ dateWindow: [graph.toDataXCoord(left), graph.toDataXCoord(right)] });
          }
          context.tap = null;
        } else if (active.horizontal == null
          && Math.hypot(x - active.startX, y - active.startY) < 10) {
          const previous = context.tap;
          if (previous
            && event.timeStamp - previous.time <= 500
            && Math.hypot(x - previous.x, y - previous.y) <= 50) {
            graph.clearSelection();
            graph.updateOptions({ dateWindow: [...initialWindow] });
            context.tap = null;
          } else {
            context.tap = { time: event.timeStamp, x, y };
          }
        } else {
          context.tap = null;
        }
        clear(context);
      },
      pointercancel: cancel,
      lostpointercapture: cancel,
      willDestroyContextMyself: true,
    };
  };

  const zoom = (entries) => {
    let syncing = false;
    return (source, initial) => {
      if (syncing || initial) return;
      syncing = true;
      try {
        const [start, end] = source.xAxisRange();
        for (const { graph } of entries) {
          if (graph === source) continue;
          const [from, to] = graph.xAxisRange();
          if (from !== start || to !== end) graph.updateOptions({ dateWindow: [start, end] });
        }
      } finally {
        syncing = false;
      }
    };
  };

  const metrics = (root) => {
    const entries = [];
    const groupEntries = new WeakMap();
    const preferenceKey = (group) => {
      const scope = root.dataset.metricScope;
      const name = group.dataset.metricGroup;
      return scope && name ? `cdelivery:metrics:v1:${scope}:${name}` : null;
    };
    const restoreOpen = (group) => {
      const key = preferenceKey(group);
      if (!key) return;
      try {
        const value = localStorage.getItem(key);
        if (value === "open") group.open = true;
        else if (value === "closed") group.open = false;
      } catch {
        return;
      }
    };
    const persistOpen = (group) => {
      const key = preferenceKey(group);
      if (!key) return;
      try {
        localStorage.setItem(key, group.open ? "open" : "closed");
      } catch {
        return;
      }
    };
    const statLabels = [root.dataset.minLabel, root.dataset.avgLabel, root.dataset.maxLabel];
    const initialWindow = [Date.parse(root.dataset.start), Date.parse(root.dataset.end)];
    const drawCallback = zoom(entries);
    const addGraph = (chart) => {
      const entry = createGraph(chart, {
        drawCallback,
        dateWindow: entries[0]?.graph.xAxisRange() ?? initialWindow,
      }, statLabels, root.dataset, initialWindow);
      entries.push(entry);
    };
    const initializeGroup = (group) => {
      const existing = groupEntries.get(group);
      if (existing) {
        existing.forEach(({ graph }) => graph.resize());
        return;
      }
      const start = entries.length;
      group.querySelectorAll(".metric-chart").forEach(addGraph);
      groupEntries.set(group, entries.slice(start));
    };
    for (const group of root.children) {
      restoreOpen(group);
      let open = group.open;
      group.addEventListener("toggle", () => {
        if (group.open === open) return;
        open = group.open;
        persistOpen(group);
        if (open) initializeGroup(group);
      });
      if (open) initializeGroup(group);
    }
    if (document.documentElement.dataset.theme === "auto") {
      matchMedia("(prefers-color-scheme: dark)").addEventListener("change", () => {
        entries.forEach(({ graph, themed }) => graph.updateOptions(themed()));
      });
    }
  };

  const navigation = () => {
    const toggle = document.getElementById("nav-toggle");
    const nav = document.getElementById("page-navigation");
    if (!toggle || !nav) return;
    const mobile = matchMedia("(width <= 48rem)");
    const firstLink = nav.querySelector("a[href]");
    const focus = (element) => element?.focus({ preventScroll: true });
    const isOpen = () => mobile.matches && toggle.getAttribute("aria-expanded") === "true";
    const contains = (target) => nav.contains(target) || toggle.contains(target);
    const close = (restoreFocus = false) => {
      const restore = restoreFocus || nav.contains(document.activeElement);
      toggle.setAttribute("aria-expanded", "false");
      if (mobile.matches && restore) focus(toggle);
    };
    const reset = () => {
      close();
      if (!mobile.matches && document.activeElement === toggle) focus(firstLink);
    };
    toggle.addEventListener("click", () => {
      if (!mobile.matches) return;
      if (isOpen()) close();
      else {
        toggle.setAttribute("aria-expanded", "true");
        nav.scrollTop = 0;
        focus(firstLink);
      }
    });
    document.addEventListener("click", (event) => {
      if (!isOpen()) return;
      if (!contains(event.target) || event.target.closest("a[href]")) close();
    });
    document.addEventListener("focusin", (event) => {
      if (isOpen() && !contains(event.target)) close();
    });
    document.addEventListener("keydown", (event) => {
      if (event.key !== "Escape" || !isOpen()) return;
      event.preventDefault();
      close(true);
    });
    mobile.addEventListener("change", reset);
    document.documentElement.setAttribute("data-navigation-ready", "");
    reset();
  };

  document.addEventListener("DOMContentLoaded", () => {
    navigation();
    const root = document.getElementById("metric-charts");
    if (!root) return;
    metrics(root);
  });

  const guard = (component, event) => {
    if (component.submitting) event.preventDefault();
    return component.submitting;
  };

  const expose = (data, source, keys) => {
    for (const key of keys) {
      Object.defineProperty(data, key, {
        get() { return this[source][key]; },
        set(value) { this[source][key] = value; },
        enumerable: true,
      });
    }
  };

  const placementOptions = (nodes, type, value, unavailable) => {
    const nodeLabel = node => `${node.hostname} (${node.addr || "—"})`;
    const schedulable = nodes.filter(node => node.availability === "Active" && node.state === "Ready");
    const labelOptions = [...schedulable.reduce((counts, node) => {
      for (const label of node.labels) counts.set(label, (counts.get(label) || 0) + 1);
      return counts;
    }, new Map()).entries()]
      .sort(([left], [right]) => left.localeCompare(right))
      .map(([name, count]) => ({ name, label: `${name} (${count})` }));
    const nodeOptions = schedulable.map(node => ({ id: node.id, label: nodeLabel(node) }));
    if (value && type === "NodeId" && !schedulable.some(node => node.id === value)) {
      const node = nodes.find(node => node.id === value);
      nodeOptions.unshift({ id: value, label: node ? `${nodeLabel(node)} — ${unavailable}` : `${value} (${unavailable})` });
    }
    if (value && type === "Label" && !labelOptions.some(label => label.name === value)) {
      labelOptions.unshift({ name: value, label: `${value} (${unavailable})` });
    }
    return { nodeOptions, labelOptions };
  };

  window.CDelivery = Object.freeze({ timeRange, formatInstant, initTimezone, zoneOptions, saveTimezone, guard, expose, placementOptions, quantity });
})();
