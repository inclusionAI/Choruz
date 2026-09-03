import { NextRequest, NextResponse } from "next/server";

import { requireAuth } from "../../../../lib/api/api-auth";
import { discoverDriverModels } from "../../../../lib/drivers/driver-models";
import { DRIVER_IDS, type DriverId } from "../../../../lib/groups/team-templates";

export async function GET(request: NextRequest) {
  const auth = await requireAuth(request);
  if (auth instanceof NextResponse) return auth;

  const driverId = request.nextUrl.searchParams.get("driver_type");
  if (!driverId || !DRIVER_IDS.includes(driverId as DriverId)) {
    return NextResponse.json(
      { error: "Query parameter `driver_type` must name a supported driver." },
      { status: 400 },
    );
  }

  return NextResponse.json(await discoverDriverModels(driverId as DriverId));
}
