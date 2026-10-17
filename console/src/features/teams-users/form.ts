import { z } from 'zod';

import type { TeamResource, TeamSpec } from '@/lib/api/types';
import { money, resourceName } from '@/lib/validation';

/** The colours a team can take. */
export const TEAM_COLORS = [
  '#3e72cb',
  '#2f9e6f',
  '#c9891a',
  '#cb3e3e',
  '#8b5cf6',
  '#0ea5a4',
  '#d97757',
  '#64748b',
];

/**
 * What happens when the team's monthly budget is reached: alert only, block, or (for a budget set up
 * on the Budgets page with approval or a downgrade) what it states, unchanged.
 */
export type TeamBudgetMode = 'soft' | 'hard' | 'kept';

export const teamSchema = z
  .object({
    name: resourceName,
    displayName: z.string().trim().max(200, 'At most 200 characters'),
    organization: resourceName,
    color: z.string().regex(/^#[0-9a-fA-F]{6}$/, 'A colour like #3366ff'),
    lead: z.string(),
    /** The unit of the stored monthly budget; a new one counts dollars. */
    budgetUnit: z.enum(['usd', 'tokens']),
    budget: z.string().trim(),
    mode: z.enum(['soft', 'hard', 'kept']),
  })
  .refine(
    (v) =>
      v.budget === '' ||
      (v.budgetUnit === 'tokens' ? /^\d+$/.test(v.budget) : money.safeParse(v.budget).success),
    { path: ['budget'], message: 'A monthly amount, or empty' },
  );

export type TeamValues = z.input<typeof teamSchema>;

type Declaration = Record<string, unknown> & { amount?: string | number; mode?: string; unit?: string };

function monthlyOf(spec: TeamSpec | undefined): Declaration | undefined {
  const d = (spec?.policies?.budgets as Record<string, unknown> | undefined)?.monthly;
  return d && typeof d === 'object' ? (d as Declaration) : undefined;
}

/** The team drawer's values for a team, or for a new one in `organization`. */
export function teamValues(team: TeamResource | undefined, organization: string): TeamValues {
  const monthly = monthlyOf(team?.spec);
  // A declaration without a mode blocks (the default is `hard`); approval and downgrade are kept.
  const mode = monthly ? (monthly.mode ?? 'hard') : 'soft';
  return {
    name: team?.name ?? '',
    displayName: team?.spec.display_name ?? '',
    organization: team?.spec.organization ?? organization,
    color: team?.spec.color ?? TEAM_COLORS[0] ?? '#3e72cb',
    lead: team?.spec.lead ?? '',
    budgetUnit: monthly?.unit === 'tokens' ? 'tokens' : 'usd',
    budget: monthly?.amount !== undefined ? String(monthly.amount) : '',
    mode: mode === 'hard' || mode === 'soft' ? mode : 'kept',
  };
}

/**
 * The team spec the drawer describes, written over `base` so everything it does not show stays as
 * stored. The monthly budget is rewritten only when its amount or mode changed, and then only those
 * two members: its unit, alerts, match and the rest stay as declared.
 */
export function toTeamSpec(v: TeamValues, base?: TeamSpec): TeamSpec {
  const before = monthlyOf(base);
  const budgets = { ...((base?.policies?.budgets ?? {}) as Record<string, unknown>) };
  const amount = v.budgetUnit === 'tokens' ? Number(v.budget) : v.budget;
  if (!v.budget) delete budgets.monthly;
  else if (!before)
    budgets.monthly = {
      unit: 'usd',
      amount,
      period: 'monthly',
      mode: v.mode === 'hard' ? 'hard' : 'soft',
      alert_at: [0.5, 0.8, 1],
    };
  else {
    const next: Declaration = { ...before };
    if (String(before.amount) !== v.budget) next.amount = amount;
    if (v.mode !== 'kept' && v.mode !== (before.mode ?? 'hard')) next.mode = v.mode;
    budgets.monthly = next;
  }
  const spec: TeamSpec = { ...(base ?? { organization: v.organization }) };
  spec.organization = v.organization;
  spec.color = v.color;
  if (v.displayName) spec.display_name = v.displayName;
  else delete spec.display_name;
  if (v.lead) spec.lead = v.lead;
  else delete spec.lead;
  const policies: NonNullable<TeamSpec['policies']> = { ...(base?.policies ?? {}) };
  if (Object.keys(budgets).length > 0) policies.budgets = budgets as typeof policies.budgets;
  else delete policies.budgets;
  if (Object.keys(policies).length > 0) spec.policies = policies;
  else delete spec.policies;
  return spec;
}
