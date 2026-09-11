import { NextRequest, NextResponse } from "next/server";

import { requireAuth } from "../../../../lib/api/api-auth";
import { discoverDriverModels, parsePiModels, parseGrokModels, parseOpenCodeModels } from "../../../../lib/drivers/driver-models";
import { deviceDriverCatalog } from "../../../../lib/drivers/device-driver-catalog";
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

  const hostId = request.nextUrl.searchParams.get("runtime_host_id");
  if (hostId || driverId === "claude_terminal") {
    const response = await deviceDriverCatalog(auth.token, hostId, driverId);
    const body = await response.json();
    if (!response.ok) return NextResponse.json(body, { status: response.status });
    let models = body.models;
    if (!models) {
      switch (driverId) {
        case "pi_terminal":
          models = parsePiModels(body.stdout ?? "");
          break;
        case "grok_terminal":
          models = parseGrokModels(`${body.stdout ?? ""}\n${body.stderr ?? ""}`);
          break;
        default:
          models = parseOpenCodeModels(body.stdout ?? "");
      }
    }
    return NextResponse.json({
      driverId,
      status: models.length ? "available" : "unavailable",
      models,
      message: models.length
        ? `${models.length} models discovered on the selected device.`
        : "The selected device returned no selectable models.",
    });
  }
  return NextResponse.json(await discoverDriverModels(driverId as DriverId));
}
