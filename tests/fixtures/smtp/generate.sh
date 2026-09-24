#!/bin/sh
# Test certificates for tests/smtp_tls.rs (change foundation D18). Run once with
# OpenSSL 3 from this directory; the output is committed, so no dev shell needs
# openssl. Trusted only through SmtpRoots::for_tests, which src/ never calls.
set -eu
cd "$(dirname "$0")"
far=20991231235959Z

ec_key() { openssl genpkey -algorithm EC -pkeyopt ec_paramgen_curve:P-256 -out "$1" 2>/dev/null; }

# Test CA.
ec_key ca.key
openssl req -x509 -new -key ca.key -subj "/CN=Kohaku test CA" -sha256 \
    -not_before 20250101000000Z -not_after "$far" \
    -addext "basicConstraints=critical,CA:TRUE" \
    -addext "keyUsage=critical,keyCertSign,cRLSign" -out ca.pem

# leaf NAME HOST NOT_BEFORE NOT_AFTER: a server certificate signed by the test CA.
leaf() {
    ec_key "$1.key"
    openssl req -new -key "$1.key" -subj "/CN=$2" -out "$1.csr"
    printf 'basicConstraints=critical,CA:FALSE\nkeyUsage=critical,digitalSignature\nextendedKeyUsage=serverAuth\nsubjectAltName=DNS:%s\n' "$2" > "$1.ext"
    openssl x509 -req -in "$1.csr" -CA ca.pem -CAkey ca.key -sha256 \
        -not_before "$3" -not_after "$4" -extfile "$1.ext" -out "$1.pem"
    rm "$1.csr" "$1.ext"
}

leaf localhost localhost 20250101000000Z "$far"
leaf expired localhost 20200101000000Z 20210101000000Z
leaf other-host other.example 20250101000000Z "$far"

# A self-signed leaf for localhost, chaining to nothing.
ec_key self-signed.key
openssl req -x509 -new -key self-signed.key -subj "/CN=localhost" -sha256 \
    -not_before 20250101000000Z -not_after "$far" \
    -addext "subjectAltName=DNS:localhost" -addext "extendedKeyUsage=serverAuth" \
    -out self-signed.pem
rm ca.key
