type Timestamp = string | number | Date;

const MS_PER_DAY = 24 * 60 * 60 * 1000;
const TIME_DIVIDER_GAP_MS = 5 * 60 * 1000;
const ENGLISH_LOCALE = "en-US";

function toDate(value: Timestamp): Date {
  return value instanceof Date ? value : new Date(value);
}

function pad2(value: number): string {
  return String(value).padStart(2, "0");
}

function formatHourMinute(date: Date): string {
  return `${pad2(date.getHours())}:${pad2(date.getMinutes())}`;
}

function startOfLocalDay(date: Date): number {
  return new Date(date.getFullYear(), date.getMonth(), date.getDate()).getTime();
}

export function formatChatDivider(timestamp: Timestamp, now: Timestamp = new Date()): string {
  const date = toDate(timestamp);
  const reference = toDate(now);
  const dayDiff = Math.floor((startOfLocalDay(reference) - startOfLocalDay(date)) / MS_PER_DAY);
  const time = formatHourMinute(date);

  if (dayDiff === 0) {
    return time;
  }

  if (dayDiff === 1) {
    return `Yesterday ${time}`;
  }

  if (dayDiff > 1 && dayDiff < 7) {
    const weekday = new Intl.DateTimeFormat(ENGLISH_LOCALE, { weekday: "short" }).format(date);
    return `${weekday} ${time}`;
  }

  if (date.getFullYear() === reference.getFullYear()) {
    const day = new Intl.DateTimeFormat(ENGLISH_LOCALE, { month: "short", day: "numeric" }).format(date);
    return `${day} ${time}`;
  }

  const day = new Intl.DateTimeFormat(ENGLISH_LOCALE, {
    year: "numeric",
    month: "short",
    day: "numeric",
  }).format(date);
  return `${day} ${time}`;
}

export function shouldShowTimeDivider(
  previousTimestamp: Timestamp | null | undefined,
  currentTimestamp: Timestamp,
): boolean {
  if (!previousTimestamp) {
    return true;
  }

  const previous = toDate(previousTimestamp);
  const current = toDate(currentTimestamp);

  if (startOfLocalDay(previous) !== startOfLocalDay(current)) {
    return true;
  }

  return current.getTime() - previous.getTime() >= TIME_DIVIDER_GAP_MS;
}

export function formatAbsoluteTime(timestamp: Timestamp): string {
  return new Intl.DateTimeFormat(ENGLISH_LOCALE, {
    year: "numeric",
    month: "numeric",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  }).format(toDate(timestamp));
}
