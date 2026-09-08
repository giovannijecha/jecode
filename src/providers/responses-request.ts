// Common Responses transport and assembly for API keys and account access.
import type { SendRequest } from "../types.ts";
import type { ResponsesBody, ResponsesSession } from "./responses-session.ts";
import type { OpenAIResponse } from "./openai-wire.ts";
import { postSse } from "./http.ts";
import { isRetryableGenerationFailure } from "./failure.ts";
import { assembleOpenAI, openAIStreamProgress, openAITerminalEvent } from "./openai-stream.ts";

export async function requestResponses(
  providerId: string,
  endpoint: string,
  headers: Record<string, string>,
  body: ResponsesBody,
  request: SendRequest,
  session?: ResponsesSession,
): Promise<OpenAIResponse> {
  const account = providerId === "openai-codex";
  const http = () => postSse(endpoint, account ? session?.httpHeaders(headers) ?? headers : headers,
    body, request.maxTokens, request.signal,
    request.onStatus, openAIStreamProgress, (error) => isRetryableGenerationFailure(providerId, error),
    { terminal: openAITerminalEvent, doneMarker: true,
      ...(account && session !== undefined ? { onHeaders: (value: Headers) => session.observeHttpHeaders(value) } : {}),
      onTransport: (event) => request.onTransport?.({ ...event, fallback: session !== undefined }) });
  const socketHeaders = account
    ? { ...headers, "openai-beta": "responses_websockets=2026-02-06" } : headers;
  const events = session === undefined ? await http() : session.events(endpoint, socketHeaders, body, request, http);
  const response = await assembleOpenAI(events, request.onStream, request.onStatus);
  session?.remember(body, response);
  return response;
}
