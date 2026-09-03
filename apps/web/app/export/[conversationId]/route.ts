import { NextRequest, NextResponse } from "next/server";

import { decodeSessionClaims, exportConversation, sessionCookieName } from "../../../lib/api/choruz-api";
import { requestUrl } from "../../../lib/api/request-origin";

type ExportRouteProps = {
  params: Promise<{
    conversationId: string;
  }>;
};

export async function GET(request: NextRequest, { params }: ExportRouteProps) {
  const { conversationId } = await params;
  const sessionToken = request.cookies.get(sessionCookieName())?.value;
  const claims = sessionToken ? decodeSessionClaims(sessionToken) : null;
  if (!sessionToken || !claims) {
    return NextResponse.redirect(requestUrl(request, "/"), 303);
  }

  try {
    const payload = await exportConversation(sessionToken, claims.principal_id, conversationId);
    return new NextResponse(JSON.stringify(payload, null, 2), {
      headers: {
        "content-type": "application/json; charset=utf-8",
        "content-disposition": `attachment; filename="conversation-${conversationId}.json"`,
      },
    });
  } catch (error) {
    const detail = error instanceof Error ? error.message : "Export failed";
    return NextResponse.redirect(requestUrl(request, `/dashboard?error=${encodeURIComponent(detail)}`), 303);
  }
}
