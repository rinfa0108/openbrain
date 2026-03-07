import sys
import json


def send(obj):
    sys.stdout.write(json.dumps(obj))
    sys.stdout.write("\n")
    sys.stdout.flush()


def envelope_ok(payload):
    return {"ok": True, **payload}


def envelope_err(code, message, details=None):
    return {"ok": False, "error": {"code": code, "message": message, "details": details}}


def rpc_response(msg_id, result=None, error=None):
    if error is not None:
        return {"jsonrpc": "2.0", "id": msg_id, "error": error}
    return {"jsonrpc": "2.0", "id": msg_id, "result": result}


for raw_line in sys.stdin:
    raw_line = raw_line.strip()
    if not raw_line:
        continue
    try:
        req = json.loads(raw_line)
    except json.JSONDecodeError:
        continue

    if req.get("method") == "tools/call":
        params = req.get("params", {})
        name = params.get("name")
        args = params.get("arguments", {})

        if name == "openbrain.ping":
            send(rpc_response(req.get("id"), envelope_ok({"version": "0.1", "server_time": "now"})))
            continue

        if name == "openbrain.search.semantic":
            if args.get("scope") == "missing-provider":
                send(
                    rpc_response(
                        req.get("id"),
                        envelope_err("OB_NOT_FOUND", "no embeddings found for provider"),
                    )
                )
            else:
                send(
                    rpc_response(
                        req.get("id"),
                        envelope_ok(
                            {"matches": [{"ref": "r1", "kind": "claim", "score": 0.6, "updated_at": "2026-01-01T00:00:00Z"}]}
                        ),
                    )
                )
            continue

        if name == "openbrain.write":
            send(rpc_response(req.get("id"), envelope_ok({"results": [{"ref": "r1", "type": "claim", "status": "draft", "version": 1}]})))
            continue

        if name == "openbrain.memory.pack":
            send(rpc_response(req.get("id"), envelope_ok({"pack": {"scope": args.get("scope"), "canonical": [], "constraints": [], "relevant": [], "conflicts": [], "summary": "ok"}})))
            continue

    send(rpc_response(req.get("id"), envelope_err("OB_INVALID_REQUEST", f"unknown tool: {req.get('params',{}).get('name', '<none>')}")))

