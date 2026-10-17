import { z } from 'zod';

import { money, resourceName } from '@/lib/validation';

export type Enforcement = 'soft' | 'hard' | 'approval' | 'downgrade';
export type BudgetUnit = 'usd' | 'tokens';
/** Which tokens a token budget counts: all of them, one direction, or weighted by pricing class. */
export type TokenCount = 'total' | 'input' | 'output' | 'weighted';
export type Period = 'daily' | 'weekly' | 'monthly' | 'quarterly' | 'yearly' | 'lifetime' | 'rolling';

/** The rolling windows the drawer offers (any duration is kept as written). */
export const ROLLING_WINDOWS = ['1h', '24h', '7d', '30d'] as const;

export interface BudgetFormValues {
  level: 'organization' | 'team' | 'project' | 'key';
  owner: string;
  name: string;
  /** What the amount counts: dollars, or tokens. */
  unit: BudgetUnit;
  /** With `tokens`: which tokens count. */
  tokens: TokenCount;
  amount: string;
  period: Period;
  /** With `rolling`: the window's length, a duration such as `7d`. */
  window: string;
  enforcement: Enforcement;
  alerts: string;
  hardCap: string;
  budgetOwner: string;
  downgradeAt: string;
  /** A downgrade's target: a Model or a tier. */
  downgradeTo: 'model' | 'tier';
  downgradeModel: string;
  downgradeTier: string;
  /** `cache_hits_bypass_budgets`: a request this budget refuses may still be served from the cache. */
  cacheBypass: boolean;
}

export const EMPTY_BUDGET: BudgetFormValues = {
  level: 'team',
  owner: '',
  name: 'monthly',
  unit: 'usd',
  tokens: 'total',
  amount: '',
  period: 'monthly',
  window: '7d',
  enforcement: 'soft',
  alerts: '50, 80, 100',
  hardCap: '',
  budgetOwner: '',
  downgradeAt: '90',
  downgradeTo: 'model',
  downgradeModel: '',
  downgradeTier: 'fast',
  cacheBypass: false,
};

const percentList = z
  .string()
  .refine(
    (v) => v.trim() === '' || v.split(',').every((p) => /^\s*\d{1,3}\s*$/.test(p) && Number(p) <= 100),
    'Percentages from 0 to 100, separated by commas',
  )
  .refine((v) => {
    const n = v
      .split(',')
      .filter((p) => p.trim())
      .map(Number);
    return n.every((x, i) => i === 0 || x > (n[i - 1] ?? -1));
  }, 'Each threshold above the previous one');

const DURATION = /^\d+(ms|s|m|h|d)$/;

export const budgetSchema = z
  .object({
    level: z.enum(['organization', 'team', 'project', 'key']),
    owner: resourceName,
    name: z.string().regex(/^[a-z0-9_]{1,64}$/, 'Lowercase letters, digits and _'),
    unit: z.enum(['usd', 'tokens']),
    tokens: z.enum(['total', 'input', 'output', 'weighted']),
    amount: z.string().trim(),
    period: z.enum(['daily', 'weekly', 'monthly', 'quarterly', 'yearly', 'lifetime', 'rolling']),
    window: z.string().trim(),
    enforcement: z.enum(['soft', 'hard', 'approval', 'downgrade']),
    alerts: percentList,
    hardCap: z
      .string()
      .trim()
      .refine((v) => v === '' || /^\d{2,3}$/.test(v), 'A percentage such as 110'),
    budgetOwner: z.string(),
    downgradeAt: z
      .string()
      .trim()
      .regex(/^\d{1,3}$/, 'A percentage'),
    downgradeTo: z.enum(['model', 'tier']),
    downgradeModel: z.string(),
    downgradeTier: z.string(),
    cacheBypass: z.boolean(),
  })
  .refine((v) => (v.unit === 'tokens' ? /^\d+$/.test(v.amount) : money.safeParse(v.amount).success), {
    path: ['amount'],
    message: 'Enter an amount',
  })
  .refine((v) => v.hardCap === '' || v.enforcement === 'soft', {
    path: ['hardCap'],
    message: 'A hard cap only adds to an alert-only budget',
  })
  .refine((v) => v.hardCap === '' || Number(v.hardCap) > 100, {
    path: ['hardCap'],
    message: 'The cap is above 100%',
  })
  .refine((v) => v.period !== 'rolling' || DURATION.test(v.window), {
    path: ['window'],
    message: 'A length such as 24h or 7d',
  })
  .refine((v) => v.enforcement !== 'downgrade' || v.downgradeTo !== 'model' || v.downgradeModel !== '', {
    path: ['downgradeModel'],
    message: 'Choose the Model to switch to',
  });

/**
 * The declaration(s) the form describes: the budget, and a hard twin at the cap when one is set.
 * Members the form turned off are `null`, so the merge patch removes them from the stored budget.
 */
