import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { http, HttpResponse } from 'msw';
import { beforeEach, describe, expect, it } from 'vitest';

import { emptySlot } from '@/lib/slot';
import { server } from '@/mocks/node';
import { pickOption, signInAs } from '@/test/utils';

import { ClientAddressCard, RateLimitCard } from '../components/gateway-forms';
import { ProvisioningSection, SCIM_OVERLAPS } from '../components/org-sections';
import { useOrgSettings } from '../store';

const meta = { etag: '"7"', generation: 7, created_at: null, updated_at: null, managed_by: 'api' };
const settings = (spec: unknown, extra: Record<string, unknown> = {}) =>
  HttpResponse.json({ spec, metadata: { ...meta, ...extra } });
const empty = () => settings({});

function resetSettings() {
  useOrgSettings.setState({
    limits: emptySlot(),
    security: emptySlot(),
    auth: emptySlot(),
    observability: emptySlot(),
  });
}

describe('User provisioning (SCIM)', () => {
  beforeEach(() => {
    signInAs(['users:read', 'security:read', 'security:write']);
    resetSettings();
  });

  function serve(opts: { enabled: boolean; rotatedAt: string | null }) {
    const posts: unknown[] = [];
    const patches: unknown[] = [];
    let enabled = opts.enabled;
    server.use(
      http.get('*/admin/v1/auth/scim/status', () =>
        HttpResponse.json({ enabled, users: 3, groups: 1, last_request_at: null, last_error: null }),
      ),
      http.get('*/admin/v1/settings/auth', () =>
        settings({ admin: { scim: { enabled } } }, { scim_token_rotated_at: opts.rotatedAt }),
      ),
      http.patch('*/admin/v1/settings/auth', async ({ request }) => {
        const body = (await request.json()) as { spec: { admin: { scim: { enabled: boolean } } } };
        patches.push(body);
        enabled = body.spec.admin.scim.enabled;
        return settings(body.spec);
      }),
      http.post('*/admin/v1/settings/auth:rotate_scim_token', async ({ request }) => {
        posts.push(await request.json());
        return HttpResponse.json({ secret: 'scim-new', previous_valid_until: null });
      }),
      http.get('*/admin/v1/settings/limits', empty),
      http.get('*/admin/v1/settings/security', empty),
      http.get('*/admin/v1/settings/observability', empty),
    );
    return { posts, patches };
  }

  it('offers overlaps the API accepts, at most an hour', () => {
    expect(SCIM_OVERLAPS.map((o) => o.value)).toEqual(['0s', '15m', '1h']);
  });

  it('generates the first token with no overlap', async () => {
    const { posts } = serve({ enabled: true, rotatedAt: null });
    render(<ProvisioningSection />);
    expect(await screen.findByText('None yet')).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Generate token' }));
    const dialog = await screen.findByRole('alertdialog');
    expect(within(dialog).queryByText('Old token keeps working for')).not.toBeInTheDocument();
    await userEvent.click(within(dialog).getByRole('button', { name: 'Generate token' }));
    await waitFor(() => expect(posts).toEqual([{ overlap: '0s' }]));
    expect(await screen.findByText('Your SCIM token')).toBeInTheDocument();
  });

  it('replaces a token, keeping the old one for the overlap chosen', async () => {
    const { posts } = serve({ enabled: false, rotatedAt: '2026-10-01T00:00:00Z' });
    render(<ProvisioningSection />);
    await userEvent.click(await screen.findByRole('button', { name: 'Replace token' }));
    const dialog = await screen.findByRole('alertdialog');
    await pickOption(within(dialog).getByLabelText('Old token keeps working for'), '1 hour');
    await userEvent.click(within(dialog).getByRole('button', { name: 'Replace token' }));
    await waitFor(() => expect(posts).toEqual([{ overlap: '1h' }]));
  });

  it('turns provisioning on with its own switch, apart from the token', async () => {
    const { patches } = serve({ enabled: false, rotatedAt: '2026-10-01T00:00:00Z' });
    render(<ProvisioningSection />);
    const toggle = await screen.findByRole('switch', { name: 'User provisioning' });
    expect(toggle).toHaveAttribute('aria-checked', 'false');
    expect(screen.getByRole('button', { name: 'Replace token' })).toBeInTheDocument();
    await userEvent.click(toggle);
    await waitFor(() => expect(patches).toEqual([{ spec: { admin: { scim: { enabled: true } } } }]));
    await waitFor(() => expect(toggle).toHaveAttribute('aria-checked', 'true'));
  });
});

