import { NextRequest } from "next/server";

function normalizeProtocol(request: NextRequest): string {
  const forwarded = request.headers.get("x-forwarded-proto");
  if (forwarded) {
    return forwarded.replace(/:$/, "");
  }

  return request.nextUrl.protocol.replace(/:$/, "");
}

export function requestOrigin(request: NextRequest): string {
  const host =
    request.headers.get("x-forwarded-host") ??
    request.headers.get("host") ??
    request.nextUrl.host;

  return `${normalizeProtocol(request)}://${host}`;
}

export function requestUrl(request: NextRequest, pathname: string): URL {
  return new URL(pathname, requestOrigin(request));
}
