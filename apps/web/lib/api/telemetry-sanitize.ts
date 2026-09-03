const REDACTED = "[REDACTED]";

function compactKey(key: string): string {
  return key.toLowerCase().replace(/[_-]/g, "");
}

function telemetryKeyIsSensitive(key: string): boolean {
  const compact = compactKey(key);
  return (
    [
      "authorization",
      "cookie",
      "setcookie",
      "database64",
      "attachmentbytes",
      "filebytes",
      "contentbytes",
      "bodybytes",
      "rawbytes",
      "bytesbase64",
      "payloadbase64",
      "filename",
      "attachmentname",
      "path",
      "paths",
    ].includes(compact) ||
    compact.endsWith("filename") ||
    compact.includes("secret") ||
    compact.includes("password") ||
    compact.endsWith("path") ||
    compact.endsWith("paths") ||
    compact.includes("sessiontoken") ||
    compact.endsWith("token")
  );
}

function telemetryKeyIsPrivateContent(key: string): boolean {
  return ["content", "message", "text", "body", "preview"].includes(key.toLowerCase());
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

export function redactSensitiveText(input: string): string {
  let redacted = input;
  for (const marker of [
    "Bearer ",
    "bearer ",
    "secret=",
    "secret:",
    "token=",
    "token:",
    "password=",
    "password:",
  ]) {
    redacted = redactAfterMarker(redacted, marker);
  }
  return redacted;
}

function redactAfterMarker(input: string, marker: string): string {
  let output = "";
  let cursor = 0;

  while (true) {
    const start = input.indexOf(marker, cursor);
    if (start === -1) break;

    const valueStart = start + marker.length;
    output += input.slice(cursor, valueStart);
    const rest = input.slice(valueStart);
    const skipWs = rest.length - rest.trimStart().length;
    const redactionStart = valueStart + skipWs;
    const valueEndOffset = input.slice(redactionStart).search(/[\s"',)}\]]/);
    const valueEnd = valueEndOffset === -1 ? input.length : redactionStart + valueEndOffset;

    output += "[REDACTED]";
    cursor = valueEnd;
  }

  return output + input.slice(cursor);
}

export function sanitizeTelemetryValue(value: unknown): unknown {
  if (Array.isArray(value)) {
    return value.map(sanitizeTelemetryValue);
  }
  if (typeof value === "string") {
    return redactSensitiveText(value);
  }
  if (!isRecord(value)) {
    return value;
  }

  const privatePayload =
    value.private === true ||
    value.is_private === true ||
    value.privacy === "private";
  const sanitized: Record<string, unknown> = {};

  for (const [key, nested] of Object.entries(value)) {
    if (telemetryKeyIsSensitive(key) || (privatePayload && telemetryKeyIsPrivateContent(key))) {
      sanitized[key] = REDACTED;
    } else {
      sanitized[key] = sanitizeTelemetryValue(nested);
    }
  }

  return sanitized;
}

export function sanitizeTelemetryData(data?: Record<string, unknown>): Record<string, unknown> | undefined {
  if (!data) return undefined;
  const sanitized = sanitizeTelemetryValue(data);
  return isRecord(sanitized) ? sanitized : undefined;
}
