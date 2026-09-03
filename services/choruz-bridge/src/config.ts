import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import yaml from 'js-yaml';

export interface BridgeConfig {
  choruz: {
    api_url: string;
    username: string;
    password: string;
  };
  slack?: {
    bot_token: string;
    app_token: string;
  };
  telegram?: {
    bot_token: string;
  };
  webhook: {
    port: number;
    secret: string;
  };
  database: {
    connection_string: string;
  };
}

export function loadConfig(): BridgeConfig {
  const configPath = process.env['CHORUZ_BRIDGE_CONFIG'] ?? 'choruz-bridge.yaml';
  const resolved = resolve(configPath);

  let raw: string;
  try {
    raw = readFileSync(resolved, 'utf-8');
  } catch {
    console.error(`Failed to read config file: ${resolved}`);
    process.exit(1);
  }

  const parsed = yaml.load(raw) as Record<string, unknown>;

  if (!parsed || typeof parsed !== 'object') {
    console.error('Invalid config: expected a YAML object');
    process.exit(1);
  }

  const choruz = parsed['choruz'] as BridgeConfig['choruz'] | undefined;
  if (!choruz || !choruz.api_url || !choruz.username || !choruz.password) {
    console.error('Missing required choruz config (api_url, username, password)');
    process.exit(1);
  }

  const webhook = (parsed['webhook'] as BridgeConfig['webhook']) ?? {
    port: 3030,
    secret: '',
  };
  if (!webhook.port) {
    webhook.port = 3030;
  }
  if (!webhook.secret) {
    console.error('Missing required webhook.secret config');
    process.exit(1);
  }

  const database = parsed['database'] as BridgeConfig['database'] | undefined;
  if (!database || !database.connection_string) {
    console.error('Missing required database.connection_string config');
    process.exit(1);
  }

  const slack = parsed['slack'] as BridgeConfig['slack'] | undefined;
  const telegram = parsed['telegram'] as BridgeConfig['telegram'] | undefined;

  return { choruz, slack, telegram, webhook, database };
}
