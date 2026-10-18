#!/usr/bin/env bash
# Deletes every AWS resource of a benchmark run, found by its tags, then checks nothing is left.
#   teardown.sh <run id>     resources tagged RunId=<run id>
#   teardown.sh --all        every resource tagged Project=antirion-bench
# Order: instances (waits until terminated), then security groups, key pairs, placement groups and
# any volume left behind.
#
# Credentials: an AWS call that fails because the credentials expired (an `aws login` session ends)
# is retried every 15 s for up to CRED_WAIT_S seconds (default 900) while the owner runs `aws login`
# in another terminal; after that wait has run out once, later calls are tried once.
#
# Exit status: 0 only when every describe call answered and none found anything tagged; 1 when
# something tagged remains; 2 when the check itself could not be made (it then never says "nothing
# remains": it prints what the run's local state says may be left and the commands that remove it).
#
# Environment: AB_REGION (default: the run's region from its state, else us-east-1), AB_STATE_DIR
# (where run.sh keeps its runs' state; default bench/aws/.state).
set -uo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT=antirion-bench
STATE_ROOT="${AB_STATE_DIR:-$HERE/.state}"
CRED_WAIT_S="${CRED_WAIT_S:-900}"
WAIT_TEXT="$(( CRED_WAIT_S / 60 )) min"; [ "$CRED_WAIT_S" -lt 120 ] && WAIT_TEXT="$CRED_WAIT_S s"
export AWS_PAGER=""

case "${1:-}" in
  --all) KEY=Project; VALUE=$PROJECT; STATE_DIR="" ;;
  ""|-h|--help) echo "usage: teardown.sh <run id> | --all" >&2; exit 2 ;;
  *) KEY=RunId; VALUE="$1"; STATE_DIR="$STATE_ROOT/runs/$1" ;;
esac
RUN_REGION=""
[ -n "$STATE_DIR" ] && [ -f "$STATE_DIR/state" ] && RUN_REGION=$(sed -n 's/^REGION=//p' "$STATE_DIR/state" | tail -1)
REGION="${AB_REGION:-${RUN_REGION:-us-east-1}}"
FILTER="Name=tag:$KEY,Values=$VALUE"
ERR=$(mktemp)
# Flags kept in files: most calls run in command substitutions (subshells).
GAVE_UP="$ERR.gave-up"
WARNED="$ERR.warned"
trap 'rm -f "$ERR" "$GAVE_UP" "$WARNED"' EXIT
cred_error() {
  grep -qiE 'expired|reauthenticate|aws login|ExpiredToken|RequestExpired|InvalidClientTokenId|UnrecognizedClient|Unable to locate credentials|could not be refreshed|security token|AuthFailure|SignatureDoesNotMatch' "$ERR"
}
# The aws CLI in the run's region; stdout passes through. Waits for fresh credentials (see above).
aws_() {
  local start rc
  start=$(date +%s)
  while :; do
    rc=0
    aws --region "$REGION" "$@" 2> "$ERR" || rc=$?
    [ "$rc" = 0 ] && return 0
    if cred_error && [ ! -e "$GAVE_UP" ]; then
      if [ $(( $(date +%s) - start )) -ge "$CRED_WAIT_S" ]; then
        touch "$GAVE_UP"
        echo "teardown: the AWS credentials are still not valid after $WAIT_TEXT; giving up waiting" >&2
        grep -v '^ *$' "$ERR" | sed 's/^/  /' >&2
        return "$rc"
      fi
      if [ ! -e "$WARNED" ]; then
        touch "$WARNED"
        echo "teardown: the AWS credentials expired or are not valid. In another terminal run:" >&2
        echo "" >&2
        echo "    aws login" >&2
        echo "" >&2
        echo "teardown: retrying every 15 s for up to $WAIT_TEXT" >&2
      fi
      sleep 15
      continue
    fi
    # After giving up on the credentials, their error was shown once already.
    cred_error && [ -e "$GAVE_UP" ] && return "$rc"
    grep -v '^ *$' "$ERR" | sed 's/^/  /' >&2
    return "$rc"
  done
}

listing() { shift; aws_ "$@"; } # what command...

live_instances() {
  listing instances ec2 describe-instances --filters "$FILTER" \
    "Name=instance-state-name,Values=pending,running,stopping,stopped,shutting-down" \
    --query 'Reservations[].Instances[].InstanceId' --output text
}
security_groups() { listing security-groups ec2 describe-security-groups --filters "$FILTER" --query 'SecurityGroups[].GroupId' --output text; }
key_pairs() { listing key-pairs ec2 describe-key-pairs --filters "$FILTER" --query 'KeyPairs[].KeyPairId' --output text; }
placement_groups() { listing placement-groups ec2 describe-placement-groups --filters "$FILTER" --query 'PlacementGroups[].GroupName' --output text; }
volumes() { listing volumes ec2 describe-volumes --filters "$FILTER" "$@" --query 'Volumes[].VolumeId' --output text; }

state_get() { [ -n "$STATE_DIR" ] && [ -f "$STATE_DIR/state" ] && sed -n "s/^$1=//p" "$STATE_DIR/state" | tail -1; }

