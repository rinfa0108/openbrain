const LOOPBACK_HOSTS = new Set(["127.0.0.1", "localhost", "::1"]);
const MAX_LIMIT = 200;
const STORAGE_KEY = "openbrain.viewer.connection.v1";

function el(id) {
  return document.getElementById(id);
}

function parseLimit(raw) {
  const n = Number(raw);
  if (!Number.isFinite(n) || n <= 0) {
    return 50;
  }
  return Math.min(MAX_LIMIT, Math.floor(n));
}

function safeJson(value, maxLen = 4000) {
  const text = JSON.stringify(value, null, 2);
  if (text.length <= maxLen) {
    return text;
  }
  return `${text.slice(0, maxLen)}\n... (truncated ${text.length - maxLen} chars)`;
}

function loadConn() {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) {
      return { baseUrl: "http://127.0.0.1:8080", token: "" };
    }
    const parsed = JSON.parse(raw);
    return {
      baseUrl: parsed.baseUrl || "http://127.0.0.1:8080",
      token: parsed.token || "",
    };
  } catch (_e) {
    return { baseUrl: "http://127.0.0.1:8080", token: "" };
  }
}

function saveConn(baseUrl, token) {
  localStorage.setItem(STORAGE_KEY, JSON.stringify({ baseUrl, token }));
}

function validateBaseUrl(baseUrl, allowNonLoopback) {
  let url;
  try {
    url = new URL(baseUrl);
  } catch (_e) {
    throw new Error("Base URL must be a valid absolute URL.");
  }
  if (!allowNonLoopback && !LOOPBACK_HOSTS.has(url.hostname)) {
    throw new Error("Non-loopback base URL blocked. Tick override checkbox to continue.");
  }
  return url.origin;
}

function showGlobalError(message) {
  const node = el("globalError");
  if (!message) {
    node.classList.add("hidden");
    node.textContent = "";
    return;
  }
  node.classList.remove("hidden");
  node.textContent = message;
}

function formatApiError(path, status, body) {
  if (!body || !body.error) {
    return `${path} failed (HTTP ${status}).`;
  }
  const code = body.error.code || "OB_UNKNOWN";
  const message = body.error.message || "request failed";
  const reason = body.error.details && body.error.details.reason_code;
  const rule = body.error.details && body.error.details.policy_rule_id;
  let out = `${path} failed (HTTP ${status})\n${code}: ${message}`;
  if (code === "OB_FORBIDDEN") {
    out += `\nDENIED: ${reason || "OB_POLICY_DENY"} (rule: ${rule || "unknown"})`;
  }
  if (body.error.details) {
    out += `\ndetails: ${safeJson(body.error.details, 1200)}`;
  }
  return out;
}

async function callApi(path, body) {
  const baseUrlRaw = el("baseUrl").value.trim();
  const token = el("token").value.trim();
  const allowNonLoopback = el("allowNonLoopback").checked;

  if (!token) {
    throw new Error("Token is required.");
  }

  const origin = validateBaseUrl(baseUrlRaw, allowNonLoopback);
  const url = `${origin}${path}`;

  const res = await fetch(url, {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      "Authorization": `Bearer ${token}`,
    },
    body: JSON.stringify(body),
  });

  let parsed = null;
  try {
    parsed = await res.json();
  } catch (_e) {
    throw new Error(`${path} failed (HTTP ${res.status}): non-JSON response.`);
  }

  if (!res.ok || !parsed.ok) {
    throw new Error(formatApiError(path, res.status, parsed));
  }

  return parsed;
}

function renderAudit(events) {
  const lines = ["timestamp | event_type | actor_id | object_id | version | summary"];
  for (const ev of events) {
    lines.push([
      ev.ts || "-",
      ev.event_type || "-",
      ev.actor_identity_id || "-",
      ev.object_id || "-",
      ev.object_version ?? "-",
      (ev.summary || "-").replace(/\s+/g, " ").trim(),
    ].join(" | "));
  }
  return lines.join("\n");
}

function setTabs() {
  const tabs = document.querySelectorAll(".tab");
  const contents = {
    object: el("tab-object"),
    key: el("tab-key"),
    actor: el("tab-actor"),
  };

  tabs.forEach((tab) => {
    tab.addEventListener("click", () => {
      tabs.forEach((t) => t.classList.remove("active"));
      tab.classList.add("active");
      Object.values(contents).forEach((node) => node.classList.remove("active"));
      const key = tab.getAttribute("data-tab");
      if (contents[key]) {
        contents[key].classList.add("active");
      }
    });
  });
}

async function onPing() {
  const statusNode = el("pingStatus");
  try {
    showGlobalError("");
    statusNode.textContent = "Checking...";
    const env = await callApi("/v1/ping", {});
    statusNode.textContent = `OK ${env.version || ""} @ ${env.server_time || ""}`;
  } catch (err) {
    statusNode.textContent = "Failed";
    showGlobalError(err.message);
  }
}

async function onWorkspace() {
  try {
    showGlobalError("");
    const env = await callApi("/v1/workspace/info", {});
    el("workspaceOut").textContent = safeJson({
      workspace_id: env.workspace_id,
      owner_identity_id: env.owner_identity_id,
      caller_identity_id: env.caller_identity_id,
      caller_role: env.caller_role,
    });
  } catch (err) {
    el("workspaceOut").textContent = "";
    showGlobalError(err.message);
  }
}

