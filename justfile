# Kohaku development tasks. `just --list` shows them.

# Dev server on http://localhost:8080: restarts on changes, pages reload, mail is printed
dev:
    #!/bin/sh
    set -eu
    # The dev database's keycheck needs the same secret on every start.
    if [ ! -f .env.dev ]; then
        (umask 077; printf 'KOHAKU_SECRET=%s\n' "$(head -c 32 /dev/urandom | base64)" > .env.dev)
    fi
    mkdir -p data
    set -a
    . ./.env.dev
    set +a
    export KOHAKU_BASE_URL=http://localhost:8080
    export KOHAKU_TRUSTED_PROXIES=none
    export KOHAKU_SMTP_FROM=kohaku@localhost
    exec watchexec --restart --watch src --watch templates --watch static --watch migrations \
        --watch Cargo.toml -- cargo run --features dev -- serve

# Build the image `dakes/kohaku:0` (the tag docker-compose.yml runs) for amd64 or arm64
image arch="amd64":
    #!/bin/sh
    set -eu
    case "{{arch}}" in
        amd64) target=x86_64-unknown-linux-musl ;;
        arm64) target=aarch64-unknown-linux-musl ;;
        *) echo "arch must be amd64 or arm64" >&2; exit 2 ;;
    esac
    cargo zigbuild --locked --release --target "$target"
    mkdir -p "dist/{{arch}}"
    cp "target/$target/release/kohaku" "dist/{{arch}}/kohaku"
    docker buildx build --platform "linux/{{arch}}" --tag dakes/kohaku:0 --load .

# Compose stack at https://localhost:8443 with the local image; creates .env and ./data once
local: image
    #!/bin/sh
    set -eu
    if [ ! -f .env ]; then
        secret=$(head -c 32 /dev/urandom | base64)
        (umask 077; sed "s|^KOHAKU_SECRET=$|KOHAKU_SECRET=$secret|" .env.local.example > .env)
    fi
    if [ ! -d data ]; then
        mkdir data
        # From inside a container, so uid 65532 is the container's, also under rootless Docker.
        docker run --rm -v "$PWD/data:/data" alpine:3 sh -c 'chown 65532:65532 /data && chmod 700 /data'
    fi
    # Pulls Caddy if missing; the kohaku image was just built, so it is never pulled.
    docker compose up -d --pull missing
    echo "Starting: https://localhost:8443 (accept Caddy's local certificate). Stop with: docker compose down"

# Build the amd64 image and run the Compose smoke test (needs ports 80 and 443 free)
smoke: (image "amd64")
    ci/smoke.sh
