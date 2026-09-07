import { redactSensitiveText } from "./telemetry-sanitize";

const PRIVATE_REGION = '[data-activity-private],.terminal-view,.xterm';
const DYNAMIC_REGION = '.attachment-queue,.msg-group,.conv-item,.folder-picker-modal,.file-tree';
const SENSITIVE_FIELD = /password|secret|token|credential|authentication|authorization|callback|invitation|api.?key|one-time-code/i;
const VALUE_LIMIT = 2048;

/** Bounded snapshots, not key events. Secret fields never expose even their length. */
export function activityValue(value: string, identity: string, privateField = false) {
  if (privateField || SENSITIVE_FIELD.test(identity)) return { value_omitted: "private" };
  if (/\bpath\b|directory|folder/i.test(identity)) return { value_omitted: "filesystem" };
  const sanitized = redactSensitiveText(value);
  return { value: sanitized.slice(0, VALUE_LIMIT), value_length: sanitized.length, value_truncated: sanitized.length > VALUE_LIMIT };
}

export function activityExcluded(el: Element): boolean {
  return Boolean(el.closest(PRIVATE_REGION));
}

function fieldLabel(el: Element): string {
  // Label text nodes exclude the input value, select options and help text.
  const labels = (el as HTMLInputElement).labels;
  const label = labels?.[0] ?? el.closest("label");
  return Array.from(label?.childNodes ?? [])
    .filter(node => node.nodeType === Node.TEXT_NODE).map(node => node.textContent).join(" ").trim();
}

/** Semantic identity has no CSS classes, screen coordinates or generated React IDs. */
export function describeActivityControl(el: Element) {
  const dynamic = Boolean(el.closest(DYNAMIC_REGION));
  const label = dynamic ? "" : (el.getAttribute("aria-label") || el.getAttribute("title") || fieldLabel(el) ||
    el.getAttribute("placeholder") || (el.matches('button,a,[role="tab"],[role="menuitem"]') ? el.textContent : "") || "").trim().slice(0, 160);
  const id = el.id && !/[:«»]/.test(el.id) ? el.id : "";
  const control = el.getAttribute("data-activity") || el.getAttribute("data-testid") || el.getAttribute("name") || id || label || el.tagName.toLowerCase();
  const dialog = el.closest('[role="dialog"]');
  return {
    tag: el.tagName.toLowerCase(), control: control.slice(0, 160), label,
    role: el.getAttribute("role") ?? undefined,
    surface: dialog?.getAttribute("data-activity-surface") ?? "dashboard",
    resource: window.location.pathname,
  };
}

export function activityFieldValue(el: Element, value: string) {
  const identity = [el.getAttribute("type"), el.getAttribute("autocomplete"), el.getAttribute("name"), el.id,
    el.getAttribute("aria-label"), el.getAttribute("data-activity"), fieldLabel(el)].join(" ");
  return activityValue(value, identity, Boolean(el.closest('[data-activity-value="private"]')));
}
