/**
 * Client-side operation identity allocation (ADR 0019).
 *
 * Operation IDs are opaque correlation identities owned by the Desktop:
 * `op_` + 32 lowercase hex characters (128 bits from the platform CSPRNG).
 * They are unique across renderer restarts with no persistent state, and
 * they carry no semantic payload — the command `kind` names the operation.
 *
 * Retry rule: a retry of a command whose delivery is unconfirmed must reuse
 * the original operation ID. Never mint a fresh identity for the same
 * logical operation, or Core's deduplication cannot recognize it.
 */

/** Canonical identifier shape shared with `altior-domain` (ADR 0004). */
export const DOMAIN_ID_PATTERN = (prefix: string): RegExp =>
  new RegExp(`^${prefix}_[0-9a-z]{16,64}$`);

/** True if `value` is a contract-valid operation identifier. */
export function isValidOperationId(value: string): boolean {
  return DOMAIN_ID_PATTERN("op").test(value);
}

function randomHex32(): string {
  return crypto.randomUUID().replaceAll("-", "");
}

/**
 * Mints a fresh operation identifier. Tests may inject a deterministic
 * hex generator; production uses the platform CSPRNG.
 */
export function createOperationId(
  randomHex: () => string = randomHex32,
): string {
  const body = randomHex();
  if (!/^[0-9a-f]{32}$/.test(body)) {
    throw new Error(
      `operation id generator produced an invalid body: ${body.slice(0, 8)}…`,
    );
  }
  return `op_${body}`;
}
