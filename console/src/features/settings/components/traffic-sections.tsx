'use client';

import { BudgetsPricingSection } from './budgets-pricing-section';
import type { Section } from './sections';

/**
 * Gateway-wide settings of the traffic itself: what requests cost, what is cached, what guardrails
 * do and how requests are routed and retried. The feature pages edit the parts that belong to one
 * Model, budget or guardrail; these are the defaults and switches behind them.
 */
export const TRAFFIC_SECTIONS: Section[] = [
  {
    id: 'budgets-pricing',
    permission: ['pricing:read', 'policies:read'],
    label: 'Budgets & pricing',
    icon: 'wallet',
    description: 'How unpriced requests are counted, and how budgets behave for every organization.',
    aliases: ['pricing', 'budgets'],
    render: () => <BudgetsPricingSection />,
  },
];
