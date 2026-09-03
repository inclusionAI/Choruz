import { Loader2 } from "lucide-react";

/**
 * Inline loading indicator. With a `label` it is a status line
 * ("Loading…") announced to assistive tech. Without one it is a bare
 * icon for buttons: mark the button `aria-busy` so its own name stays
 * intact instead of being replaced by a live region.
 */
export function Spinner({ size = 14, label }: { size?: number; label?: string }) {
  const icon = <Loader2 size={size} className="icon-spin" aria-hidden="true" />;
  if (!label) return icon;
  return (
    <span className="spinner" role="status">
      {icon}
      <span>{label}</span>
    </span>
  );
}
