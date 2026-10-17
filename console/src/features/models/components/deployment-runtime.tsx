'use client';

import { useEffect, useState } from 'react';

import { Dot } from '@/components/ui/badge';
import { ConfirmDialog } from '@/components/ui/confirm';
import { DatePicker } from '@/components/ui/date-picker';
import { Field, Select } from '@/components/ui/form';
import { useCan } from '@/components/ui/gate';
import { Tooltip } from '@/components/ui/tooltip';
import { describeError, isChangeControl } from '@/lib/api/client';
import { auditActorBy } from '@/lib/audit-words';
import type { DeploymentHealth } from '@/lib/api/types';
import { formatDateTime, formatRelative } from '@/lib/time';
import { usePreferences } from '@/stores/preferences';
import { notify } from '@/stores/ui';

import { deploymentRuntimeApi, runtimeStatus, useDeploymentDisables } from '../deployment-state';
import { healthName } from '../types';

/** Reads who turned deployments off once a list shows one turned off (needs the audit log). */
export function useDisablesFor(health: DeploymentHealth[] | null | undefined): void {
  const canAudit = useCan('listAudit');
  const load = useDeploymentDisables((s) => s.load);
  const off = (health ?? [])
    .filter((h) => h.state === 'disabled')
    .map(healthName)
    .join(',');
  useEffect(() => {
    if (canAudit && off) void load();
  }, [canAudit, off, load]);
}

/** "Turned off by ada 5m ago", or "Turned off" when the audit log can't say. */
export function TurnedOffBy({ name }: { name: string }) {
  const entry = useDeploymentDisables((s) => s.byName[name]);
  const timeZone = usePreferences((s) => s.timeZone);
  if (!entry) return <>Turned off</>;
  return (
    <>
      Turned off by {auditActorBy(entry.actor)}{' '}
      <Tooltip content={formatDateTime(entry.time, timeZone)}>
        <span className="text-muted">{formatRelative(entry.time)}</span>
      </Tooltip>
    </>
  );
}

/** A deployment's runtime state with a dot, who turned it off, and when it comes back by itself. */
export function RuntimeState({
  name,
  health,
  withUntil = true,
}: {
  name: string;
  health: DeploymentHealth | undefined;
  /** Says when it comes back by itself (off for a table with its own column). */
  withUntil?: boolean;
}) {
  const timeZone = usePreferences((s) => s.timeZone);
  const status = runtimeStatus(health);
  return (
    <span className="flex flex-wrap items-center gap-x-1.5">
      <Dot tone={status.tone} />
      {status.kind === 'off' ? (
        <TurnedOffBy name={name} />
      ) : (
        <span className={status.kind === 'unknown' ? 'text-muted' : undefined}>{status.label}</span>
      )}
      {withUntil && status.until ? (
        <Tooltip content={formatDateTime(status.until, timeZone)}>
          <span className="text-caption text-muted">
            {status.kind === 'off' ? 'until' : 'tried again'} {formatRelative(status.until)}
          </span>
        </Tooltip>
      ) : null}
    </span>
  );
}

const UNTIL = [
  { value: '', label: 'Until someone turns it on' },
  { value: '1', label: 'For 1 hour' },
  { value: '4', label: 'For 4 hours' },
  { value: '24', label: 'For a day' },
  { value: 'date', label: 'Until a date' },
];

/** The end of a timed disable from the dialog's choice, or undefined to keep it off until turned on. */
export function untilOf(choice: string, date: string, now = Date.now()): string | undefined {
  if (choice === '') return undefined;
  if (choice === 'date') return date ? new Date(`${date}T00:00:00`).toISOString() : undefined;
  return new Date(now + Number(choice) * 3_600_000).toISOString();
}

/**
 * Turns a deployment off by hand: no request goes to it until someone turns it on (or the time
 * chosen passes), on every node and after restarts. Models move on to their other deployments.
 */
export function TurnOffDialog({
  name,
  open,
  onClose,
  onDone,
}: {
  name: string;
  open: boolean;
  onClose: () => void;
  onDone: () => void | Promise<void>;
}) {
  const [choice, setChoice] = useState('');
  const [date, setDate] = useState('');
  const [tomorrow] = useState(() => new Date(Date.now() + 86_400_000).toISOString().slice(0, 10));
  return (
    <ConfirmDialog
      open={open}
      onClose={() => {
        setChoice('');
        setDate('');
        onClose();
      }}
      title={`Turn off ${name}?`}
      objectName={name}
      confirmLabel="Turn off deployment"
      blockedReason={choice === 'date' && !date ? 'Choose the date it comes back on' : undefined}
      effect="No request goes to it, on every node and after restarts. Models that use it move on to their other deployments, or fail when it is their only one."
      onConfirm={async (reason) => {
        await deploymentRuntimeApi.disable(name, untilOf(choice, date), reason);
        notify('success', 'Deployment turned off', name);
        await onDone();
      }}
    >
      <Field label="How long">
        <Select value={choice} onChange={setChoice} options={UNTIL} />
      </Field>
      {choice === 'date' ? (
        <Field label="Comes back on">
          <DatePicker value={date} onChange={setDate} min={tomorrow} />
        </Field>
      ) : null}
    </ConfirmDialog>
  );
}

async function quick(
  run: () => Promise<unknown>,
  done: string,
  failed: string,
  name: string,
  onDone: () => void | Promise<void>,
): Promise<void> {
  try {
    await run();
    notify('success', done, name);
  } catch (e) {
    // Change control says so itself (a change request offered or opened).
    if (!isChangeControl(e)) notify('error', failed, describeError(e));
  }
  await onDone();
}

/** Turns a manually disabled deployment back on. */
export function turnOn(name: string, onDone: () => void | Promise<void>): Promise<void> {
  return quick(
    () => deploymentRuntimeApi.enable(name),
    'Deployment turned on',
    `Couldn’t turn on ${name}`,
    name,
    onDone,
  );
}

/** Clears a breaker or cooldown pause; a manual disable stays. */
export function resumeNow(name: string, onDone: () => void | Promise<void>): Promise<void> {
  return quick(
    () => deploymentRuntimeApi.reset(name),
    'Deployment resumed',
    `Couldn’t resume ${name}`,
    name,
    onDone,
  );
}
