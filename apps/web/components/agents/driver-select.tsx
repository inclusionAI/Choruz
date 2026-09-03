import { driverDisplayName, type DriverId } from "../../lib/drivers/driver-registry";

/** Driver dropdown; labels come from the driver registry, never inline. */
export function DriverSelect({
  value,
  onChange,
  drivers,
  disabled,
  "aria-label": ariaLabel,
}: {
  value: DriverId;
  onChange: (driver: DriverId) => void;
  drivers: ReadonlyArray<DriverId>;
  disabled?: boolean;
  "aria-label"?: string;
}) {
  return (
    <select
      aria-label={ariaLabel}
      value={value}
      disabled={disabled}
      onChange={(e) => onChange(e.target.value as DriverId)}
    >
      {drivers.map((driver) => (
        <option key={driver} value={driver}>
          {driverDisplayName(driver)}
        </option>
      ))}
    </select>
  );
}