async function onAuditObject() {
  try {
    showGlobalError("");
    const env = await callApi("/v1/audit/object_timeline", {
      scope: el("auditObjectScope").value.trim(),
      object_id: el("auditObjectId").value.trim(),
      from: el("auditObjectFrom").value.trim() || null,
      to: el("auditObjectTo").value.trim() || null,
      limit: parseLimit(el("auditObjectLimit").value),
    });
    el("auditOut").textContent = `${renderAudit(env.events || [])}\nlimit=${env.limit} offset=${env.offset}`;
  } catch (err) {
    el("auditOut").textContent = "";
    showGlobalError(err.message);
  }
}

async function onAuditKey() {
  try {
    showGlobalError("");
    const env = await callApi("/v1/audit/memory_key_timeline", {
      scope: el("auditKeyScope").value.trim(),
      memory_key: el("auditKeyValue").value.trim(),
      from: el("auditKeyFrom").value.trim() || null,
      to: el("auditKeyTo").value.trim() || null,
      limit: parseLimit(el("auditKeyLimit").value),
    });
    el("auditOut").textContent = `${renderAudit(env.events || [])}\nlimit=${env.limit} offset=${env.offset}`;
  } catch (err) {
    el("auditOut").textContent = "";
    showGlobalError(err.message);
  }
}

async function onAuditActor() {
  try {
    showGlobalError("");
    const env = await callApi("/v1/audit/actor_activity", {
      scope: el("auditActorScope").value.trim(),
      actor_identity_id: el("auditActorId").value.trim(),
      from: el("auditActorFrom").value.trim() || null,
      to: el("auditActorTo").value.trim() || null,
      limit: parseLimit(el("auditActorLimit").value),
    });
    el("auditOut").textContent = `${renderAudit(env.events || [])}\nlimit=${env.limit} offset=${env.offset}`;
  } catch (err) {
    el("auditOut").textContent = "";
    showGlobalError(err.message);
  }
}

async function onRetention() {
  try {
    showGlobalError("");
    const scope = el("retentionScope").value.trim();

    const search = await callApi("/v1/search/structured", {
      scope,
      where_expr: 'type == "policy.retention"',
      order_by: { field: "updated_at", direction: "Desc" },
      limit: 25,
      offset: 0,
      include_states: ["accepted", "candidate", "scratch", "deprecated"],
      include_expired: true,
      include_conflicts: true,
    });

    if (!Array.isArray(search.results) || search.results.length === 0) {
      el("retentionOut").textContent = "No policy.retention object found for this scope.";
      return;
    }

    const item = search.results[0];
    const read = await callApi("/v1/read", {
      scope,
      refs: [item.ref],
      include_states: ["accepted", "candidate", "scratch", "deprecated"],
      include_expired: true,
      include_conflicts: true,
    });

    const obj = (read.objects && read.objects[0]) || null;
    if (!obj) {
      el("retentionOut").textContent = "Found policy reference but object details could not be loaded.";
      return;
    }

    el("retentionOut").textContent = safeJson({
      ref: item.ref,
      version: obj.version,
      lifecycle_state: obj.lifecycle_state,
      default_ttl_by_kind: obj.data && obj.data.default_ttl_by_kind ? obj.data.default_ttl_by_kind : {},
      max_ttl_by_kind: obj.data && obj.data.max_ttl_by_kind ? obj.data.max_ttl_by_kind : {},
      immutable_kinds: obj.data && obj.data.immutable_kinds ? obj.data.immutable_kinds : [],
    });
  } catch (err) {
    el("retentionOut").textContent = "";
    showGlobalError(err.message);
  }
}

async function onInspect() {
  try {
    showGlobalError("");
    const env = await callApi("/v1/read", {
      scope: el("inspectScope").value.trim(),
      refs: [el("inspectId").value.trim()],
      include_states: ["accepted", "candidate", "scratch", "deprecated"],
      include_expired: true,
      include_conflicts: true,
    });

    const obj = (env.objects && env.objects[0]) || null;
    if (!obj) {
      el("inspectMeta").textContent = "Object not found in requested scope.";
      el("inspectData").textContent = "";
      return;
    }

    el("inspectMeta").textContent = safeJson({
      id: obj.id,
      type: obj.type,
      scope: obj.scope,
      version: obj.version,
      created_at: obj.created_at,
      updated_at: obj.updated_at,
      lifecycle_state: obj.lifecycle_state,
      expires_at: obj.expires_at,
      memory_key: obj.memory_key,
      conflict_status: obj.conflict_status,
      resolved_by_object_id: obj.resolved_by_object_id,
      resolved_at: obj.resolved_at,
      resolution_note: obj.resolution_note,
    });

    el("inspectData").textContent = safeJson(obj.data || {}, 3000);
  } catch (err) {
    el("inspectMeta").textContent = "";
    el("inspectData").textContent = "";
    showGlobalError(err.message);
  }
}

function bindEvents() {
  el("saveConn").addEventListener("click", () => {
    try {
      const baseUrl = validateBaseUrl(el("baseUrl").value.trim(), el("allowNonLoopback").checked);
      saveConn(baseUrl, el("token").value.trim());
      showGlobalError("Connection settings saved.");
    } catch (err) {
      showGlobalError(err.message);
    }
  });

  el("pingBtn").addEventListener("click", onPing);
  el("workspaceBtn").addEventListener("click", onWorkspace);
  el("auditObjectBtn").addEventListener("click", onAuditObject);
  el("auditKeyBtn").addEventListener("click", onAuditKey);
  el("auditActorBtn").addEventListener("click", onAuditActor);
  el("retentionBtn").addEventListener("click", onRetention);
  el("inspectBtn").addEventListener("click", onInspect);
}

function init() {
  const conn = loadConn();
  el("baseUrl").value = conn.baseUrl;
  el("token").value = conn.token;
  setTabs();
  bindEvents();
}

init();
