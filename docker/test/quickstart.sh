#!/bin/sh
# The quick start of an image: with no config file and only its two ports published, the first start
# prints an admin token in the log, the console answers at http://localhost:<admin port>/ui/, the
# token signs in to the Admin API, and a restart keeps the token without printing it again.
#
#   docker/test/quickstart.sh <image> [data port] [admin port]    # host ports, 8080 and 8081
set -eu

image=${1:?usage: docker/test/quickstart.sh <image> [data port] [admin port]}
data_port=${2:-8080}
admin_port=${3:-8081}
name="antirion-quickstart-$$"
admin="http://localhost:$admin_port"

cleanup() {
  docker rm -f -v "$name" >/dev/null 2>&1 || true
}
trap cleanup EXIT

fail() {
  echo "quickstart: $*" >&2
  docker logs "$name" >&2 || true
  exit 1
}

ready() {
  for _ in $(seq 1 60); do
    curl -sf "http://localhost:$data_port/-/readyz" >/dev/null 2>&1 && return 0
    sleep 1
  done
  fail "the gateway did not become ready"
}

docker run -d --name "$name" -p "127.0.0.1:$data_port:8080" -p "127.0.0.1:$admin_port:8081" "$image" >/dev/null
ready

token=$(docker logs "$name" 2>/dev/null | sed -n 's/^  Admin token  \([^ ]*\)$/\1/p' | head -n 1)
[ -n "$token" ] || fail "the first start printed no admin token"
docker logs "$name" 2>/dev/null | grep -q "Console      $admin/ui/" || fail "the log does not name the console at $admin/ui/"

status=$(curl -s -o /dev/null -w '%{http_code}' "$admin/ui/")
[ "$status" = 200 ] || fail "the console answered $status"

headers=$(curl -s -o /dev/null -D - -X POST "$admin/admin/v1/auth/session" \
  -H 'Content-Type: application/json' -H "Origin: $admin" -d "{\"token\": \"$token\"}")
cookie=$(printf '%s\n' "$headers" | sed -n 's/^[Ss]et-[Cc]ookie: antirion_session=\([^;]*\);.*/\1/p')
[ -n "$cookie" ] || fail "the token did not start a console session: $headers"
curl -sf "$admin/admin/v1/auth/me" -H "Cookie: antirion_session=$cookie" | grep -q '"type":"bootstrap"' \
  || fail "the session is not the bootstrap admin's"
curl -sf "$admin/admin/v1/settings/store" -H "Authorization: Bearer $token" | grep -q '/var/lib/antirion/data' \
  || fail "the store is not in /var/lib/antirion/data"

docker restart "$name" >/dev/null
ready
[ "$(docker logs "$name" 2>&1 | grep -c -- "$token")" = 1 ] || fail "the token was printed again after a restart"
curl -sf -o /dev/null "$admin/admin/v1/auth/me" -H "Authorization: Bearer $token" \
  || fail "the token changed across a restart"

echo "quickstart test passed: $image"
