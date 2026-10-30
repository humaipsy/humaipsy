# Privacy & Security

HumaiPsy is a research tool: it can carry participant IDs, full chat transcripts, and
free-text final answers - potentially sensitive human-subjects data. This document
explains what the application does to protect
that data, what it does *not* do, and what an operator (a researcher, an IRB
reviewer, a system administrator) should know before running a real study with it.
It describes the implementation as it exists in the code today.

If you take one thing from this document: **the master-password vault gates access
to the running application; it does not encrypt the data files on disk.** See
[Data at rest](#data-at-rest-what-is-and-isnt-encrypted) below - this is the single
most important thing to understand about HumaiPsy's privacy posture.

## Threat model

HumaiPsy is designed around one primary boundary: **the local machine it runs on**.
Everything it does — inference, recording, storage — happens on that machine unless
an operator explicitly configures a remote Ollama host. It is not designed to defend
against a compromised OS, a determined attacker with physical or administrator
access to the machine, or a malicious remote Ollama host. Its actual protections are:

1. **Keeping data from leaving the machine by default** (network boundary).
2. **Gating who can look at the running application** (the master-password vault).
3. **Not making the on-disk data files easier to find/read than necessary** (file
   permissions, path sanitization).

Each is covered below, along with its real limits.

## Network boundary

### Outbound: the Ollama connection

The only network call HumaiPsy makes by default is to whatever `OLLAMA_HOST` is
configured to (default `http://127.0.0.1:11434`). **With air-gap mode off — the
default — there is no restriction on what host that can be.** If an operator points
`OLLAMA_HOST` at a remote server, full experiment transcripts (every message, every
model response) are sent to that host.

### Air-gap mode

Setting `HUMAIPSY_AIR_GAP=1` blocks all outbound HTTP except to a loopback Ollama host
(`127.0.0.1`, `::1`, or `localhost`); `HUMAIPSY_ALLOW_LOCAL_OLLAMA=0` blocks the
loopback exception too, i.e. no outbound HTTP at all. This is enforced centrally —
every HTTP call site in `humaipsy-core::ollama` goes through
`guard_ollama_request` before a request is sent - and is covered by unit tests
(`crates/humaipsy-core/src/ollama/mod.rs::tests::guard_ollama_request_enforces_air_gap_policy`)
that assert the exact enforcement matrix: air-gap off → anything allowed; air-gap on
with local Ollama allowed → only loopback hosts pass; air-gap on with local Ollama
also disabled → nothing passes.


### Inbound: the local web dashboard

`crates/humaipsy-web` is a small local dashboard, spawned automatically inside the
desktop app's own process, that serves a read-only view of the current experiment
over HTTP:

- **Bound to `127.0.0.1` by default.** `HUMAIPSY_WEB_BIND` can widen this (e.g. to
  `0.0.0.0:8787` for LAN access) — doing so is an explicit, deliberate choice an
  operator has to make; it is never the default. `HUMAIPSY_WEB_DISABLED=1` turns the
  server off entirely.
- **No write path exists.** Every route is a `GET`; nothing served by `humaipsy-web`
  can start a trial, send a message, or otherwise mutate application state. There is
  currently no code path by which the dashboard could be used to control the
  experiment, by accident or otherwise.
- **Gated by the same master-password vault as the desktop app.** This wasn't true
  in the first version of the dashboard and was fixed once identified: the desktop
  app's vault-lock state (`MasterVault::is_unlocked()`) is mirrored into a shared
  flag (`humaipsy_web::SharedVaultLock`) that `HumaiPsyApp::update` syncs every frame. The
  dashboard's `/api/status` and `/api/messages` routes check that flag *before*
  touching any state or the database, and return empty/blanked responses whenever
  it's `false`. Locking the desktop app (the Settings "Lock" button) now stops the
  dashboard from serving transcript content within one frame, the same way it blanks
  the desktop UI — the dashboard being open is not a way to see data the lock screen
  is otherwise hiding.
- **No authentication of its own, and no HTTPS.** The vault-lock gate controls
  *whether* content is served, but there is no login, session, token, or TLS on the
  dashboard's HTTP surface itself. Anyone who can reach the bound address while the
  app is unlocked can view the transcript — on a single-user machine with the
  loopback-only default, that's "any process running as any user on that machine";
  if `HUMAIPSY_WEB_BIND` is widened to a LAN address, it's "anyone on that network."
  Treat widening the bind address as equivalent to deciding who gets to watch the
  session live, because that's exactly what it does.
- The dashboard's static assets (`crates/humaipsy-web/web/*.html`/`.js`/`.css`) are
  embedded into the binary at compile time and reference no external resources — no
  CDN scripts, no remote fonts, no third-party analytics. Opening the dashboard page
  makes no network calls other than to the desktop app's own local API.

## Application access: the master-password vault

- **Release builds always enforce the master-password lock**; the app boots straight
  to an unlock screen and nothing else renders until a correct password is entered.
  Debug builds skip this by default for faster local iteration
  (`HUMAIPSY_ENABLE_PASSWORD_LOCK=1` to enforce it in a debug build anyway,
  `HUMAIPSY_SKIP_VAULT=1` to force-skip even then).
- The password is hashed with **Argon2id** (memory-hard, GPU/ASIC-resistant),
  default parameters 64 MiB memory cost, 3 iterations, parallelism up to the
  machine's core count (capped at 4) — tunable via `HUMAIPSY_ARGON2_M_KIB`/`_T`/`_P` if
  a deployment needs different cost/latency tradeoffs. The stored artifact
  (`vault/.master_hash`) is a one-line PHC-format hash string, never the password
  itself, and is regenerated via `cargo run --bin gen_master_hash`.
- **Repeated failed unlock attempts are throttled** with exponential backoff
  (`1, 2, 4, 8, 16, 30` seconds, capped at 30s) — a basic but real deterrent against
  automated password guessing against the unlock screen itself. This does not
  protect the underlying `.master_hash` file if it's copied off the machine and
  attacked offline; Argon2id's memory-hardness is what raises the cost of *that*.

## Data at rest: what is and isn't encrypted

This is the section to read carefully. HumaiPsy writes several files during and after
an experiment, all of them **plaintext**, all of them under the same "protection" as
any other file on the machine:

| Artifact | Location | Encrypted? |
|---|---|---|
| Experiment CSV | `metrics/<name>.csv` | No — plaintext |
| Event ledger | `metrics/runs/<experiment_id>/<run_id>/events.jsonl`, `summary.json` | No — plaintext (payload hashes are SHA-256 for integrity/audit purposes, not confidentiality) |
| Chat transcript | `<metrics dir>/chat.sqlite` | No — plaintext SQLite |
| Master password hash | `vault/.master_hash` | N/A — this is a one-way Argon2id hash, not the password, and not experiment data |

**The master-password vault gates the application's UI. It does not encrypt any of
the files above.** Once a participant's session is recorded, that transcript sits on
disk in a plain CSV and a plain SQLite database, readable by anything with
filesystem access to those paths — a different local user account (if file
permissions allow, see below), a backup tool, a forensic image, another process
running as the same user, or a person with a screwdriver and the disk. Locking the
HumaiPsy UI does not change any of that.

What file-level protection *does* exist:

- On **Unix**, `vault/.master_hash` and `chat.sqlite` are both explicitly set to
  `0600` (owner read/write only) at creation time — the default umask would
  otherwise leave them world-readable (`0644`), letting any other local user account
  read the password hash or the chat log. This is enforced in code
  (`vault.rs::restrict_to_owner`, `store.rs`'s equivalent), not left to the OS default.
- On **Windows**, this is a no-op — per-user profile directory ACLs already restrict
  cross-account access to files under a user's own profile, so there's no additional
  step HumaiPsy takes (or needs to take) there.
- Neither mechanism protects against another process **running as the same OS
  user**, which can read anything that user can read regardless of the `0600`/ACL
  setting. That's a fundamentally different, and much stronger, threat model than
  file permissions can address.

**If you are handling real human-subjects data and need protection against someone
with access to the machine or its disk, that protection has to come from outside
HumaiPsy** — full-disk encryption (BitLocker, FileVault, LUKS), an encrypted volume for
the `metrics/` output directory, or physical security of the machine. This isn't a
gap HumaiPsy silently hides; it's a boundary the application doesn't claim to cross.

## Filename / path safety

Experiment CSV filenames are derived from free-text UI fields (the design's
"name"/"CSV name"). Those fields are run through `sanitize_file_stem`
(`recording/experiment_csv.rs`) before ever being joined into a filesystem path —
stripping `/`, `\`, `:`, and control characters, and trimming leading `.` — so a
design named `../../etc/passwd` or `C:\Windows\System32\config` can't escape the
configured output directory. Covered by tests
(`csv_path_rejects_parent_dir_traversal`, `csv_path_rejects_unix_absolute_path`,
`csv_path_rejects_windows_absolute_path`).

## Known limitations, stated plainly

- Data at rest is not encrypted (see above) — this is the big one.
- The web dashboard, once the app is unlocked, has no authentication of its own; the
  loopback-only default is the actual protection, not a login screen.
- Air-gap mode is opt-in, not the default; a fresh install with no configuration
  sends transcripts to whatever `OLLAMA_HOST` is set to.
- Debug builds skip the master-password lock unless explicitly told not to — don't
  distribute or use a debug build for anything you wouldn't want unlocked-by-default.
- None of the above is enforced or checked by an external audit — this document
  describes the implementation as read from the source, not a certification.

## Recommendations for a real study

1. Use a **release build** (the password lock can't be accidentally skipped the way
   it can in debug), and set a real master password before the first session.
2. Set `HUMAIPSY_AIR_GAP=1` with Ollama running locally, unless you have a specific,
   trusted reason to use a remote host.
3. Leave the web dashboard on its loopback-only default, or set
   `HUMAIPSY_WEB_DISABLED=1` if you don't need it — don't widen `HUMAIPSY_WEB_BIND` on a
   shared network unless you specifically want the session visible to that network.
4. Enable full-disk encryption on the machine running HumaiPsy — this is the actual
   answer to "is the data protected at rest," not anything in this application.
5. Treat the `metrics/` output directory (CSV, ledger, SQLite) the same way you'd
   treat any other folder of unencrypted participant data on a shared or portable
   machine: control who has an account on it, and don't sync it to a location you
   haven't separately vetted for your IRB/ethics requirements.

For how these mechanisms fit into the rest of the application's architecture, see
[ARCHITECTURE.md](ARCHITECTURE.md); for contributor-facing rules around this code
(what not to change casually, where new privacy-relevant code should go), see
[AGENTS.md](AGENTS.md).
