/** Recovery policy for LINE connections. Never replay message sends. */
export type FailureKind = "auth" | "network" | "server" | "protocol";

export function failureKind(error: unknown): FailureKind {
  const e = error as {
    name?: string;
    message?: string;
    code?: unknown;
    data?: { code?: unknown };
  } | null;
  const message = [
    e?.name,
    e?.message,
    e?.code,
    e?.data?.code,
    typeof error === "string" ? error : "",
  ].join(" ");
  // Only explicit LINE authentication failures invalidate a saved token.
  // An HTTP 403, DNS failure, or a generic TypeError does not prove logout.
  if (
    /\b(NOT_AUTHORIZED(?:_DEVICE)?|V3_TOKEN_CLIENT_LOGGED_OUT|LOGGED_OUT|AUTHENTICATION_FAILED|AUTHENTICATION_ERROR)\b/
      .test(message)
  ) {
    return "auth";
  }
  if (/LINE_HTTP_(?:408|429|5\d\d)\b/.test(message)) return "server";
  if (
    /\b(?:TimeoutError|NetworkError|ConnectionRefused|ConnectionReset|ConnectionAborted|NotConnected)\b|fetch failed|failed to fetch|error sending request|dns|connection (?:reset|refused|closed)|network (?:unreachable|error)|timed? out|unexpected eof/i
      .test(message)
  ) {
    return "network";
  }
  return "protocol";
}

/** Bound each HTTP exchange, without retrying non-idempotent LINE operations. */
export function createLineFetch(
  timeoutMs = 25_000,
  fetcher = globalThis.fetch,
  sessionSignal?: AbortSignal,
) {
  return async (request: Request): Promise<Response> => {
    const signal = AbortSignal.any([
      request.signal,
      AbortSignal.timeout(timeoutMs),
      ...(sessionSignal ? [sessionSignal] : []),
    ]);
    const response = await fetcher(request, { signal });
    if (!response.ok) {
      await response.body?.cancel();
      throw new Error(`LINE_HTTP_${response.status}`);
    }
    return response;
  };
}

/** Retry session restoration only, with a bounded exponential delay. */
export async function restoreWithRetry<T>(
  restore: () => Promise<T>,
  reconnecting: (attempt: number) => void,
  sleep = (ms: number) =>
    new Promise<void>((resolve) => setTimeout(resolve, ms)),
): Promise<T> {
  for (let attempt = 0;; attempt++) {
    try {
      return await restore();
    } catch (error) {
      const kind = failureKind(error);
      if (attempt >= 3 || (kind !== "network" && kind !== "server")) {
        throw error;
      }
      reconnecting(attempt + 1);
      await sleep(1000 * 2 ** attempt);
    }
  }
}
