import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { http, HttpResponse } from 'msw';
import { beforeEach, describe, expect, it } from 'vitest';

import { emptySlot } from '@/lib/slot';
import { server } from '@/mocks/node';
import { pickOption, signInAs } from '@/test/utils';

import { BudgetsPricingSection, pricingValues } from '../components/budgets-pricing-section';
import { useTraffic } from '../traffic-store';

const meta = {
  etag: '"5"',
  generation: 5,
  created_at: '2026-10-01T00:00:00Z',
  updated_at: '2026-10-01T00:00:00Z',
  managed_by: 'api',
};

function resetTraffic() {
  useTraffic.setState({
    budgets: emptySlot(),
    pricing: emptySlot(),
    cache: emptySlot(),
    guardrails: emptySlot(),
    resilience: emptySlot(),
    routing: emptySlot(),
    server: emptySlot(),
    policies: emptySlot(),
  });
}

/** Serves `/settings/<key>` from `spec` and records each PATCH body. */
function settings(key: string, spec: Record<string, unknown>) {
  const patches: unknown[] = [];
  server.use(
    http.get(`*/admin/v1/settings/${key}`, () => HttpResponse.json({ spec, metadata: meta })),
    http.patch(`*/admin/v1/settings/${key}`, async ({ request }) => {
      patches.push(await request.json());
      return HttpResponse.json({ spec, metadata: meta });
    }),
  );
  return patches;
}

const section = (title: string) =>
  screen.getByText(title, { selector: 'h3' }).closest('section') ?? document.body;

describe('Budgets & pricing', () => {
  beforeEach(() => {
    resetTraffic();
    signInAs(['pricing:read', 'pricing:write', 'policies:read', 'policies:write', 'config:read']);
    server.use(
      http.get('*/admin/v1/settings/observability', () =>
        HttpResponse.json({ spec: { alerts: { webhooks: { finance: {} } } }, metadata: meta }),
      ),
    );
  });

  it('starts from the default for unpriced requests while nothing is stored', () => {
    expect(pricingValues({})).toEqual({
      onUnpriced: 'reject_if_budgeted',
      catalog: 'bundled',
      inBody: false,
    });
    expect(pricingValues({ on_unpriced: 'zero' }).onUnpriced).toBe('zero');
  });

  it('counts unpriced requests as free, writing only on_unpriced', async () => {
    const pricing = settings('pricing', {});
    settings('budgets', {});
    render(<BudgetsPricingSection />);
    const free = await screen.findByRole('radio', { name: /Count them as free/ });
    expect(screen.getByRole('radio', { name: /Refuse them when a budget applies/ })).toBeChecked();
    await userEvent.click(free);
    await userEvent.click(within(section('Prices')).getByRole('button', { name: 'Save changes' }));
    await waitFor(() => expect(pricing).toEqual([{ spec: { on_unpriced: 'zero' } }]));
  });

  it('refuses them again from a stored "zero"', async () => {
    const pricing = settings('pricing', { on_unpriced: 'zero' });
    settings('budgets', {});
    render(<BudgetsPricingSection />);
    await userEvent.click(await screen.findByRole('radio', { name: /Refuse them when a budget applies/ }));
    await userEvent.click(within(section('Prices')).getByRole('button', { name: 'Save changes' }));
    await waitFor(() => expect(pricing).toEqual([{ spec: { on_unpriced: 'reject_if_budgeted' } }]));
  });

  it('sends budget threshold alerts to a webhook and keeps going while the store is down', async () => {
    settings('pricing', {});
    const budgets = settings('budgets', { expose_headers: true });
    render(<BudgetsPricingSection />);
    const card = await waitFor(() => section('Budgets'));
    expect(await within(card).findByRole('switch', { name: /Tell apps what is left/ })).toBeChecked();
    await pickOption(within(card).getByRole('combobox', { name: /If the budget store/ }), /Keep going/);
    await pickOption(within(card).getByRole('combobox', { name: 'Send threshold alerts to' }), 'finance');
    await userEvent.click(within(card).getByRole('button', { name: 'Save changes' }));
    await waitFor(() =>
      expect(budgets).toEqual([
        { spec: { on_store_unavailable: 'local_share', alerts: { sink: 'finance' } } },
      ]),
    );
  });

  it('shows the settings read-only without the write permission', async () => {
    signInAs(['pricing:read', 'policies:read']);
    settings('pricing', {});
    settings('budgets', {});
    render(<BudgetsPricingSection />);
    expect(await screen.findByRole('radio', { name: /Count them as free/ })).toBeDisabled();
    expect(within(section('Prices')).getByRole('button', { name: 'Save changes' })).toBeDisabled();
  });
});
