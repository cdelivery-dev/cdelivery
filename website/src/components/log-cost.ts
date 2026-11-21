export const defaults = {
  dailyBytes: 1e9,
  logBytes: 1000,
  ratio: 6.5,
  retentionMonths: 12,
};

export const pricing = {
  checked: '2026-09-28',
  daysPerMonth: 365 / 12,
  storageTiers: [
    { capacity: 51_200, rate: 0.023 },
    { capacity: 460_800, rate: 0.022 },
    { capacity: Infinity, rate: 0.021 },
  ],
  objectBytes: 16e6,
  putPerThousand: 0.005,
  ingestionPerGb: 0.1,
  indexingPerMillion: 2.5,
  flexPerMillionMonth: 0.6,
  flexRetentionMonths: [3, 6, 12, 15],
  flexMonthlyEvents: 2e9,
  flexStoredEvents: 10e9,
};

function monthlyStorage(gibibytes: number) {
  let remaining = gibibytes;
  let cost = 0;
  for (const tier of pricing.storageTiers) {
    const quantity = Math.min(remaining, tier.capacity);
    cost += quantity * tier.rate;
    remaining -= quantity;
  }
  return cost;
}

export function calculate(input: typeof defaults) {
  const monthlyBytes = input.dailyBytes * pricing.daysPerMonth;
  const compressedMonthlyGiB = monthlyBytes / input.ratio / 2 ** 30;
  let storage = 0;
  for (let month = 0; month < input.retentionMonths; month++) {
    storage += monthlyStorage(compressedMonthlyGiB * (month + 0.5));
  }
  const totalBytes = monthlyBytes * input.retentionMonths;
  const put = Math.ceil(totalBytes / input.ratio / pricing.objectBytes) * pricing.putPerThousand / 1000;
  const cdelivery = storage + put;
  const ingestion = totalBytes / 1e9 * pricing.ingestionPerGb;
  const monthlyEvents = monthlyBytes / input.logBytes;
  const plan: 'standard' | 'flex' | 'extrapolated' = input.retentionMonths === 1
    ? 'standard'
    : !pricing.flexRetentionMonths.includes(input.retentionMonths)
      || monthlyEvents >= pricing.flexMonthlyEvents
      || monthlyEvents * input.retentionMonths > pricing.flexStoredEvents
      ? 'extrapolated'
      : 'flex';
  const processing = plan === 'standard'
    ? monthlyEvents / 1e6 * pricing.indexingPerMillion
    : monthlyEvents * input.retentionMonths ** 2 / 2 / 1e6 * pricing.flexPerMillionMonth;
  const datadog = ingestion + processing;
  return { cdelivery, datadog, relative: cdelivery / datadog, plan };
}

export function formatMoney(value: number, locale: string) {
  const formatter = new Intl.NumberFormat(locale, { style: 'currency', currency: 'USD', currencyDisplay: 'narrowSymbol' });
  return value > 0 && value < 0.01 ? `< ${formatter.format(0.01)}` : formatter.format(value);
}

export function formatPercent(value: number, locale: string) {
  const formatter = new Intl.NumberFormat(locale, { style: 'percent', maximumFractionDigits: 2 });
  return value > 0 && value < 0.0001 ? `< ${formatter.format(0.0001)}` : formatter.format(value);
}
