// Export only known transport codes, never error messages, hosts, or addresses.
const NETWORK_CODES = [
  "ENOTFOUND", "EAI_AGAIN", "ETIMEDOUT", "ECONNRESET", "ECONNREFUSED",
  "ENETUNREACH", "EHOSTUNREACH", "EPIPE", "UND_ERR_CONNECT_TIMEOUT",
  "UND_ERR_HEADERS_TIMEOUT", "UND_ERR_BODY_TIMEOUT", "UND_ERR_SOCKET",
  "ERR_TLS_CERT_ALTNAME_INVALID", "CERT_HAS_EXPIRED", "DEPTH_ZERO_SELF_SIGNED_CERT",
  "SELF_SIGNED_CERT_IN_CHAIN", "UNABLE_TO_VERIFY_LEAF_SIGNATURE",
  "UNABLE_TO_GET_ISSUER_CERT_LOCALLY",
] as const;

export type NetworkCode = typeof NETWORK_CODES[number];

export function isNetworkCode(value: unknown): value is NetworkCode {
  return typeof value === "string" && NETWORK_CODES.some(code => code === value);
}

/** fetch and provider normalization wrap the useful code in an error cause. */
export function networkErrorCode(error: unknown): NetworkCode | undefined {
  const pending: unknown[] = [error];
  const seen = new Set<Error>();
  for (let index = 0; index < pending.length && index < 8; index++) {
    const current = pending[index];
    if (!(current instanceof Error) || seen.has(current)) continue;
    seen.add(current);
    const code = (current as NodeJS.ErrnoException).code;
    if (isNetworkCode(code)) return code;
    if (pending.length < 8 && current.cause !== undefined) pending.push(current.cause);
    if (current instanceof AggregateError && Array.isArray(current.errors)) {
      pending.push(...current.errors.slice(0, 8 - pending.length));
    }
  }
  return undefined;
}
