//! Revoked MCP sessions shared by a cluster's nodes, kept until their tokens expire. Each
//! revocation new to the store takes the next number of a sequence, and a log orders the
//! revocations by it, so a node reads only those after the last number it saw: the cost of a sync
//! follows the revocations, not the size of the keyspace. A second set holds each revocation's
//! expiry, for exact lookups by digest and for pruning.
//!
//! Each revocation is also written as `<prefix>mrev:<digest>`, the key nodes of the previous
//! version look up and scan for, and lookups fall back to it, so a cluster upgraded one node at a
//! time keeps its revocations.

use redis::Value;

use super::{ValkeyCounters, items, num, proto, store_err, text};
use crate::store::StoreError;

/// The most revocations one read returns.
const PAGE: usize = 1000;

/// KEYS: sequence, log (digest by number), expiries (digest by expiry), then the legacy key of each
/// revocation. ARGV: digest and expiry (Unix seconds) per revocation. Drops expired revocations,
/// adds the unexpired ones the store lacks, and keeps the sets as long as their last revocation.
/// Returns how many it added.
const SHARE: &str = r"
local now = tonumber(redis.call('TIME')[1])
local gone = redis.call('ZRANGEBYSCORE', KEYS[3], '-inf', now)
for i = 1, #gone, 500 do
  local part = { unpack(gone, i, math.min(i + 499, #gone)) }
  redis.call('ZREM', KEYS[3], unpack(part))
  redis.call('ZREM', KEYS[2], unpack(part))
end
local added = 0
for i = 1, #ARGV, 2 do
  local digest, exp = ARGV[i], tonumber(ARGV[i + 1])
  if exp > now then
    redis.call('SET', KEYS[3 + (i + 1) / 2], ARGV[i + 1], 'EXAT', exp, 'NX')
    if not redis.call('ZSCORE', KEYS[3], digest) then
      redis.call('ZADD', KEYS[2], redis.call('INCR', KEYS[1]), digest)
      redis.call('ZADD', KEYS[3], exp, digest)
      added = added + 1
    end
  end
end
local last = redis.call('ZRANGE', KEYS[3], -1, -1, 'WITHSCORES')
if last[2] then
  local until_s = math.floor(tonumber(last[2])) + 1
  redis.call('EXPIREAT', KEYS[2], until_s)
  redis.call('EXPIREAT', KEYS[3], until_s)
end
return added
";

/// KEYS: sequence, log, expiries. ARGV: the last number read, the most log rows to read. Returns
/// the number read up to, how many rows were read, then digest and expiry of each unexpired
/// revocation among them. A number past the sequence means the store lost the log: it is read
/// again from the start.
const READ: &str = r"
local seq = tonumber(redis.call('GET', KEYS[1]) or '0')
local from = tonumber(ARGV[1])
if from > seq then from = 0 end
local now = tonumber(redis.call('TIME')[1])
local rows = redis.call('ZRANGEBYSCORE', KEYS[2], '(' .. from, '+inf', 'WITHSCORES', 'LIMIT', 0, tonumber(ARGV[2]))
local out = { tostring(from), tostring(#rows / 2) }
for i = 1, #rows, 2 do
  out[1] = rows[i + 1]
  local exp = redis.call('ZSCORE', KEYS[3], rows[i])
  if exp and tonumber(exp) > now then
    out[#out + 1] = rows[i]
    out[#out + 1] = exp
  end
end
return out
";

/// A whole number of seconds or of the sequence, as the server writes scores.
fn whole(v: &Value) -> Option<i64> {
    #[allow(clippy::cast_possible_truncation)]
    match v {
        Value::Double(d) => Some(*d as i64),
        other => text(other)?.parse::<f64>().ok().map(|d| d as i64),
    }
}

impl ValkeyCounters {
    fn revocation_keys(&self) -> [String; 3] {
        [
            format!("{}mrevs:n", self.prefix),
            format!("{}mrevs:log", self.prefix),
            format!("{}mrevs:exp", self.prefix),
        ]
    }

    fn legacy_revocation_key(&self, digest: &str) -> String {
        format!("{}mrev:{digest}", self.prefix)
    }

    /// Records revoked MCP sessions (hex digest, expiry in Unix seconds) until they expire;
    /// returns how many the store did not hold yet.
    pub async fn share_revocations(&self, items: &[(String, i64)]) -> Result<usize, StoreError> {
        let mut added = 0;
        for chunk in items.chunks(PAGE) {
            let mut cmd = redis::cmd("EVAL");
            cmd.arg(SHARE).arg(3 + chunk.len()).arg(&self.revocation_keys());
            for (digest, _) in chunk {
                cmd.arg(self.legacy_revocation_key(digest));
            }
            for (digest, exp) in chunk {
                cmd.arg(digest).arg(exp.to_string());
            }
            added += usize::try_from(num(&self.run(&cmd).await?)?).unwrap_or(0);
        }
        Ok(added)
    }

    /// The unexpired revocations shared after number `after` of the sequence, and the number they
    /// reach: pass it back to read only newer ones (0 reads every revocation).
    pub async fn revocations_since(&self, after: u64) -> Result<(u64, Vec<(String, i64)>), StoreError> {
        let mut from = after;
        let mut out = Vec::new();
        loop {
            let mut cmd = redis::cmd("EVAL");
            cmd.arg(READ)
                .arg(3)
                .arg(&self.revocation_keys())
                .arg(from)
                .arg(PAGE);
            let reply = self.run(&cmd).await?;
            let [upto, read, pairs @ ..] = items(&reply) else {
                return Err(proto("revocation log"));
            };
            let upto = whole(upto).and_then(|n| u64::try_from(n).ok());
            let read = whole(read).and_then(|n| usize::try_from(n).ok());
            let (Some(upto), Some(read)) = (upto, read) else {
                return Err(proto("revocation log"));
            };
            for [digest, exp] in pairs.as_chunks::<2>().0 {
                if let (Some(digest), Some(exp)) = (text(digest), whole(exp)) {
                    out.push((digest, exp));
                }
            }
            if read < PAGE {
                return Ok((upto, out));
            }
            from = upto;
        }
    }

    /// Every unexpired revocation the store holds (hex digest, expiry in Unix seconds).
    pub async fn revocations(&self) -> Result<Vec<(String, i64)>, StoreError> {
        Ok(self.revocations_since(0).await?.1)
    }

    /// The expiry (Unix seconds) of one revoked MCP session, by hex digest, if it was revoked.
    pub async fn revocation(&self, digest: &str) -> Result<Option<i64>, StoreError> {
        let [_, _, expiries] = self.revocation_keys();
        let mut pipe = redis::pipe();
        pipe.cmd("ZSCORE")
            .arg(expiries)
            .arg(digest)
            .cmd("GET")
            .arg(self.legacy_revocation_key(digest));
        let mut conn = self.conn.clone();
        let (shared, legacy): (Value, Value) =
            pipe.query_async(&mut conn).await.map_err(|e| store_err(&e))?;
        Ok(whole(&shared).or_else(|| whole(&legacy)))
    }
}
