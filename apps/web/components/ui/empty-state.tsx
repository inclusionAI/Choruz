import type { ReactNode } from "react";

/**
 * The one empty-state layout: optional haloed icon, a short title, a line
 * of guidance. `inline` is the compact variant for lists and panel
 * sections; the default fills its container and centres.
 */
export function EmptyState({
  icon,
  title,
  description,
  inline = false,
  className,
}: {
  icon?: ReactNode;
  title?: ReactNode;
  description?: ReactNode;
  inline?: boolean;
  className?: string;
}) {
  const classes = ["empty-state", inline && "empty-state--inline", className]
    .filter(Boolean)
    .join(" ");
  return (
    <div className={classes}>
      {icon && (
        <div className="empty-icon" aria-hidden="true">
          {icon}
        </div>
      )}
      {title && <h3>{title}</h3>}
      {description && <p>{description}</p>}
    </div>
  );
}
