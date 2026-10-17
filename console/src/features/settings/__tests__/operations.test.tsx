import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { http, HttpResponse } from 'msw';
import { beforeEach, describe, expect, it } from 'vitest';

import { emptySlot } from '@/lib/slot';
import { server } from '@/mocks/node';
import { signInAs } from '@/test/utils';

import { ConfigSection } from '../components/config-section';
import { ObjectivesSection } from '../components/objectives-section';
import { useConfigDoc, useGatewaySettings, useOrgSettings } from '../store';

const meta = { etag: '"7"', generation: 7, created_at: null, updated_at: null, managed_by: 'api' };
const list = (items: unknown[]) => HttpResponse.json({ items, next_cursor: null });
/** A POST to an `<path>:<verb>` operation (msw reads `:verb` in a string path as a parameter). */
const action = (path: string) => (r: { request: Request }) => new URL(r.request.url).pathname.endsWith(path);

function resetConfig() {
  useConfigDoc.setState({
    document: emptySlot(),
    draft: '',
    parseError: null,
    diff: emptySlot(),
    validation: emptySlot(),
    applied: emptySlot(),
    appliedBy: null,
    convergence: null,
  });
  useGatewaySettings.setState({ admin: emptySlot() });
}

/** A gateway at version 7 whose apply makes version 8; `converged` answers each convergence call. */
function serveConfig(opts: {
  converged?: { converged: boolean; lagging?: string[] }[];
  validate?: { valid: boolean; errors: unknown[]; warnings: unknown[] };
}) {
  const asked: string[] = [];
  const applied: unknown[] = [];
  let validated = 0;
  const answers = [...(opts.converged ?? [{ converged: true }])];
  server.use(
    http.get('*/admin/v1/config', () =>
      HttpResponse.json({ routes: {} }, { headers: { ETag: applied.length ? '"8"' : '"7"' } }),
    ),
    http.get('*/admin/v1/config/generations', () => list([])),
    http.get('*/admin/v1/settings/admin', () => HttpResponse.json({ spec: {}, metadata: meta })),
    http.post(action('/config:diff'), () =>
      HttpResponse.json({
        base_generation: 7,
        changes: [{ op: 'update', path: 'routes', before: {}, after: { a: 1 } }],
        static_ignored: [],
        warnings: [],
      }),
    ),
    http.post(action('/config:validate'), () => {
      validated += 1;
      return HttpResponse.json(opts.validate ?? { valid: true, errors: [], warnings: [] });
    }),
    http.post(action('/config:apply'), async ({ request }) => {
      applied.push(await request.json());
      return HttpResponse.json({ generation: 8, changes: [], static_ignored: [], propagated: false });
    }),
    http.get('*/admin/v1/cluster/converged', ({ request }) => {
      asked.push(new URL(request.url).search);
      return HttpResponse.json(answers.length > 1 ? answers.shift() : answers[0]);
    }),
  );
  return { asked, applied, validated: () => validated };
}

async function editAndReview() {
  const editor = await screen.findByRole('textbox', { name: 'The configuration as JSON' });
  await userEvent.clear(editor);
  await userEvent.click(editor);
  await userEvent.paste('{"routes":{"a":1}}');
  await userEvent.click(screen.getByRole('button', { name: 'Review changes' }));
}

