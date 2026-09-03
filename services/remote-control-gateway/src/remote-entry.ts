// The Cloud Gateway relays ciphertext; it does not serve the dashboard. A
// browser that lands on `/` or `/remote` is sent to a hosted Choruz dashboard
// when one is configured, and otherwise told where the Remote page lives.

export function remoteEntryResponse(
  requestUrl: URL,
  dashboardUrl: string | undefined,
): Response {
  const gateway = requestUrl.origin;
  if (dashboardUrl && dashboardUrl.trim()) {
    const target = new URL("/remote", dashboardUrl.trim());
    const params = new URLSearchParams(requestUrl.search);
    params.set("gateway", gateway);
    target.search = params.toString();
    return Response.redirect(target.toString(), 302);
  }
  const body = [
    "This Choruz Cloud Gateway only relays end-to-end encrypted Remote Control traffic.",
    "Open the Remote page of any Choruz dashboard to pair with a host:",
    `  https://<your-choruz>/remote?gateway=${gateway}`,
    "",
  ].join("\n");
  return new Response(body, {
    status: 404,
    headers: {
      "content-type": "text/plain; charset=utf-8",
      "cache-control": "no-store",
      "x-content-type-options": "nosniff",
    },
  });
}
