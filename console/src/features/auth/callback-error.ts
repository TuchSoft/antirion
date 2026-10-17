/**
 * A failed single sign-on comes back to the sign-in page as `?error=<code>&message=<text>`. These
 * are the words the page shows for it: what happened, by code, then the gateway's own message
 * when it sent one.
 */
const BY_CODE: Record<string, string> = {
  access_denied: 'Your identity provider did not let you sign in.',
  forbidden: 'Your account is not allowed to sign in to this gateway.',
  unauthenticated: 'The answer from your identity provider could not be verified.',
  invalid_parameter: 'The sign-in expired or was already used.',
  invalid_state: 'The sign-in expired or was already used.',
  expired: 'The sign-in expired or was already used.',
  conflict: 'Your account could not be matched to a person on this gateway.',
  store_unavailable: 'The gateway could not finish signing you in.',
  internal: 'The gateway could not finish signing you in.',
};

const GENERAL = 'Single sign-on did not work.';
const NEXT_STEP = 'Try again, or sign in another way.';

/** The longest message shown from the address bar; anything longer is cut. */
const MAX_MESSAGE = 300;

export interface CallbackError {
  title: string;
  detail: string;
}

/** The error a failed sign-in left in the address, in plain words; null when there is none. */
export function callbackError(code: string | null, message: string | null): CallbackError | null {
  const key = code?.trim().toLowerCase() ?? '';
  const text = (message ?? '').replace(/\s+/g, ' ').trim();
  if (!key && !text) return null;
  const title = BY_CODE[key] ?? GENERAL;
  if (!text || text === title) return { title, detail: NEXT_STEP };
  const cut = text.length > MAX_MESSAGE ? `${text.slice(0, MAX_MESSAGE - 1).trimEnd()}…` : text;
  return { title, detail: /[.!?…]$/.test(cut) ? cut : `${cut}.` };
}
