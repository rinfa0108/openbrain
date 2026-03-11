export type Json = unknown;
export type ReadonlyJsonMap = { [key: string]: Json } | readonly Json[] | Json;

export interface ErrorEnvelope {
  code: string;
  message: string;
  details?: Json;
}

export interface EnvelopeOk<T> {
  ok: true;
}

export type EnvelopeOkData<T> = EnvelopeOk<T> & T;

export interface EnvelopeErr {
  ok: false;
  error: ErrorEnvelope;
}

export type Envelope<T> = EnvelopeOkData<T> | EnvelopeErr;

export interface MemoryObject {
  type?: string;
  id?: string;
  scope?: string;
  status?: string;
  spec_version?: string;
  tags?: string[];
  data?: Json;
  provenance?: Json;
}

export interface PutObjectsRequest {
  objects: MemoryObject[];
  actor?: string;
  idempotency_key?: string;
}

export interface PutResult {
  ref: string;
  type: string;
  status: string;
  version: number;
}

export interface PutObjectsResponse {
  results: PutResult[];
  replayed?: boolean;
  request_id?: string;
  accepted_count?: number;
  object_ids?: string[];
  receipt_hash?: string;
}

export interface ReadRequest {
  scope: string;
  refs: string[];
}

export interface GetObjectsResponse {
  objects: Array<{
    type: string;
    id: string;
    scope: string;
    status: string;
    spec_version: string;
    tags: string[];
    data: Json;
    provenance: Json;
    version: number;
    created_at: string;
    updated_at: string;
  }>;
}

export type SearchStructuredLimitOffset = number;

export interface SearchStructuredRequest {
  scope: string;
  where_expr?: string;
  limit?: SearchStructuredLimitOffset;
  offset?: SearchStructuredLimitOffset;
  order_by?: {
    field: string;
    direction: "Asc" | "Desc";
  };
}

export interface SearchItem {
  ref: string;
  type: string;
  status: string;
  updated_at: string;
  version: number;
}

export interface SearchStructuredResponse {
  results: SearchItem[];
}

export interface SearchSemanticRequest {
  scope: string;
  query: string;
  top_k?: number;
  model?: string;
  embedding_provider?: string;
  embedding_model?: string;
  embedding_kind?: string;
  filters?: string;
  types?: string[];
  status?: string[];
}

export interface SearchMatch {
  ref: string;
  kind: string;
  score: number;
  updated_at: string;
  snippet?: string;
}

export interface SearchSemanticResponse {
  matches: SearchMatch[];
}

export type EmbedTarget =
  | { text: string }
  | { ref: string };

export interface EmbedGenerateRequest {
  scope: string;
  target: EmbedTarget;
  model: string;
  dims?: number;
}

export interface EmbedGenerateResponse {
  embedding_id: string;
  object_id?: string;
  model: string;
  dims: number;
  checksum: string;
  reused: boolean;
}

export interface RerankRequest {
  scope: string;
  query: string;
  candidates?: {
    refs: string[];
  } | {
    candidates: Array<{
      ref: string;
      type: string;
      snippet: string;
    }>;
  };
  top_k?: number;
}

export interface RerankResponse {
  ranked_refs: string[];
  rationale_short?: Array<{
    ref: string;
    why: string;
  }>;
}

export interface PackPolicy {
  max_items?: number;
  include_types?: string[];
  include_status?: string[];
}

export interface MemoryPackRequest {
  scope: string;
  task_hint: string;
  query?: string;
  policy?: PackPolicy;
}

export interface MemoryPack {
  scope: string;
  canonical: string[];
  constraints: string[];
  relevant: string[];
  conflicts: string[];
  recent?: string[];
  summary: string;
  next_actions?: string[];
}

export interface MemoryPackResponse {
  pack: MemoryPack;
}

export interface PingResponse {
  version: string;
  server_time: string;
}

export interface OpenBrainToolRequestMap {
  "openbrain.ping": {};
  "openbrain.write": PutObjectsRequest;
  "openbrain.read": ReadRequest;
  "openbrain.search.structured": SearchStructuredRequest;
  "openbrain.embed.generate": EmbedGenerateRequest;
  "openbrain.search.semantic": SearchSemanticRequest;
  "openbrain.rerank": RerankRequest;
  "openbrain.memory.pack": MemoryPackRequest;
}

export interface OpenBrainToolResponseMap {
  "openbrain.ping": PingResponse;
  "openbrain.write": PutObjectsResponse;
  "openbrain.read": GetObjectsResponse;
  "openbrain.search.structured": SearchStructuredResponse;
  "openbrain.embed.generate": EmbedGenerateResponse;
  "openbrain.search.semantic": SearchSemanticResponse;
  "openbrain.rerank": RerankResponse;
  "openbrain.memory.pack": MemoryPackResponse;
}

export interface OpenBrainHttpResponseMap {
  "ping": PingResponse;
  "write": PutObjectsResponse;
  "read": GetObjectsResponse;
  "search.structured": SearchStructuredResponse;
  "embed.generate": EmbedGenerateResponse;
  "search.semantic": SearchSemanticResponse;
  "rerank": RerankResponse;
  "memory.pack": MemoryPackResponse;
}
