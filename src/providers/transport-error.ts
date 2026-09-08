// Fixed transport diagnostics survive provider normalization without retaining
// peer close reasons, frame contents, endpoints, or native error messages.
const FAILURES = ["connection", "closed", "idle-timeout", "progress-timeout",
  "not-ready", "event-limit", "queue-limit", "stream-limit", "invalid-json"] as const;
export type TransportFailure = typeof FAILURES[number];
export type TransportFailureDiagnostic = {
  transportFailure?: TransportFailure;
  webSocketCloseCode?: number;
};

export function isTransportFailure(value: unknown): value is TransportFailure {
  return typeof value === "string" && FAILURES.some(failure => failure === value);
}

export class TransportError extends Error {
  readonly failure: TransportFailure;
  readonly webSocketCloseCode: number | undefined;
  readonly kind: "network" | "timeout" | "unknown";

  constructor(failure: TransportFailure, message: string, options: ErrorOptions & { closeCode?: number } = {}) {
    super(message, options);
    this.name = "TransportError";
    this.failure = failure;
    this.webSocketCloseCode = options.closeCode;
    this.kind = failure === "idle-timeout" || failure === "progress-timeout" ? "timeout"
      : failure === "connection" || failure === "closed" ? "network" : "unknown";
  }
}

export function transportFailureDiagnostic(error: unknown): TransportFailureDiagnostic {
  for (let depth = 0; depth < 8 && error instanceof Error; depth++, error = error.cause) {
    if (error instanceof TransportError) {
      return { transportFailure: error.failure,
        ...(error.webSocketCloseCode === undefined ? {} : { webSocketCloseCode: error.webSocketCloseCode }) };
    }
  }
  return {};
}