describe('Configuration: review and apply', () => {
  beforeEach(() => {
    resetConfig();
  });

  it('checks the draft with the gateway when reviewing, and blocks apply on a problem', async () => {
    signInAs(['config:read', 'config:apply']);
    const { validated } = serveConfig({
      validate: {
        valid: false,
        errors: [{ path: 'routes.a', code: 'type', message: 'must be an object' }],
        warnings: [{ path: 'routes', code: 'deprecated', message: 'routes will be renamed' }],
      },
    });
    render(<ConfigSection />);
    await editAndReview();
    expect(await screen.findByText('One problem to fix before you can apply')).toBeInTheDocument();
    expect(screen.getByText('routes.a')).toBeInTheDocument();
    expect(screen.getByText(/must be an object/)).toBeInTheDocument();
    expect(screen.getByText(/routes will be renamed/)).toBeInTheDocument();
    expect(validated()).toBe(1);
    expect(screen.getByRole('button', { name: 'Apply 1 change' })).toBeDisabled();
  });

  it('keeps the apply result and follows the new version to every node', async () => {
    signInAs(['config:read', 'config:apply', 'cluster:read']);
    const { asked, applied } = serveConfig({
      converged: [{ converged: false, lagging: ['node-b'] }, { converged: true }],
    });
    render(<ConfigSection />);
    await editAndReview();
    const apply = await screen.findByRole('button', { name: 'Apply 1 change' });
    await waitFor(() => expect(apply).toBeEnabled());
    await userEvent.click(apply);
    const dialog = await screen.findByRole('alertdialog');
    await userEvent.click(within(dialog).getByRole('button', { name: 'Apply changes' }));
    await waitFor(() => expect(applied).toHaveLength(1));
    expect(
      await screen.findByText('Applied as version 8, live on every node', { exact: false }),
    ).toBeInTheDocument();
    expect(asked[0]).toContain('config_version=8');
    expect(asked[0]).toContain('timeout=10s');
    expect(asked).toHaveLength(2);
  });

  it('names the nodes that still lag', async () => {
    signInAs(['config:read', 'config:apply', 'cluster:read']);
    serveConfig({ converged: [{ converged: false, lagging: ['node-b', 'node-c'] }] });
    render(<ConfigSection />);
    await editAndReview();
    const apply = await screen.findByRole('button', { name: 'Apply 1 change' });
    await waitFor(() => expect(apply).toBeEnabled());
    await userEvent.click(apply);
    await userEvent.click(
      within(await screen.findByRole('alertdialog')).getByRole('button', { name: 'Apply changes' }),
    );
    expect(await screen.findByText(/still not on node-b and node-c after a minute/)).toBeInTheDocument();
    expect(screen.getByRole('link', { name: 'See the nodes' })).toBeInTheDocument();
  });

  it('falls back to the apply answer without permission to read the cluster', async () => {
    signInAs(['config:read', 'config:apply']);
    const { asked } = serveConfig({});
    render(<ConfigSection />);
    await editAndReview();
    const apply = await screen.findByRole('button', { name: 'Apply 1 change' });
    await waitFor(() => expect(apply).toBeEnabled());
    await userEvent.click(apply);
    await userEvent.click(
      within(await screen.findByRole('alertdialog')).getByRole('button', { name: 'Apply changes' }),
    );
    expect(await screen.findByText(/Applied as version 8, reaching every node/)).toBeInTheDocument();
    expect(asked).toHaveLength(0);
  });
});

