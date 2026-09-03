/**
 * Choruz HTTP API client.
 *
 * Wraps login, sendMessage, listConversations, createGroup, provisionAgent
 * and setEventWebhook. Automatically refreshes the session token on 401.
 */

export interface LoginResult {
  session_token: string;
  principal_id: string;
}

export class ChoruzClient {
  private apiUrl: string;
  private token: string | null = null;
  private principalId: string | null = null;
  private username: string;
  private password: string;

  constructor(config: { api_url: string; username: string; password: string }) {
    this.apiUrl = config.api_url.replace(/\/+$/, '');
    this.username = config.username;
    this.password = config.password;
  }

  // ── public getters ──────────────────────────────────────────────────

  getPrincipalId(): string {
    if (!this.principalId) throw new Error('Not logged in yet');
    return this.principalId;
  }

  // ── auth ────────────────────────────────────────────────────────────

  /** Login and cache session_token + principal_id. */
  async login(): Promise<LoginResult> {
    const res = await fetch(`${this.apiUrl}/v1/auth/local/login`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ username: this.username, password: this.password }),
    });

    if (!res.ok) {
      const body = await res.text();
      throw new Error(`Choruz login failed (${res.status}): ${body}`);
    }

    const data = (await res.json()) as {
      session_token: string;
      principal: { id: string };
    };

    this.token = data.session_token;
    this.principalId = data.principal.id;

    return { session_token: this.token, principal_id: this.principalId };
  }

  /** Ensure we have a valid token; login if not. */
  private async ensureAuth(): Promise<string> {
    if (!this.token) {
      await this.login();
    }
    return this.token!;
  }

  // ── request helper ──────────────────────────────────────────────────

  private async request<T>(
    method: string,
    path: string,
    body?: unknown,
    retry = true,
  ): Promise<T> {
    const token = await this.ensureAuth();
    const url = `${this.apiUrl}${path}`;

    const res = await fetch(url, {
      method,
      headers: {
        'Authorization': `Bearer ${token}`,
        'Content-Type': 'application/json',
      },
      ...(body !== undefined ? { body: JSON.stringify(body) } : {}),
    });

    // Auto-refresh on 401 and retry once
    if (res.status === 401 && retry) {
      await this.login();
      return this.request<T>(method, path, body, false);
    }

    if (!res.ok) {
      const text = await res.text();
      throw new Error(`Choruz API ${method} ${path} failed (${res.status}): ${text}`);
    }

    const contentType = res.headers.get('content-type') ?? '';
    if (contentType.includes('application/json')) {
      return (await res.json()) as T;
    }
    return undefined as T;
  }

  // ── messages ────────────────────────────────────────────────────────

  /** Send a message to a conversation. */
  async sendMessage(params: {
    actor_id: string;
    conversation_id: string;
    content: string;
    content_type?: string;
    metadata?: Record<string, unknown>;
  }): Promise<{ id: string }> {
    return this.request<{ id: string }>('POST', '/v1/messages', {
      actor_id: params.actor_id,
      conversation_id: params.conversation_id,
      content: params.content,
      content_type: params.content_type ?? 'text/plain',
      idempotency_key: `bridge-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`,
      metadata: params.metadata ?? {},
    });
  }

  // ── conversations ───────────────────────────────────────────────────

  /** List all conversations visible to the logged-in principal. */
  async listConversations(): Promise<
    Array<{ id: string; name: string; type: string }>
  > {
    const pid = this.getPrincipalId();
    return this.request<Array<{ id: string; name: string; type: string }>>(
      'GET',
      `/v1/conversations?principal_id=${pid}`,
    );
  }

  /** Create a group conversation. */
  async createGroup(params: {
    name: string;
    member_ids: string[];
  }): Promise<{ id: string; name: string }> {
    // Gateway /v1/groups requires `actor_id` to attribute the creation.
    // Without it the handler 400s — historic bug reported by review.
    return this.request<{ id: string; name: string }>('POST', '/v1/groups', {
      actor_id: this.getPrincipalId(),
      name: params.name,
      member_ids: params.member_ids,
    });
  }

  // ── agents ──────────────────────────────────────────────────────────

  /**
   * Provision an agent via the Next.js /api/agents/provision route.
   * The Next.js route expects `driver_type` (not `driver`) and an
   * session cookie for auth, because it does a multi-step flow
   * against the gateway on behalf of the caller.
   */
  async provisionAgent(params: {
    name: string;
    driver: string;
    instructions?: string;
  }): Promise<{ principal_id: string }> {
    // Map to the field the Next.js route reads (`driver_type`).
    return this.request<{ principal_id: string }>(
      'POST',
      '/api/agents/provision',
      {
        name: params.name,
        driver_type: params.driver,
        instructions: params.instructions,
      },
    );
  }

  // ── webhooks ────────────────────────────────────────────────────────

  /** Register a webhook URL for the given principal's events. */
  async setEventWebhook(params: {
    actorId: string;
    url: string;
    secret: string;
  }): Promise<void> {
    await this.request<void>(
      'POST',
      `/v1/principals/${params.actorId}/event-webhook`,
      {
        actor_id: params.actorId,
        url: params.url,
        event_types: ['message.created'],
        secret: params.secret,
      },
    );
  }
}
