import { NextRequest, NextResponse } from "next/server";

import { requireAuth } from "../../../../lib/api/api-auth";
import { getDriverAvailability } from "../../../../lib/drivers/driver-availability";
import type { DriverAvailabilityItem } from "../../../../lib/drivers/driver-availability";
import { deviceDriverCatalog } from "../../../../lib/drivers/device-driver-catalog";

export async function GET(request: NextRequest) {
  const auth = await requireAuth(request);
  if (auth instanceof NextResponse) return auth;

  const hostId = request.nextUrl.searchParams.get("runtime_host_id");
  if (hostId) {
    const response = await deviceDriverCatalog(auth.token, hostId);
    return NextResponse.json(await response.json(), { status: response.status });
  }

  const drivers = await getDriverAvailability();
  return NextResponse.json({ drivers: drivers.map(publicDriverAvailabilityItem) });
}

function publicDriverAvailabilityItem(item: DriverAvailabilityItem) {
  return {
    label: item.label,
    driverId: item.driverId,
    status: item.status,
    reason: item.reason,
    setupHint: item.setupHint,
    ...(item.envVar ? { envVar: item.envVar } : {}),
  };
}
