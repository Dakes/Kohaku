#!/bin/sh
# Compose smoke test (design §13, change foundation D29): runs the shipped docker-compose.yml
# and Caddyfile unchanged against the image built from dist/amd64/kohaku with the release
# Dockerfile, Caddy on its local CA. Needs Docker Engine 27+, free ports 80 and 443.
set -eu

root=$(cd "$(dirname "$0")/.." && pwd)
work=$(mktemp -d)
project=kohaku-smoke
domain=kohaku.smoke.test
compose() { docker compose --project-name "$project" --project-directory "$work" -f "$work/docker-compose.yml" "$@"; }
fail() { echo "smoke: FAIL: $*" >&2; exit 1; }
cleanup() {
    compose down -v --remove-orphans >/dev/null 2>&1 || true
    # The containers wrote ./data and ./caddy as other users.
    docker run --rm -v "$work:/work" alpine:3 rm -rf /work/data /work/caddy >/dev/null 2>&1 || true
    rm -rf "$work"
}
trap cleanup EXIT

engine=$(docker version --format '{{.Server.Version}}')
[ "${engine%%.*}" -ge 27 ] || fail "Docker Engine $engine is older than 27"
[ -x "$root/dist/amd64/kohaku" ] || fail "dist/amd64/kohaku is missing (just smoke builds it)"

cp "$root/docker-compose.yml" "$root/Caddyfile" "$work/"
# The data directory as the README prepares it: owned by the image's uid, mode 0700.
mkdir "$work/data"
docker run --rm -v "$work:/work" alpine:3 sh -c 'chown 65532:65532 /work/data && chmod 700 /work/data'

# The image under test, tagged as the shipped docker-compose.yml names it.
secret=$(head -c 32 /dev/urandom | base64)
cat > "$work/.env" <<ENV
KOHAKU_DOMAIN=$domain
KOHAKU_SECRET=$secret
KOHAKU_SMTP_HOST=smtp.smoke.test
KOHAKU_SMTP_PORT=587
KOHAKU_SMTP_TLS=starttls
KOHAKU_SMTP_USERNAME=kohaku
KOHAKU_SMTP_PASSWORD='smoke test password'
KOHAKU_SMTP_FROM=kohaku@$domain
KOHAKU_CADDY_CI="local_certs
skip_install_trust"
ENV
chmod 600 "$work/.env"
compose config --quiet || fail "docker compose config"
image=$(compose config --images | grep kohaku)
docker build --quiet --platform linux/amd64 -t "$image" "$root" >/dev/null
entrypoint=$(docker inspect --format '{{json .Config.Entrypoint}}' "$image")
[ "$entrypoint" = "null" ] || fail "the image declares an ENTRYPOINT: $entrypoint"

# An unedited template never runs: compose refuses empty secrets.
cp "$root/.env.example" "$work/.env.template"
if docker compose --project-name "$project" --project-directory "$work" --env-file "$work/.env.template" \
    -f "$work/docker-compose.yml" config --quiet 2>"$work/err"; then
    fail "compose accepted the unedited .env.example"
fi
grep -q KOHAKU_SECRET "$work/err" || fail "the refusal does not name KOHAKU_SECRET"

wait_healthy() {
    for _ in $(seq 1 90); do
        state=$(docker inspect --format '{{.State.Health.Status}}' "$(compose ps -q kohaku)" 2>/dev/null || true)
        [ "$state" = healthy ] && return 0
        sleep 1
    done
    compose logs kohaku >&2
    fail "kohaku did not become healthy"
}

# Caddy comes from its registry; the kohaku image under test only exists locally.
compose pull --quiet caddy
compose up -d --pull never
wait_healthy

# Both containers run confined as docker-compose.yml says.
inspect() { docker inspect --format "$2" "$(compose ps -q "$1")"; }
[ "$(inspect kohaku '{{.Config.User}}')" = 65532 ] || fail "kohaku does not run as 65532"
for service in kohaku caddy; do
    [ "$(inspect $service '{{.HostConfig.ReadonlyRootfs}}')" = true ] || fail "$service root is writable"
    [ "$(inspect $service '{{json .HostConfig.CapDrop}}')" = '["ALL"]' ] || fail "$service keeps capabilities"
    [ "$(inspect $service '{{json .HostConfig.SecurityOpt}}')" = '["no-new-privileges:true"]' ] \
        || fail "$service may gain privileges"
    inspect $service '{{json .HostConfig.Tmpfs}}' | grep -q '"/tmp"' || fail "$service has no /tmp tmpfs"
done
[ "$(inspect kohaku '{{json .HostConfig.CapAdd}}')" = null ] || fail "kohaku adds capabilities"
[ "$(inspect caddy '{{json .HostConfig.CapAdd}}')" = '["CAP_NET_BIND_SERVICE"]' ] || fail "caddy capabilities"
[ "$(inspect kohaku '{{.HostConfig.Memory}}')" = $((640 * 1024 * 1024)) ] || fail "kohaku memory limit"
[ "$(inspect caddy '{{.HostConfig.Memory}}')" = $((256 * 1024 * 1024)) ] || fail "caddy memory limit"
[ "$(inspect kohaku '{{json .HostConfig.PortBindings}}')" = '{}' ] || fail "kohaku publishes a port"

