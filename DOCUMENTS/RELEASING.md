# Releasing OSflux

## Continuous integration

`.github/workflows/ci.yml` runs on every pull request to `main`, on every push to `main`, and by hand from the Actions tab.

| Job | Runs |
|---|---|
| Windows | clippy with warnings as errors, all tests, and the release build of `osflux.exe`, kept as a workflow artifact for 14 days |
| Linux | the formatting check, clippy, all tests (with mtools, dosfstools, wimtools and xmllint installed, so the independent cross-checks run instead of being skipped), and the Windows build check from Linux |

Rust is pinned in `rust-toolchain.toml`, so CI and every developer machine use the same compiler and see the same lints. rustup installs that version the first time `cargo` runs in the repository. Moving to a newer Rust is a deliberate change: update the version there, fix any new lints, and open a pull request.

**If runs fail before any job starts** (a "startup failure", as happened when CI was first tried on this repository), the cause is a GitHub setting, not the code:
- Under Settings → Actions → General, Actions must be allowed, along with the actions the workflows use: `actions/*` and `sslcom/esigner-codesign`.
- For a private repository, the account needs Actions minutes or a payment method. Windows runners count each minute twice, and a billing problem blocks every run.

## Cutting a release

1. **Update `CHANGELOG.md`.** Rename `## [Unreleased]` to `## [0.1.0] - 2026-12-01` (the new version and today's date), then add a fresh, empty `## [Unreleased]` above it.
2. **Set the version** in `[workspace.package]` in `Cargo.toml`, run `cargo build` so `Cargo.lock` follows, and merge both through a pull request.
3. **Tag the merged commit and push the tag:**
   ```powershell
   git checkout main
   git pull
   git tag -a v0.1.0 -m "OSflux 0.1.0"
   git push origin v0.1.0
   ```
4. **Watch `.github/workflows/release.yml` run:**
   - It checks that the tag matches `Cargo.toml`.
   - It runs the tests, builds `osflux.exe` and signs it, if signing is set up.
   - It packages `osflux-<version>-windows-x64.zip` (with the README and changelog) and `SHA256SUMS.txt`.
   - It publishes a GitHub release whose notes are the version's changelog section.
   - A version with a hyphen, such as `0.2.0-beta.1`, is published as a pre-release.

To try the release build without publishing, run the Release workflow by hand from the Actions tab. It does everything except publish, and keeps the package as a workflow artifact.

## Code signing

Windows SmartScreen warns before running unsigned programs from the internet, and a tool that needs Administrator and erases drives should carry its publisher's name. The release workflow signs `osflux.exe` as soon as a certificate is set up, and builds unsigned until then. The release notes say which.

Since 2023, code signing keys must live in certified hardware: a USB token, a hardware security module (HSM), or a cloud signing service. A token cannot be plugged into GitHub's runners, so signing in CI needs one of the two cloud routes the workflow supports.

### Turning it on

Settings → Environments → `release` (created by the first release run, or create it by hand). Add the variable `SIGNING_PROVIDER` and the secrets for one route there. You can also add yourself as a required reviewer, so each release waits for your approval before the signing secrets are used.

| `SIGNING_PROVIDER` | Secrets |
|---|---|
| `ssl-com` (SSL.com eSigner) | `ES_USERNAME`, `ES_PASSWORD`, `ES_CREDENTIAL_ID`, `ES_TOTP_SECRET`, from SSL.com's eSigner automation setup |
| `azure-key-vault` (a certificate from any CA, kept in Azure Key Vault Premium) | `AZURE_KEY_VAULT_URL`, `AZURE_KEY_VAULT_CERTIFICATE` (the certificate's name), and `AZURE_CLIENT_ID`, `AZURE_CLIENT_SECRET`, `AZURE_TENANT_ID` (an app registration allowed to sign with it). Optional variable: `SIGNING_TIMESTAMP_URL` (default `http://timestamp.digicert.com`). |

After signing, the workflow checks the signature and its timestamp with `signtool` and fails the release if either is missing.

The signing steps follow each provider's documented setup but cannot be tried until a certificate exists. The first signed release should be a hand-run test build, and its `osflux.exe` checked under Properties → Digital Signatures.

### Options for a Ghana-registered company (checked November 2026)

| Option | Approximate yearly cost | Eligible | Fits CI |
|---|---|---|---|
| **SSL.com OV certificate with eSigner cloud signing** | Certificate about $129 a year, plus eSigner about $15 a month: about $310 a year in total | Yes. SSL.com validates organisations worldwide. | Yes: the `ssl-com` route |
| **OV certificate from Sectigo, DigiCert or GlobalSign, issued into Azure Key Vault** | Certificate about $220–$550 a year (Key Vault versions from about $400), plus Azure Key Vault Premium, a few dollars a month | Yes. These CAs validate organisations worldwide. | Yes: the `azure-key-vault` route |
| **Microsoft Artifact Signing** (formerly Trusted Signing) | About $10 a month | **No.** Public signing is limited to organisations in the USA, Canada, the EU and the UK, with reports of a few more countries; Ghana is not among them. | Would fit, if eligibility widens |
| EV certificate | $300–$700 a year | Yes | Same routes as OV |

**Recommendation:** an OV certificate. Microsoft removed EV certificates' SmartScreen advantage in 2024, so OV and EV now build reputation the same way, as signed downloads accumulate. SSL.com with eSigner is the cheapest route that works in CI.

**What a CA will typically ask a Ghanaian company for**, as part of organisation validation:
- The legal name and registration number at the Office of the Registrar of Companies, with a certificate of incorporation.
- A business address, and a phone number that the CA can verify in an independent directory. A D-U-N-S number from Dun & Bradstreet is the usual way. It is free, but can take a few weeks to get.
- Where the CA cannot check those records online, a letter from a lawyer or accountant confirming them.
- Confirmation that the person ordering may act for the company.

The certificate shows the company's legal name exactly as registered. Ask the CA to confirm the documents it needs before paying.
