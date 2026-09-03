import { NextRequest, NextResponse } from "next/server";
import { requireAuth } from "../../../../lib/api/api-auth";

const GW =
  process.env.CHORUZ_API_BASE_URL?.trim()
  || process.env.CHORUZ_API_URL?.trim()
  || `http://127.0.0.1:${process.env.CHORUZ_API_PORT ?? "3000"}`;

export async function GET(
  request: NextRequest,
  { params }: { params: Promise<{ id: string }> },
) {
  const auth = await requireAuth(request);
  if (auth instanceof NextResponse) return auth;

  const { id } = await params;
  const resp = await fetch(
    `${GW}/v1/attachments/${id}?actor_id=${auth.claims.principal_id}`,
    { headers: { Cookie: `choruz_session=${auth.token}` } },
  );

  if (!resp.ok) {
    return new NextResponse(null, { status: resp.status });
  }

  const body = await resp.arrayBuffer();
  const contentType = resp.headers.get("content-type") || "application/octet-stream";

  return new NextResponse(body, {
    headers: {
      "Content-Type": contentType,
      // Attachment is session-scoped — never cache it on shared proxies /
      // CDNs, where the first authorised response could leak to a
      // different user. `Vary: Cookie` is defense in depth if a browser
      // or intermediary ignores `private, no-store`.
      "Cache-Control": "private, no-store",
      "Vary": "Cookie",
    },
  });
}
