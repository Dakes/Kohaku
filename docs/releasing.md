# Releasing Kohaku

Official images are `dakes/kohaku` on Docker Hub, built by `.github/workflows/release.yml`
when a bare semver tag (`1.2.3`, no `v`) is pushed. Four things keep a leaked or
misused credential from publishing an image:

- **`release` environment:** the Docker Hub token exists only as an environment secret,
  and the publish job waits until the owner approves it.
- **Tag ruleset:** release tags cannot be moved or deleted.
- **Scoped sandbox token:** the token used by agents cannot edit workflows, approve
  deployments or change settings.
- **Immutable tags on Docker Hub:** a published `X.Y.Z` can never be overwritten.

UI names below were checked against GitHub and Docker documentation in September 2026.
Anything that could not be confirmed is marked **(unconfirmed)**.

## One-time setup

### 1. Docker Hub account and repository

1. Sign in at <https://hub.docker.com> to your existing account `dakes` (created in
   2021). If you no longer have access, recover it with Docker's password reset. If the
   account isn't yours, pick another namespace and change `dakes/kohaku` everywhere:
   design §2/§13/§14, README, this guide and the cosign command. Turn on two-factor
   authentication in the account settings.
2. Create the repository: **My Hub** → **Repositories** → **Create repository**.
   - **Namespace:** `dakes`
   - **Repository Name:** `kohaku`
   - **Short description:** `Tiny self-hosted bug report inbox`
   - **Visibility:** **Public**
   - Select **Create**.
3. Make version tags immutable: **My Hub** → **Repositories** → `kohaku` → **Settings**
   → **General** → **Tag mutability settings**.
   - Choose **Specific tags are immutable**.
   - Regex (Go/RE2 syntax): `^[0-9]+\.[0-9]+\.[0-9]+$`
   - Select **Save**.
   - `1.2`, `1` and `latest` stay mutable on purpose, since they move with each release.
     cosign's `sha256-…` signature/referrer tags do not match the regex, so signing
     still works.
   - Immutable tags are a **Beta** feature. Docker's documentation names no plan
     restriction for it. If the option is missing on your plan, say so in the release
     notes; the other safeguards still apply.

### 2. Docker Hub access token

1. Go to <https://app.docker.com>, select your avatar (top right), then **Account
   settings** → **Personal access tokens** → **Generate new token**.
2. Fill in:
   - **Description:** `github-actions kohaku release`
   - **Expiration date:** at most 1 year out. Put a calendar reminder a week before it
     expires.
   - **Access permissions:** the level that can push but **not delete** (Read & Write).
     Never pick the level that includes Delete. Docker's docs name the levels Read,
     Write and Delete; the exact label in the dropdown is **(unconfirmed)**.
3. Select **Generate** and copy the token. It is shown only once.

A personal access token works on **every repository in the `dakes` namespace**, not
only `kohaku`. If that namespace ever holds other images that matter, publish Kohaku
from a dedicated namespace instead. (Docker Hub also offers GitHub OIDC connections,
but only for organizations on a paid Team or Business plan, so they are not used here.)

### 3. GitHub environment `release`

Repository `Dakes/Kohaku` → **Settings** → **Environments** → **New environment**.
Name it `release`, then **Configure environment**.

1. **Required reviewers:** tick it and add yourself (`Dakes`).
   - Leave **Prevent self-review** **off**: you push the tag yourself, and with it on
     you could not approve your own release.
2. Untick **Allow administrators to bypass configured protection rules**, then select
   **Save protection rules**.
3. **Deployment branches and tags:** choose **Selected branches and tags** →
   **Add deployment branch or tag rule**.
   - **Ref type:** **Tag**
   - Pattern: `[0-9]*.[0-9]*.[0-9]*`
   - Select **Add rule**.
   - The pattern uses Ruby `File.fnmatch` syntax. The workflow separately rejects any
     tag that is not exactly `X.Y.Z` or does not equal the `Cargo.toml` version.
4. **Environment secrets** → **Add secret**, twice:
   - `DOCKERHUB_USERNAME` = `dakes`
   - `DOCKERHUB_TOKEN` = the token from step 2
5. Delete any repository-level copies: **Settings** → **Secrets and variables** →
   **Actions** → under **Repository secrets**, remove `DOCKERHUB_USERNAME` and
   `DOCKERHUB_TOKEN` if present.

On GitHub Free, required reviewers and environment secrets are available only for
**public** repositories. Keep the repository public.

A job that uses the environment cannot read its secrets until a required reviewer
approves it. Only the `publish` job references `environment: release`.

### 4. Tag ruleset

**Settings** → **Rules** → **Rulesets** → **New ruleset** → **New tag ruleset**.

1. **Ruleset name:** `release tags`
2. **Enforcement status:** **Active**
3. **Bypass list:** leave empty.
4. **Target tags:** **Add a target** → include by pattern `[0-9]*.[0-9]*.[0-9]*`.
5. **Tag protections:** tick **Restrict updates** and **Restrict deletions**. Leave
   **Restrict creations** off, because you create these tags.
