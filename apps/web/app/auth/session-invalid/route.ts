import { NextRequest, NextResponse } from "next/server";

import { sessionCookieName } from "../../../lib/api/choruz-api";
import { requestUrl } from "../../../lib/api/request-origin";

export async function GET(request: NextRequest) {
  const response = NextResponse.redirect(requestUrl(request, "/"), 303);
  response.cookies.delete(sessionCookieName());
  return response;
}
