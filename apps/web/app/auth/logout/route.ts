import { NextRequest, NextResponse } from "next/server";

import { sessionCookieName } from "../../../lib/api/choruz-api";
import { requestUrl } from "../../../lib/api/request-origin";

export async function POST(request: NextRequest) {
  const redirect = NextResponse.redirect(requestUrl(request, "/"), 303);
  redirect.cookies.delete(sessionCookieName());
  return redirect;
}
