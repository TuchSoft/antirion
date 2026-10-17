import { describe, expect, it } from 'vitest';

import type { TeamResource } from '@/lib/api/types';

import { teamValues, toTeamSpec } from '../form';

const meta = {
  etag: '"1"',
  generation: 1,
  created_at: '2026-01-01T00:00:00Z',
  updated_at: '2026-01-01T00:00:00Z',
  managed_by: 'api' as const,
};

const team = (spec: TeamResource['spec']): TeamResource => ({ name: 'core', spec, metadata: meta });

describe('team form', () => {
  const spec: TeamResource['spec'] = {
    organization: 'acme',
    display_name: 'Core',
    color: '#2f9e6f',
    lead: 'ada',
    enabled: false,
    tags: ['env:prod'],
    metadata: { cost_center: '42' },
    models: ['gpt-*'],
    policies: {
      budgets: {
        monthly: {
          unit: 'tokens',
          amount: 5_000_000,
          period: 'monthly',
          mode: 'approval',
          alert_at: [0.333],
          match: { models: ['claude-*'] },
          owner: 'ada',
        },
        daily: { unit: 'usd', amount: '20', period: 'daily' },
      },
      limits: { rpm: { unit: 'requests', limit: 100, per: '1m', burst: 10 } },
    },
  };

  it('saves a team unchanged when nothing was edited', () => {
    expect(toTeamSpec(teamValues(team(spec), 'default'), spec)).toEqual(spec);
  });

  it('keeps a token monthly budget in tokens and changes only its amount', () => {
    const values = teamValues(team(spec), 'default');
    expect(values.budgetUnit).toBe('tokens');
    expect(values.mode).toBe('kept');
    const out = toTeamSpec({ ...values, budget: '6000000' }, spec);
    expect(out.policies?.budgets?.monthly).toEqual({
      ...(spec.policies?.budgets?.monthly as object),
      amount: 6_000_000,
    });
  });

  it('declares a new monthly budget in dollars', () => {
    const values = { ...teamValues(undefined, 'acme'), name: 'new', budget: '300', mode: 'hard' as const };
    expect(toTeamSpec(values).policies?.budgets?.monthly).toEqual({
      unit: 'usd',
      amount: '300',
      period: 'monthly',
      mode: 'hard',
      alert_at: [0.5, 0.8, 1],
    });
  });

  it('reads a monthly budget without a mode as blocking', () => {
    const plain = { organization: 'acme', policies: { budgets: { monthly: { unit: 'usd', amount: '5' } } } };
    expect(teamValues(team(plain as TeamResource['spec']), 'acme').mode).toBe('hard');
  });
});
