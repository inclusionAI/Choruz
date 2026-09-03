export function validateModelId(model: unknown): string | null {
  if (model === undefined || model === null || model === "") return null;
  if (typeof model !== "string") return "Field `model` must be a string.";
  const trimmed = model.trim();
  if (!trimmed) return null;
  if (trimmed.length > 256) return "Field `model` must be 256 characters or fewer.";
  if (/\p{Cc}/u.test(trimmed)) return "Field `model` cannot contain control characters.";
  if (trimmed.startsWith("-")) return "Field `model` cannot start with `-`.";
  return null;
}
