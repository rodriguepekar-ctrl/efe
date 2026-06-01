"""Lua scripts used against Redis.

Kept as plain strings so they can be loaded via ``SCRIPT LOAD`` and
replayed via ``EVALSHA`` — and so we can also replay them in an
in-memory lua interpreter for unit tests.

Both scripts return either a string ``"ok"`` on success or a Redis
error-like tuple ``{err, ...}`` on failure. The Python wrappers in
:mod:`lua_runner` translate both into typed results / exceptions.
"""

from __future__ import annotations

from typing import Final

ATOMIC_SWAP_LUA: Final[str] = r"""
-- atomic_swap.lua — CAS on (version, routing_hash) + PubSub notify.
-- KEYS[1] = "routing:{tenant}:{task}:v"
-- KEYS[2] = "routing:{tenant}:{task}"
-- KEYS[3] = pubsub channel, e.g. "routing.updated"
-- ARGV[1] = expected_version (str int)
-- ARGV[2] = new_version      (str int)
-- ARGV[3..N] = endpoint_id, "<weight>:<role>", ...

if #ARGV < 2 or (#ARGV - 2) % 2 ~= 0 then
    return {err = 'invalid_argc'}
end

local cur = tonumber(redis.call('GET', KEYS[1]) or '0')
local exp = tonumber(ARGV[1])
local nxt = tonumber(ARGV[2])

if cur ~= exp then
    return {err = 'version_mismatch', current = cur}
end
if nxt ~= cur + 1 then
    return {err = 'version_not_sequential'}
end

-- Validate sum of weights = 100 for champion + candidate roles.
local weight_sum = 0
for i = 3, #ARGV, 2 do
    local value = ARGV[i + 1]
    local sep = string.find(value, ':')
    if sep == nil then
        return {err = 'invalid_value_format', at = ARGV[i]}
    end
    local w = tonumber(string.sub(value, 1, sep - 1))
    local role = string.sub(value, sep + 1)
    if role == 'champion' or role == 'candidate' then
        weight_sum = weight_sum + w
    end
end
if weight_sum ~= 100 then
    return {err = 'weights_not_100', sum = weight_sum}
end

-- Atomic replace.
redis.call('DEL', KEYS[2])
for i = 3, #ARGV, 2 do
    redis.call('HSET', KEYS[2], ARGV[i], ARGV[i + 1])
end
redis.call('SET', KEYS[1], ARGV[2])

-- Notify subscribers.
local msg = string.format('{"tenant_key":"%s","version":%d}', KEYS[2], nxt)
redis.call('PUBLISH', KEYS[3], msg)

return 'ok'
"""


QUOTA_BUCKET_LUA: Final[str] = r"""
-- quota_bucket.lua — atomic token-bucket rate limit.
-- KEYS[1] = "quota:{tenant}:{api_key}"
-- ARGV[1] = now_ms
-- ARGV[2] = capacity
-- ARGV[3] = refill_per_sec
-- ARGV[4] = cost

local now     = tonumber(ARGV[1])
local cap     = tonumber(ARGV[2])
local refill  = tonumber(ARGV[3])
local cost    = tonumber(ARGV[4])

local tokens  = tonumber(redis.call('HGET', KEYS[1], 'tokens') or cap)
local last_ts = tonumber(redis.call('HGET', KEYS[1], 'last_ts') or now)

local elapsed = (now - last_ts) / 1000.0
if elapsed < 0 then elapsed = 0 end
local new_tokens = math.min(cap, tokens + elapsed * refill)

if new_tokens < cost then
    redis.call('HMSET', KEYS[1], 'tokens', new_tokens, 'last_ts', now)
    redis.call('EXPIRE', KEYS[1], 3600)
    return 0
end

redis.call('HMSET', KEYS[1],
            'tokens', new_tokens - cost,
            'last_ts', now)
redis.call('EXPIRE', KEYS[1], 3600)
return 1
"""


__all__ = ["ATOMIC_SWAP_LUA", "QUOTA_BUCKET_LUA"]
