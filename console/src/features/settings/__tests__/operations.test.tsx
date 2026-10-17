import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { http, HttpResponse } from 'msw';
import { beforeEach, describe, expect, it } from 'vitest';

import { emptySlot } from '@/lib/slot';
import { server } from '@/mocks/node';
import { signInAs } from '@/test/utils';

import { ConfigSection } from '../components/config-section';
import { useConfigDoc, useGatewaySettings } from '../store';

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