6. Select **Create**.

Rulesets are available for public repositories on GitHub Free. With no bypass actor,
a wrong tag cannot be fixed: release the next patch version instead.

### 5. Token for the dev sandbox (only if it has GitHub access)

Currently not needed: agent sandboxes hold no GitHub credentials, and all pushes come
from the owner's machine. If a sandbox or agent is ever given GitHub access, give it a
fine-grained token that can push code and open PRs, and nothing else.

1. GitHub avatar → **Settings** → **Developer settings** → **Personal access tokens** →
   **Fine-grained tokens** → **Generate new token**.
2. Fill in:
   - **Token name:** `claude-kohaku sandbox`
   - **Expiration:** 90 days. Never choose **No expiration**, even though GitHub
     allows it.
   - **Resource owner:** `Dakes`
   - **Repository access:** **Only select repositories** → `Dakes/Kohaku`
3. **Repository permissions:**
   - **Contents:** Read and write
   - **Pull requests:** Read and write
   - **Metadata:** Read-only. GitHub adds this automatically.
   - Everything else stays at no access, in particular **Workflows**, **Actions**,
     **Environments**, **Deployments**, **Administration** and **Secrets**.
4. Select **Generate token** and copy it.
5. On the **host** (not inside the sandbox), run:

   ```sh
   sbx secret set github --sandbox claude-kohaku -t <token>
   ```

   It takes effect immediately. `-t` leaves the token in your shell history; to avoid
   that, leave out `-t <token>` and paste the token at the prompt.
6. Run `sbx secret ls --service github`. A global `github` secret (for example an
   earlier `gh auth token`) reaches every sandbox, including new ones. If it holds a
   broader token, remove it with `sbx secret rm github`.

Without the Workflows permission, any push that changes `.github/workflows/` is
refused. The token can still push a tag, since tags need Contents write. That would
start a release run, which then waits for your approval. **Only approve a deployment
for a tag you pushed yourself.**

### 6. Private vulnerability reporting

Repository `Dakes/Kohaku` → **Settings** → in the sidebar section **Security and
quality**, **Advanced Security** → next to **Private vulnerability reporting**, select
**Enable**. The README's "report it privately" link only works once this is on.

## Cutting a release

Everything below runs on the host, not in the sandbox.

1. Make sure `main` is green in CI and contains everything for the release.
2. Bump the version:
   - Set `version = "1.2.3"` in `Cargo.toml`.
   - Run `cargo check` to update `Cargo.lock`.
   - Set `V=1.2.3` in the README install snippet.
   - Commit (`Release 1.2.3`), push to `main` and wait for CI.
3. Tag and push:

   ```sh
   git tag -a 1.2.3 -m "1.2.3"
   git push origin 1.2.3
   ```

4. Open the **Actions** tab, then the release run for `1.2.3`. The `build` jobs run
   first. `publish` then shows as waiting for review.
   - Check that the run is for the tag and commit you just pushed.
   - Select **Review deployments**, tick `release`, optionally add a comment, then
     select **Approve and deploy**.
5. When `publish` finishes, check that Docker Hub lists `1.2.3`, `1.2`, `1` and `latest`
   for the same digest, then verify the signature (below).
6. Write the release notes: **Releases** → **Draft a new release** → choose tag `1.2.3`.
   Call out any change to `docker-compose.yml`, `Caddyfile` or `.env.example`, because
   `docker compose pull` never updates those files for users.

If something fails:

- **Before `publish` pushed anything:** fix it on `main` and release the next patch
  version. The tag cannot be moved.
- **`publish` failed before the `1.2.3` tag was applied** (it pushes by digest, signs,
  then tags `1.2`, `1`, `latest` and `1.2.3` last): re-run the job. No tag points at
  an unsigned image in between.
- **After `1.2.3` reached Docker Hub:** that tag is immutable, so release the next
  patch version.

Renew the Docker Hub token (at most yearly) and the sandbox token (every 90 days)
before they expire. A new Docker Hub token only needs `DOCKERHUB_TOKEN` updated in
the `release` environment. For a new sandbox token, run
`sbx secret set github --sandbox claude-kohaku -f -t <token>` on the host (`-f`
overwrites the existing secret), or the prompt form without `-t`.

## Verifying an image

Signatures are keyless (Sigstore, GitHub OIDC). They are made only by the release
workflow running on a bare semver tag. Install [cosign](https://docs.sigstore.dev/)
**3.0 or newer** (2.x cannot find these bundle-format signatures and reports the image
as unsigned) and run:

```sh
cosign verify \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com \
  --certificate-identity-regexp '^https://github\.com/Dakes/Kohaku/\.github/workflows/release\.yml@refs/tags/[0-9]+\.[0-9]+\.[0-9]+$' \
  dakes/kohaku:1.2.3
```

- On success cosign prints the verified signature payload, including the image digest.
  Any other signer, workflow or ref fails.
- The same command works for `dakes/kohaku:1`, which points at a signed release digest.
- To pin exactly what you verified, use `dakes/kohaku@sha256:<digest>` in
  `docker-compose.yml`.
