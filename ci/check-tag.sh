#!/bin/sh
# Fails unless the tag is exactly X.Y.Z and equals the version in Cargo.toml (design
# §13, change foundation D28). Usage: ci/check-tag.sh <tag>
set -eu
tag=${1:?usage: ci/check-tag.sh <tag>}
if ! printf '%s\n' "$tag" | grep -Eqx '[0-9]+\.[0-9]+\.[0-9]+'; then
    echo "release tag '$tag' is not of the form X.Y.Z" >&2
    exit 1
fi
version=$(cargo metadata --locked --no-deps --format-version 1 \
    | sed -n 's/.*"name":"kohaku","version":"\([^"]*\)".*/\1/p')
if [ "$tag" != "$version" ]; then
    echo "release tag '$tag' does not equal the Cargo.toml version '$version'" >&2
    exit 1
fi