describe('Objectives', () => {
  beforeEach(() => {
    useOrgSettings.setState({ observability: emptySlot() });
  });

  function serveSlos() {
    const patches: unknown[] = [];
    let slos: Record<string, unknown> = {
      'chat-availability': {
        kind: 'availability',
        objective: 0.999,
        window: '30d',
        scope: { model: 'gpt-4.1' },
      },
    };
    let generation = 7;
    server.use(
      http.get('*/admin/v1/settings/observability', () =>
        HttpResponse.json({ spec: { slos }, metadata: { ...meta, generation, etag: `"${generation}"` } }),
      ),
      http.patch('*/admin/v1/settings/observability', async ({ request }) => {
        const body = (await request.json()) as { spec: { slos: Record<string, unknown> } };
        patches.push(body.spec);
        for (const [k, v] of Object.entries(body.spec.slos)) {
          if (v === null) {
            const { [k]: _, ...rest } = slos;
            slos = rest;
          } else slos = { ...slos, [k]: { ...(slos[k] as object), ...(v as object) } };
        }
        generation += 1;
        return HttpResponse.json({ spec: { slos }, metadata: { ...meta, generation } });
      }),
      http.get('*/admin/v1/runtime/slos', () =>
        list([
          {
            name: 'chat-availability',
            kind: 'availability',
            objective: 0.999,
            attained: 0.9994,
            error_budget_remaining: 0.4,
            burn_rate_1h: 2.5,
            burn_rate_6h: 0.7,
            window: '30d',
          },
        ]),
      ),
    );
    return patches;
  }

  it('lists objectives with their status, then adds, edits and removes one', async () => {
    signInAs(['config:read', 'config:apply', 'analytics:read', 'health:read']);
    const patches = serveSlos();
    render(<ObjectivesSection />);
    const table = await screen.findByRole('table', { name: 'Objectives' });
    const row = within(table).getByText('chat-availability').closest('tr') ?? table;
    expect(within(row).getByText('99.9%')).toBeInTheDocument();
    expect(await within(row).findByText('99.940%')).toBeInTheDocument();
    expect(within(row).getByText('40%')).toBeInTheDocument();
    expect(within(row).getByText('2.5×')).toBeInTheDocument();
    expect(within(row).getByText('Model gpt-4.1')).toBeInTheDocument();

    await userEvent.click(screen.getByRole('button', { name: 'Add objective' }));
    const drawer = await screen.findByRole('dialog', { name: 'Add objective' });
    await userEvent.type(within(drawer).getByRole('textbox', { name: 'Name' }), 'chat-latency');
    await userEvent.click(within(drawer).getByRole('radio', { name: 'Requests answered in time' }));
    await userEvent.type(
      within(drawer).getByRole('spinbutton', { name: 'In time means within (ms)' }),
      '2000',
    );
    const target = within(drawer).getByRole('spinbutton', { name: 'Target (%)' });
    await userEvent.clear(target);
    await userEvent.type(target, '95');
    await userEvent.click(within(drawer).getByRole('button', { name: 'Add objective' }));
    await waitFor(() =>
      expect(patches[0]).toEqual({
        slos: {
          'chat-latency': { kind: 'latency', objective: 0.95, latency_threshold_ms: 2000, window: '30d' },
        },
      }),
    );
    expect(await within(table).findByText('chat-latency')).toBeInTheDocument();

    await userEvent.click(screen.getByRole('button', { name: 'Actions for chat-availability' }));
    await userEvent.click(await screen.findByRole('menuitem', { name: 'Edit' }));
    const edit = await screen.findByRole('dialog', { name: 'Objective chat-availability' });
    await userEvent.click(within(edit).getByRole('button', { name: 'Remove condition 1' }));
    await userEvent.click(within(edit).getByRole('button', { name: 'Save objective' }));
    await waitFor(() => expect(patches[1]).toEqual({ slos: { 'chat-availability': { scope: null } } }));

    await userEvent.click(screen.getByRole('button', { name: 'Actions for chat-latency' }));
    await userEvent.click(await screen.findByRole('menuitem', { name: 'Remove objective' }));
    const confirm = await screen.findByRole('alertdialog');
    await userEvent.click(within(confirm).getByRole('button', { name: 'Remove objective' }));
    await waitFor(() => expect(patches[2]).toEqual({ slos: { 'chat-latency': null } }));
  });

  it('refuses a target of 100% or more', async () => {
    signInAs(['config:read', 'config:apply']);
    const patches = serveSlos();
    render(<ObjectivesSection />);
    await userEvent.click(await screen.findByRole('button', { name: 'Add objective' }));
    const drawer = await screen.findByRole('dialog', { name: 'Add objective' });
    await userEvent.type(within(drawer).getByRole('textbox', { name: 'Name' }), 'perfect');
    const target = within(drawer).getByRole('spinbutton', { name: 'Target (%)' });
    await userEvent.clear(target);
    await userEvent.type(target, '100');
    await userEvent.click(within(drawer).getByRole('button', { name: 'Add objective' }));
    expect(await within(drawer).findByText(/above 0 and below 100/)).toBeInTheDocument();
    expect(patches).toHaveLength(0);
  });
});