describe('Endpoints & security forms', () => {
  beforeEach(() => {
    signInAs(['policies:read', 'policies:write', 'security:read', 'security:write']);
    resetSettings();
  });

  it('shows the built-in rate limiting values when nothing is stored, and saves only a change', async () => {
    const patches: unknown[] = [];
    server.use(
      http.get('*/admin/v1/settings/limits', empty),
      http.patch('*/admin/v1/settings/limits', async ({ request }) => {
        patches.push(await request.json());
        return empty();
      }),
    );
    void useOrgSettings.getState().loadSettings();
    const { container } = render(<RateLimitCard />);
    expect(await screen.findByRole('switch', { name: 'Enforce rate limits' })).toHaveAttribute(
      'aria-checked',
      'true',
    );
    expect(screen.getByText('Smooth rate (GCRA)')).toBeInTheDocument();
    expect(screen.getByText('Each node enforces its share')).toBeInTheDocument();
    expect(screen.getByLabelText('Limits listed at most')).toHaveValue('10');
    expect(container.textContent).not.toContain('Gateway default');

    await pickOption(screen.getByLabelText('Counting method'), /Sliding window/);
    await userEvent.click(screen.getByRole('button', { name: 'Save changes' }));
    await waitFor(() => expect(patches).toEqual([{ spec: { algorithm: 'sliding_window' } }]));
  });

  it('asks before rate limiting is turned off', async () => {
    const patches: unknown[] = [];
    server.use(
      http.get('*/admin/v1/settings/limits', empty),
      http.patch('*/admin/v1/settings/limits', async ({ request }) => {
        patches.push(await request.json());
        return empty();
      }),
    );
    void useOrgSettings.getState().loadSettings();
    render(<RateLimitCard />);
    await userEvent.click(await screen.findByRole('switch', { name: 'Enforce rate limits' }));
    await userEvent.click(screen.getByRole('button', { name: 'Save changes' }));
    const dialog = await screen.findByRole('alertdialog');
    expect(patches).toEqual([]);
    await userEvent.click(within(dialog).getByRole('button', { name: 'Turn off and save' }));
    await waitFor(() => expect(patches).toEqual([{ spec: { enabled: false } }]));
  });

  it('round-trips the trusted proxies', async () => {
    let stored: Record<string, unknown> = { secrets: { encryption_keys: ['${env:KEY}'] } };
    const patches: unknown[] = [];
    server.use(
      http.get('*/admin/v1/settings/limits', empty),
      http.get('*/admin/v1/settings/security', () => settings(stored)),
      http.patch('*/admin/v1/settings/security', async ({ request }) => {
        const body = (await request.json()) as { spec: Record<string, unknown> };
        patches.push(body);
        stored = { ...stored, ...body.spec };
        return settings(stored, { etag: '"8"' });
      }),
    );
    void useOrgSettings.getState().loadSettings();
    render(<ClientAddressCard />);
    const field = await screen.findByLabelText('Trusted proxies');
    expect(screen.getByText('X-Forwarded-For')).toBeInTheDocument();
    await userEvent.click(field);
    await userEvent.keyboard('10.0.0.0/8{Enter}');
    await userEvent.keyboard('{Escape}');
    await userEvent.click(screen.getByRole('button', { name: 'Save changes' }));
    await waitFor(() =>
      expect(patches).toEqual([{ spec: { client_ip: { trusted_proxies: ['10.0.0.0/8'] } } }]),
    );
    // Read back from the gateway: the saved range is shown, and nothing is left to save.
    expect(await screen.findByText('10.0.0.0/8')).toBeInTheDocument();
    await waitFor(() => expect(screen.getByRole('button', { name: 'Save changes' })).toBeDisabled());
    expect(useOrgSettings.getState().security.data?.spec).toMatchObject({
      client_ip: { trusted_proxies: ['10.0.0.0/8'] },
      secrets: { encryption_keys: ['${env:KEY}'] },
    });
  });

  it('refuses a trusted proxy that is not an address or range', async () => {
    const patches: unknown[] = [];
    server.use(
      http.get('*/admin/v1/settings/limits', empty),
      http.get('*/admin/v1/settings/security', empty),
      http.patch('*/admin/v1/settings/security', async ({ request }) => {
        patches.push(await request.json());
        return empty();
      }),
    );
    void useOrgSettings.getState().loadSettings();
    render(<ClientAddressCard />);
    await userEvent.click(await screen.findByLabelText('Trusted proxies'));
    await userEvent.keyboard('proxy.internal{Enter}{Escape}');
    await userEvent.click(screen.getByRole('button', { name: 'Save changes' }));
    expect(await screen.findByText(/isn't an IP address or range/)).toBeInTheDocument();
    expect(patches).toEqual([]);
  });
});