# What the run's local state says it created, and the commands that remove it.
leftover_help() { # instances sg keys pgs volumes (as found, or empty when not known)
  local i="$1" sg="$2" kp="$3" pg="$4" vol="$5" r="--region $REGION" dm
  if [ -n "$STATE_DIR" ] && [ -f "$STATE_DIR/state" ]; then
    [ -z "$i" ] && [ -s "$STATE_DIR/machines.tsv" ] && i=$(awk '{printf "%s ", $7}' "$STATE_DIR/machines.tsv")
    [ -z "$sg" ] && sg=$(state_get SG)
    [ -z "$kp" ] && kp=$(state_get KEY_NAME)
    [ -z "$pg" ] && pg=$(state_get PLACEMENT_GROUP)
    dm=$(state_get DEADMAN_AT)
  fi
  echo "What may remain of $KEY=$VALUE:" >&2
  echo "  instances:        ${i:-unknown}" >&2
  echo "  security group:   ${sg:-unknown}" >&2
  echo "  key pair:         ${kp:-unknown}" >&2
  echo "  placement group:  ${pg:-none known}" >&2
  [ -n "$vol" ] && echo "  volumes:          $vol" >&2
  [ -n "${dm:-}" ] && echo "  every instance terminates itself by $(date -u -r "$dm" +%Y-%m-%dT%H:%MZ) (dead-man switch); the rest costs nothing but must still be deleted" >&2
  echo "Remove it with:" >&2
  echo "    aws login" >&2
  echo "    AB_REGION=$REGION $HERE/teardown.sh $( [ "$KEY" = Project ] && echo --all || echo "$VALUE")" >&2
  echo "or by hand:" >&2
  [ -n "$i" ] && { echo "    aws $r ec2 terminate-instances --instance-ids $i" >&2; echo "    aws $r ec2 wait instance-terminated --instance-ids $i" >&2; }
  for x in $sg; do echo "    aws $r ec2 delete-security-group --group-id $x" >&2; done
  for x in $kp; do
    case "$x" in key-*) echo "    aws $r ec2 delete-key-pair --key-pair-id $x" >&2 ;; *) echo "    aws $r ec2 delete-key-pair --key-name $x" >&2 ;; esac
  done
  for x in $pg; do echo "    aws $r ec2 delete-placement-group --group-name $x" >&2; done
  for x in $vol; do echo "    aws $r ec2 delete-volume --volume-id $x" >&2; done
  echo "then check: aws $r ec2 describe-tags --filters Name=tag:$KEY,Values=$VALUE --output text" >&2
}

if ids=$(live_instances) && [ -n "$ids" ]; then
  echo "terminating instances: $ids"
  # shellcheck disable=SC2086
  aws_ ec2 terminate-instances --instance-ids $ids --query 'TerminatingInstances[].[InstanceId,CurrentState.Name]' --output text
  # shellcheck disable=SC2086
  aws_ ec2 wait instance-terminated --instance-ids $ids || true
fi
for _ in $(seq 1 30); do
  ids=$(live_instances) || break
  [ -z "$ids" ] && break
  sleep 10
done

if sgs=$(security_groups); then
  for sg in $sgs; do
    for attempt in $(seq 1 20); do
      if aws_ ec2 delete-security-group --group-id "$sg" 2>/dev/null; then
        echo "deleted security group $sg"; break
      fi
      [ "$attempt" = 20 ] && echo "cannot delete security group $sg yet" >&2
      sleep 6
    done
  done
fi

if kps=$(key_pairs); then
  for kp in $kps; do
    aws_ ec2 delete-key-pair --key-pair-id "$kp" && echo "deleted key pair $kp"
  done
fi

# A placement group can be deleted once no instance (not even a shutting-down one) is in it.
if pgs=$(placement_groups); then
  for pg in $pgs; do
    for attempt in $(seq 1 20); do
      if aws_ ec2 delete-placement-group --group-name "$pg" 2>/dev/null; then
        echo "deleted placement group $pg"; break
      fi
      [ "$attempt" = 20 ] && echo "cannot delete placement group $pg yet" >&2
      sleep 6
    done
  done
fi

if vols=$(volumes Name=status,Values=available); then
  for vol in $vols; do
    aws_ ec2 delete-volume --volume-id "$vol" && echo "deleted volume $vol"
  done
fi

# The verdict: every listing must answer.
LIST_FAILED=""
left_i=$(live_instances) || { left_i=""; LIST_FAILED="$LIST_FAILED instances"; }
left_sg=$(security_groups) || { left_sg=""; LIST_FAILED="$LIST_FAILED security-groups"; }
left_kp=$(key_pairs) || { left_kp=""; LIST_FAILED="$LIST_FAILED key-pairs"; }
left_pg=$(placement_groups) || { left_pg=""; LIST_FAILED="$LIST_FAILED placement-groups"; }
left_vol=$(volumes) || { left_vol=""; LIST_FAILED="$LIST_FAILED volumes"; }
if [ -n "$LIST_FAILED" ]; then
  echo "TEARDOWN NOT VERIFIED: could not list$LIST_FAILED (see the aws errors above); resources may remain." >&2
  leftover_help "$left_i" "$left_sg" "$left_kp" "$left_pg" "$left_vol"
  exit 2
fi
if [ -n "$left_i$left_sg$left_kp$left_pg$left_vol" ]; then
  echo "LEFT tagged $KEY=$VALUE: instances[$left_i] sg[$left_sg] keys[$left_kp] pg[$left_pg] volumes[$left_vol]" >&2
  leftover_help "$left_i" "$left_sg" "$left_kp" "$left_pg" "$left_vol"
  exit 1
fi
echo "nothing tagged $KEY=$VALUE remains (instances, security groups, key pairs, placement groups, volumes)"