compose cp caddy:/data/caddy/pki/authorities/local/root.crt "$work/root.crt"
curl_main() {
    curl --silent --show-error --noproxy '*' --cacert "$work/root.crt" \
        --resolve "$domain:443:127.0.0.1" "$@"
}
for _ in $(seq 1 30); do
    curl_main --output /dev/null "https://$domain/" 2>/dev/null && break
    sleep 1
done
curl_main --dump-header "$work/headers" --output "$work/landing" "https://$domain/"
grep -q '^HTTP/[0-9.]* 200' "$work/headers" || fail "landing page status"
grep -qi "^content-security-policy: default-src 'none'" "$work/headers" || fail "no CSP"
grep -qi '^strict-transport-security: max-age=31536000' "$work/headers" || fail "no HSTS"
grep -q '<h1>Kohaku</h1>' "$work/landing" || fail "not the landing page"
ask=$(curl_main --output /dev/null --write-out '%{http_code}' "https://$domain/.well-known/kohaku/tls-ask?domain=$domain")
[ "$ask" = 404 ] || fail "tls-ask reachable through Caddy ($ask)"
if curl --silent --noproxy '*' --cacert "$work/root.crt" --resolve "unknown.smoke.test:443:127.0.0.1" \
    --output /dev/null "https://unknown.smoke.test/"; then
    fail "TLS handshake succeeded for a name Kohaku does not serve"
fi

# The admin area answers through Caddy: the sign-in form, and a redirect to it.
curl_main --dump-header "$work/login.headers" --output "$work/login" "https://$domain/admin/login"
grep -q '^HTTP/[0-9.]* 200' "$work/login.headers" || fail "sign-in page status"
grep -q 'action="/admin/login"' "$work/login" || fail "not the sign-in page"
! grep -qi '^set-cookie' "$work/login.headers" || fail "the sign-in page sets a cookie"
admin=$(curl_main --output /dev/null --write-out '%{http_code} %{redirect_url}' "https://$domain/admin")
[ "$admin" = "303 https://$domain/admin/login" ] || fail "/admin without a session: $admin"

# The account commands run beside serve and refuse an unknown address with status 1.
for command in unlock reset-password; do
    if compose exec -T kohaku kohaku admin "$command" --email nobody@smoke.test \
        > "$work/admin.out" 2> "$work/admin.err"; then
        fail "admin $command accepted an unknown address"
    fi
    grep -q 'no account has that email address' "$work/admin.err" || fail "admin $command: $(cat "$work/admin.err")"
    [ ! -s "$work/admin.out" ] || fail "admin $command wrote to stdout"
done

# Forged X-Forwarded-For through Caddy: every request still counts against the client's
# own read budget (300 per minute), so a burst of 400 ends in 429s.
: > "$work/burst.conf"
for i in $(seq 1 400); do
    # `next` resets every option, so each transfer carries its own.
    [ "$i" -gt 1 ] && echo next >> "$work/burst.conf"
    cat >> "$work/burst.conf" <<CONF
url = "https://$domain/"
header = "X-Forwarded-For: 198.51.$((i / 250)).$((i % 250))"
noproxy = "*"
cacert = "$work/root.crt"
resolve = "$domain:443:127.0.0.1"
silent
output = "/dev/null"
write-out = "%{http_code}\\n"
CONF
done
curl --config "$work/burst.conf" > "$work/burst.codes"
admitted=$(grep -c '^200$' "$work/burst.codes" || true)
limited=$(grep -c '^429$' "$work/burst.codes" || true)
[ "$limited" -gt 0 ] && [ "$admitted" -lt 330 ] || fail "forged X-Forwarded-For escaped the rate limit ($admitted admitted)"
[ "$(tail -n 1 "$work/burst.codes")" = 429 ] || fail "the last request of the burst was not rate limited"

# Backup through exec, restore through run, as the README does.
compose exec -T kohaku kohaku backup - > "$work/backup.db"
head -c 16 "$work/backup.db" | grep -q 'SQLite format 3' || fail "backup is not a SQLite database"
compose stop kohaku
compose run --rm -T kohaku kohaku restore - < "$work/backup.db" || fail "restore"
compose start kohaku
wait_healthy
curl_main --output /dev/null --fail "https://$domain/" || fail "landing page after restore"

# A template with only its secrets filled in: kohaku refuses the example values. The
# main stack goes first: both use the same fixed subnets.
compose down -v >/dev/null 2>&1
docker run --rm -v "$work:/work" alpine:3 sh -c 'rm -rf /work/data/* /work/data/.[!.]*' >/dev/null
template=kohaku-smoke-template
sed -e "s|^KOHAKU_SECRET=.*|KOHAKU_SECRET=$secret|" -e "s|^KOHAKU_SMTP_PASSWORD=.*|KOHAKU_SMTP_PASSWORD='x'|" \
    "$root/.env.example" > "$work/.env.secrets-only"
template_compose() {
    docker compose --project-name "$template" --project-directory "$work" \
        --env-file "$work/.env.secrets-only" -f "$work/docker-compose.yml" "$@"
}
template_compose up -d --pull never kohaku
sleep 5
template_state=$(docker inspect --format '{{.State.Health.Status}}' "$(template_compose ps -aq kohaku)")
template_compose logs kohaku > "$work/template.log" 2>&1
template_compose down -v >/dev/null 2>&1
[ "$template_state" != healthy ] || fail "kohaku ran with the example values"
grep -q 'still has the example value' "$work/template.log" || fail "the refusal names no example value"

echo "smoke: OK"
