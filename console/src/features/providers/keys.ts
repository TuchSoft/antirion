import type { DeploymentHealth, DeploymentResource, ProviderResource } from '@/lib/api/types';
import { parseDuration } from '@/lib/format';

import type { ProviderStatus, ProviderTestResult } from './api';

export type RateLimits = NonNullable<ProviderStatus['rate_limits']>;

/** The name the gateway gives the key of a provider with one `api_key`. */
export const SINGLE_KEY = 'default';

export interface KeyState {
  state: 'working' | 'resting' | 'not_working' | 'untested' | 'unknown';
  /** Why it is not working, as the gateway put it. */
  reason: string | null;
  /** When it is tried again (not working) or used again (resting). */
  until: string | null;
}

export interface ProviderKeyRow {
  name: string;
  hint: string | undefined;
  weight: number | null;
  rotatedAt: string | null;
  health: KeyState;
  readings: RateLimits | null;
}

/** The provider's named API keys, or the one key of `api_key`. */
function keyNames(p: ProviderResource): { name: string; weight: number | null; hintPath: string }[] {
  const auth = p.spec.auth;
  if (auth?.api_keys && auth.api_keys.length > 0)
    return auth.api_keys.map((k, i) => ({
      name: k.name,
      weight: k.weight ?? 1,
      hintPath: `auth.api_keys[${String(i)}].key`,
    }));
  if (auth?.api_key) return [{ name: SINGLE_KEY, weight: null, hintPath: 'auth.api_key' }];
  return [];
}

/** A key's state over the deployments of its provider: dead anywhere wins, then resting. */
export function keyState(name: string, deployments: string[], health: DeploymentHealth[] | null): KeyState {
  if (!health) return { state: 'unknown', reason: null, until: null };
  const entries = health
    .filter((h) => deployments.includes(h.name))
    .flatMap((h) => h.keys.filter((k) => k.key === name));
  const dead = entries.find((k) => k.scope_state === 'dead');
  if (dead) return { state: 'not_working', reason: dead.dead_reason ?? null, until: dead.recheck_at ?? null };
  const resting = entries.filter((k) => k.pair_state !== 'alive');
  if (resting.length > 0) {
    const until = resting
      .map((k) => k.cooldown_until ?? null)
      .filter((t): t is string => t !== null)
      .sort()
      .pop();
    return {
      state: resting.some((k) => k.pair_state === 'dead') ? 'not_working' : 'resting',
      reason: null,
      until: until ?? null,
    };
  }
  // Nothing failing is no evidence: a key works once an attempt with it has succeeded.
  if (!entries.some((k) => k.last_success_at)) return { state: 'untested', reason: null, until: null };
  return { state: 'working', reason: null, until: null };
}

/**
 * The provider's key status for its table row, from evidence: the gateway's credential state
 * when it failed or refreshes, else the last test in this session, else "Not tested yet" until
 * one of its keys has answered.
 */
export function providerKeyStatus(
  credentialState: string | undefined,
  keys: ProviderKeyRow[],
  test?: Pick<ProviderTestResult, 'ok'> | null,
): string | undefined {
  // A gateway that names a tested key's state says `working`.
  const state = credentialState === 'working' ? 'valid' : credentialState;
  if (state !== 'valid') return state;
  if (test) return test.ok ? 'valid' : 'failed';
  if (keys.length === 0) return state;
  return keys.some((k) => k.health.state === 'working') ? 'valid' : 'untested';
}

/**
 * Keys' states with the tests of this session: a key's own test, or the provider's when it has
 * one key. A passing test is evidence where its deployments have none yet; a failing one wins.
 */
export function withTest(
  keys: ProviderKeyRow[],
  test: ProviderTestResult | undefined,
  keyTests: Record<string, ProviderTestResult | undefined> = {},
): ProviderKeyRow[] {
  return keys.map((k) => {
    const t = keyTests[k.name] ?? (keys.length === 1 ? test : undefined);
    if (!t) return k;
    if (!t.ok) return { ...k, health: { state: 'not_working', reason: 'test', until: null } };
    return k.health.state === 'untested' || k.health.state === 'unknown'
      ? { ...k, health: { state: 'working', reason: null, until: null } }
      : k;
  });
}

/**
 * A key's state as the gateway reports it on `/runtime/providers`, when it does: working, cooling
 * down (rate limited) until a time, dead (the provider refused it) or not tested.
 */
function reportedState(credential: unknown): KeyState | null {
  const c = credential as { state?: unknown; until?: unknown; reason?: unknown };
  if (typeof c.state !== 'string') return null;
  const until = typeof c.until === 'string' ? c.until : null;
  const reason = typeof c.reason === 'string' ? c.reason : null;
  if (c.state === 'working' || c.state === 'valid') return { state: 'working', reason: null, until: null };
  if (c.state === 'cooling' || c.state === 'rate_limited') return { state: 'resting', reason, until };
  if (c.state === 'dead' || c.state === 'failed') return { state: 'not_working', reason, until };
  if (c.state === 'untested') return { state: 'untested', reason: null, until: null };
  return null;
}

/** Each key of a provider with its last replacement, its state and its latest rate-limit reading. */
export function providerKeys(
  p: ProviderResource,
  status: ProviderStatus | undefined,
  deployments: DeploymentResource[] | null,
  health: DeploymentHealth[] | null,
): ProviderKeyRow[] {
  const mine = (deployments ?? []).filter((d) => d.spec.provider === p.name).map((d) => d.name);
  const rotated = p.metadata.credential_rotated_at ?? {};
  const hints = (p.metadata.secret_hints ?? {}) as Record<string, string>;
  const credentials = status?.credentials ?? [];
  const named = keyNames(p);
  const rows: ProviderKeyRow[] = named.map((k) => {
    const reported = credentials.find((c) => c.name === k.name);
    return {
      name: k.name,
      hint: hints[k.hintPath],
      weight: k.weight,
      rotatedAt: rotated[k.name] ?? null,
      health: (reported && reportedState(reported)) ?? keyState(k.name, mine, health),
      readings: (reported?.rate_limits as RateLimits | undefined) ?? null,
    };
  });
  for (const c of credentials) {
    if (rows.some((r) => r.name === c.name)) continue;
    rows.push({
      name: c.name,
      hint: undefined,
      weight: null,
      rotatedAt: rotated[c.name] ?? null,
      health: { state: 'unknown', reason: null, until: null },
      readings: c.rate_limits,
    });
  }
  return rows;
}

export interface Rotation {
  /** Age of the oldest credential, in days. */
  ageDays: number;
  maxDays: number | null;
  due: boolean;
}

/** How old the provider's oldest credential is, against its rotation policy. */
export function rotation(p: ProviderResource, now: number): Rotation | null {
  const times = Object.values(p.metadata.credential_rotated_at ?? {})
    .map((t) => Date.parse(t))
    .filter((t) => !Number.isNaN(t));
  if (times.length === 0) return null;
  const ageDays = Math.max(0, (now - Math.min(...times)) / 86_400_000);
  const max = p.spec.credential_rotation_max_age ? parseDuration(p.spec.credential_rotation_max_age) : null;
  return {
    ageDays,
    maxDays: max === null ? null : max / 86_400,
    due: p.metadata.rotation_due === true,
  };
}

/** "12 days", "less than a day". */
export function ageText(days: number): string {
  if (days < 1) return 'less than a day';
  const n = Math.floor(days);
  return n === 1 ? '1 day' : `${String(n)} days`;
}