export function toDeclarations(v: BudgetFormValues): Record<string, Record<string, unknown> | null> {
  const alerts = v.alerts
    .split(',')
    .map((p) => p.trim())
    .filter(Boolean)
    .map((p) => Number(p) / 100);
  const rolling = v.period === 'rolling';
  const period = rolling ? v.window.trim() : v.period;
  const tokens = v.unit === 'tokens';
  const base: Record<string, unknown> = {
    unit: v.unit,
    amount: amountOf(v.unit, v.amount),
    // `tokens` and `weights` only exist on a token budget; the default count is left unwritten.
    tokens: tokens ? (v.tokens === 'total' ? undefined : v.tokens) : null,
    ...(tokens ? {} : { weights: null }),
    period,
    algorithm: rolling ? 'sliding_window' : null,
    ...(rolling ? {} : { precision: null }),
    mode: v.enforcement === 'approval' ? 'approval' : v.enforcement === 'hard' ? 'hard' : 'soft',
    alert_at: alerts.length > 0 ? alerts : null,
    owner: v.budgetOwner ? v.budgetOwner : null,
    downgrade:
      v.enforcement === 'downgrade'
        ? {
            at: Number(v.downgradeAt) / 100,
            ...(v.downgradeTo === 'model'
              ? { to_model: v.downgradeModel, to_tier: null }
              : { to_tier: v.downgradeTier, to_model: null }),
          }
        : null,
    cache_hits_bypass_budgets: v.cacheBypass ? true : null,
  };
  if (base.tokens === undefined) delete base.tokens;
  const out: Record<string, Record<string, unknown> | null> = { [v.name]: base };
  const capName = `${v.name}_cap`.slice(0, 64);
  if (v.hardCap) {
    const capAmount = (Number(v.amount) * Number(v.hardCap)) / 100;
    out[capName] = {
      unit: v.unit,
      amount: tokens ? Math.round(capAmount) : String(capAmount),
      ...(tokens && v.tokens !== 'total' ? { tokens: v.tokens } : {}),
      period,
      algorithm: rolling ? 'sliding_window' : null,
      mode: 'hard',
    };
  } else {
    out[capName] = null;
  }
  return out;
}

/** An amount as a declaration writes it: a decimal string of dollars, or a whole number of tokens. */
export function amountOf(unit: BudgetUnit, amount: string): string | number {
  return unit === 'tokens' ? Number(amount.trim()) : amount.trim();
}

/** A merge patch that sets or removes one partition value's own amount. */
export function toOverride(
  name: string,
  value: string,
  amount: string | null,
  unit: BudgetUnit = 'usd',
): Record<string, Record<string, unknown>> {
  return { [name]: { overrides: { [value]: amount === null ? null : amountOf(unit, amount) } } };
}

/** The unit a declaration counts in; a declaration without one is read as dollars. */
export function unitOf(decl: Record<string, unknown> | undefined): BudgetUnit {
  return decl?.unit === 'tokens' ? 'tokens' : 'usd';
}

const PERIODS = ['daily', 'weekly', 'monthly', 'quarterly', 'yearly', 'lifetime'];

export function fromDeclaration(
  level: BudgetFormValues['level'],
  owner: string,
  name: string,
  decl: Record<string, unknown> | undefined,
  cap: Record<string, unknown> | undefined,
): BudgetFormValues {
  const d = decl ?? {};
  // A declaration without a mode blocks at the limit (the default is `hard`).
  const mode = typeof d.mode === 'string' ? d.mode : decl ? 'hard' : 'soft';
  const tokens = ['total', 'input', 'output', 'weighted'].includes(d.tokens as string)
    ? (d.tokens as TokenCount)
    : 'total';
  const downgrade = d.downgrade as { at?: number; to_tier?: string; to_model?: string } | undefined;
  const alerts = Array.isArray(d.alert_at)
    ? (d.alert_at as unknown[]).map((a) => String(Math.round(Number(a) * 100))).join(', ')
    : '';
  const amount = typeof d.amount === 'string' || typeof d.amount === 'number' ? String(d.amount) : '';
  const capAmount =
    cap && (typeof cap.amount === 'string' || typeof cap.amount === 'number') ? Number(cap.amount) : null;
  const period = typeof d.period === 'string' ? d.period : 'monthly';
  const rolling = d.algorithm === 'sliding_window' || !PERIODS.includes(period);
  return {
    level,
    owner,
    name,
    unit: unitOf(decl),
    tokens,
    amount,
    period: rolling ? 'rolling' : (period as Period),
    window: rolling ? period : '7d',
    enforcement: downgrade
      ? 'downgrade'
      : mode === 'approval'
        ? 'approval'
        : mode === 'hard'
          ? 'hard'
          : 'soft',
    alerts,
    hardCap: capAmount && Number(amount) > 0 ? String(Math.round((capAmount / Number(amount)) * 100)) : '',
    budgetOwner: typeof d.owner === 'string' ? d.owner : '',
    downgradeAt: downgrade?.at !== undefined ? String(Math.round(downgrade.at * 100)) : '90',
    downgradeTo: downgrade?.to_tier ? 'tier' : 'model',
    downgradeModel: downgrade?.to_model ?? '',
    downgradeTier: downgrade?.to_tier ?? 'fast',
    cacheBypass: d.cache_hits_bypass_budgets === true,
  };
}

/** The Models a declaration covers, in words ("All Models", "gpt-4o*, claude-*", "All but …"). */
export function modelsCovered(decl: Record<string, unknown> | undefined): {
  label: string;
  patterns: string[];
  all: boolean;
} {
  const match = (decl?.match as { models?: string[] } | undefined)?.models ?? [];
  const exclude = (decl?.exclude as { models?: string[] } | undefined)?.models ?? [];
  if (match.length === 0 && exclude.length === 0) return { label: 'All Models', patterns: [], all: true };
  if (match.length === 0)
    return { label: `All Models but ${exclude.join(', ')}`, patterns: exclude, all: false };
  return {
    label: `${match.join(', ')}${exclude.length ? ` (not ${exclude.join(', ')})` : ''}`,
    patterns: match,
    all: false,
  };
}
