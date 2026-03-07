import { OpenBrainError } from "./error.js";
import {
  EmbedGenerateRequest,
  EmbedGenerateResponse,
  Envelope,
  GetObjectsResponse,
  MemoryPackRequest,
  MemoryPackResponse,
  OpenBrainHttpResponseMap,
  OpenBrainToolResponseMap,
  PingResponse,
  PutObjectsRequest,
  PutObjectsResponse,
  ReadRequest,
  RerankRequest,
  RerankResponse,
  SearchMatch,
  SearchSemanticRequest,
  SearchSemanticResponse,
  SearchStructuredRequest,
  SearchStructuredResponse,
} from "./types.js";

interface RequestOptions {
  timeoutMs?: number;
  signal?: AbortSignal;
}

export interface HttpClientOptions {
  baseUrl?: string;
  timeoutMs?: number;
  fetchImpl?: typeof fetch;
}

interface ApiErrorEnvelope {
  code: string;
  message: string;
  details?: unknown;
}

type HttpMethod = "GET" | "POST";

export interface HttpRequestResult {
  ok: boolean;
  status: number;
}

export class OpenBrainHttpClient {
  private readonly baseUrl: string;
  private readonly timeoutMs: number;
  private readonly fetchFn: typeof fetch;

  constructor(options?: HttpClientOptions) {
    this.baseUrl = (options?.baseUrl ?? "http://127.0.0.1:7981").replace(/\/$/, "");
    this.timeoutMs = options?.timeoutMs ?? 15_000;
    this.fetchFn = options?.fetchImpl ?? (globalThis.fetch as typeof fetch);
  }

  async ping(signal?: AbortSignal): Promise<PingResponse> {
    return this.call<"ping", {}, PingResponse>("/v1/ping", {}, { signal });
  }

  async write(
    req: PutObjectsRequest,
    options?: RequestOptions
  ): Promise<PutObjectsResponse> {
    return this.call<"write", PutObjectsRequest, PutObjectsResponse>("/v1/write", req, options);
  }

  async read(req: ReadRequest, options?: RequestOptions): Promise<GetObjectsResponse> {
    return this.call<"read", ReadRequest, GetObjectsResponse>("/v1/read", req, options);
  }

  async searchStructured(
    req: SearchStructuredRequest,
    options?: RequestOptions
  ): Promise<SearchStructuredResponse> {
    return this.call<"search.structured", SearchStructuredRequest, SearchStructuredResponse>(
      "/v1/search/structured",
      req,
      options
    );
  }

  async embedGenerate(
    req: EmbedGenerateRequest,
    options?: RequestOptions
  ): Promise<EmbedGenerateResponse> {
    return this.call<"embed.generate", EmbedGenerateRequest, EmbedGenerateResponse>(
      "/v1/embed/generate",
      req,
      options
    );
  }

  async searchSemantic(
    req: SearchSemanticRequest,
    options?: RequestOptions
  ): Promise<SearchSemanticResponse> {
    return this.call<"search.semantic", SearchSemanticRequest, SearchSemanticResponse>(
      "/v1/search/semantic",
      req,
      options
    );
  }

  async rerank(req: RerankRequest, options?: RequestOptions): Promise<RerankResponse> {
    return this.call<"rerank", RerankRequest, RerankResponse>("/v1/rerank", req, options);
  }

  async memoryPack(
    req: MemoryPackRequest,
    options?: RequestOptions
  ): Promise<MemoryPackResponse> {
    return this.call<"memory.pack", MemoryPackRequest, MemoryPackResponse>(
      "/v1/memory/pack",
      req,
      options
    );
  }

  private async request<T>(path: string, req: T, requestOptions?: RequestOptions): Promise<any> {
    const timeoutMs = requestOptions?.timeoutMs ?? this.timeoutMs;
    const url = `${this.baseUrl}${path}`;

    const timeoutController = new AbortController();
    const timeout = setTimeout(() => timeoutController.abort(), timeoutMs);

    const mergedSignal = requestOptions?.signal
      ? this.mergeSignals(requestOptions.signal, timeoutController.signal)
      : timeoutController.signal;

    try {
      const response = await this.fetchFn(url, {
        method: "POST" as HttpMethod,
        headers: { "content-type": "application/json" },
        body: JSON.stringify(req),
        signal: mergedSignal,
      });

      const rawText = await response.text();
      const envelope = rawText ? this.parseEnvelope(rawText) : null;

      if (!response.ok) {
        if (envelope && typeof envelope === "object" && "error" in envelope) {
          throw this.openBrainErrorFromEnvelope(
            (envelope as { error: ApiErrorEnvelope }).error,
            response.status
          );
        }
        throw new OpenBrainError(
          "HTTP_ERROR",
          `Request failed with status ${response.status}`,
          { status: response.status }
        );
      }

      if (!envelope || typeof envelope.ok !== "boolean") {
        throw new OpenBrainError("OB_INTERNAL", "invalid response format", {
          status: response.status,
        });
      }

      if (!envelope.ok && "error" in envelope) {
        throw this.openBrainErrorFromEnvelope(envelope.error, response.status);
      }

      return envelope;
    } finally {
      clearTimeout(timeout);
      timeoutController.abort();
    }
  }

  private async call<K extends keyof OpenBrainHttpResponseMap, Req, Resp>(
    path: string,
    req: Req,
    options?: RequestOptions
  ): Promise<Resp> {
    const envelope = await this.request<Req>(path, req, options);
    return (envelope as { ok: true } & OpenBrainHttpResponseMap[K]) as Resp;
  }

  private parseEnvelope(raw: string): Envelope<Record<string, unknown>> {
    try {
      return JSON.parse(raw) as Envelope<Record<string, unknown>>;
    } catch (_err) {
      throw new OpenBrainError("OB_INTERNAL", "invalid response JSON");
    }
  }

  private openBrainErrorFromEnvelope(
    payload: ApiErrorEnvelope,
    status?: number
  ): OpenBrainError {
    return new OpenBrainError(payload.code, payload.message, {
      status,
      details: payload.details,
    });
  }

  private mergeSignals(a: AbortSignal, b: AbortSignal): AbortSignal {
    const controller = new AbortController();

    const onAbort = () => controller.abort();
    if (a.aborted || b.aborted) controller.abort();
    a.addEventListener("abort", onAbort, { once: true });
    b.addEventListener("abort", onAbort, { once: true });

    return controller.signal;
  }
}

