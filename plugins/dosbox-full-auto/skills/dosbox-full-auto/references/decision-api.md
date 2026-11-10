# Decision API (decision.vllm-sr.ai) — reference for the merged skill

Key facts from `decision-vllm-skill`; load that installed skill for the full spec.

## Endpoints

- `POST /v1/systemone` (alias `/v1/decisions`) — ask questions. Key required.
  `/v1/decisions` answers per-question; `/v1/systemone` on `vllm-sr/d3-ultra`
  fails the whole request on any failed question (422) — use `/v1/decisions`
  when questions may fail individually.
- `GET /v1/systemone/models`, `/api/info`, `/api/status`, `/api/stats` — no key.
- Key self-service (key required): `GET /api/keys/self`,
  `GET /api/keys/self/usage`, `DELETE /api/keys/self`, `POST /api/keys/self/forget`.

## Auth

Key format `sk-vsr-` + 43 chars. `Authorization: Bearer *** or `X-API-Key`.
Key shown once, only hash stored. Use `${VLLM_API_KEY}`; never hardcode, echo,
or store key text. Verify liveness with `GET /api/keys/self` (status field).

Key creation (PoW) exists server-side. **Rate limits strictly enforced:**
5 keys/IP/hour, 20/day. Creation requests respect 12 min cooldown and daily limit
to avoid 429 key_creation_limited. Keys verified on creation via GET /api/keys/self.

## Request body

`{model, state, questions, images?, videos?, states?, options?}`. Ask ALL
questions about one input in ONE request — media is processed once and counts
as one request. Images: base64 data URLs. `options` accepts only
`return_meta` / `deadline_ms`.

Question types: `choice` (criteria = >=2 options -> `choice` + probabilities),
`noul` (yes/no -> P(yes)), `score` (2-10 ordered levels), `box` (edges as
fractions + confidence — instructions are a noun phrase, e.g. "the whole word
User, all four letters", never an imperative).

## Model

Only `vllm-sr/d3-ultra` (27B), asked by name. Turbo names are aliases answered
by ultra. Box: one request in flight per key+model; pair a `noul` question
when testing a possibly-absent target (box always returns a box).

## Limits

Live values from `GET /api/info`. 35 s per request, 429/503/504 obey
Retry-After exactly with backoff + jitter; timed-out requests still burn
budget. `X-RateLimit-*` headers on every response. Never parallel-fire
beyond a few. Everything may be logged/trained on — no sensitive data.
