# Server immutable-version storage

The server owns its data directory exclusively. At startup it takes a nonblocking
Linux advisory lock on `.ssc.lock` before opening SQLite; a second server using
that directory fails instead of racing publication or garbage collection. The
descriptor stays open for the server's lifetime. The small lock file intentionally
remains after exit: **do not unlink it while a server is running**. Stop older
server processes before upgrading. External writers bypassing this lock are not
supported.

## References and reclamation

The collector preserves every version referenced by:

- Any catalog asset, including disabled assets.
- A live world's immutable world version.
- Any participant's equipped immutable avatar version.
- An unexpired, unconsumed game grant.

Publication, grant creation/consumption, avatar selection, and opening an
authorized download use the same lifecycle gate as collection. Thus a pending
grant's reference transfers to an admitted instance without an unpinned window.
Existing instances can still download their exact world/avatar versions after
catalog replacement or deletion, subject to the normal visibility/block rules.
An already opened Linux download descriptor remains valid even if the last
reference disappears and its pathname is later reclaimed.

Only regular files named `<64 lowercase hex digits>.zip` directly inside the
dedicated `assets` directory are candidates. Collection does not descend into
directories or follow symlinks, and ignores unrelated filenames. An unreferenced
file becomes eligible when its modification time is at least **24 hours old**.
This is age since publication/file modification, not a fresh 24-hour grace period
after every disconnect. Future timestamps are retained conservatively. There is
no permanent historical-version archive guarantee.

The worker runs at startup and then once per minute. A pass examines at most 64
directory entries and 64 metadata rows, with a 25-ms between-entry processing
budget. This is not a deadline for a filesystem operation. Its in-memory iterator
and lexical metadata cursor let later entries progress without loading the full
directory/catalog or writing per-access metadata. A catalog-version SQLite index
keeps reference checks bounded to indexed lookups.

Obsolete metadata is removed before unlinking its file. If unlink fails or the
process stops between those operations, a subsequent directory pass can reclaim
the orphan. Missing-file metadata is removed only when it has no catalog/live/
grant references. Current catalog rows are never silently deleted to repair a
missing file. Failures are reported in bounded pass statistics/server logs.

## Administration

`POST /api/v1/admin/assets/gc` requires the administrator secret and runs one
bounded pass, returning `scanned`, `files`, `rows`, `bytes`, and `errors`. It does
not expose a force-delete or reduced-age option. It starts a fresh scan; it is
not a promise to drain a large directory in one request. The background worker
retains scan progress across its passes. Reclaimed versions return HTTP 404 to
an otherwise authorized historical-version download request.

## Upload staging and cancellation

The server admits at most **two receiving/validating uploads** and reserves at
most **2,000,000,000 staging bytes** across them. Each request reserves its
account's per-package ceiling, capped at the existing 1,000,000,000-byte HTTP
package limit, before creating a staging file. Excess work gets HTTP 429 rather
than an unbounded waiting queue. Administrator health includes active upload and
reserved-byte counts. Per-account size/slot quotas still apply independently.

Receiving has a 30-second idle timeout and a 30-minute total deadline checked
between chunks. This is not a deadline for filesystem operations or synchronous
native validation. Validation/publication runs on a blocking worker; ownership
of the staging guard and reservation transfers into that worker. Cancelling the
HTTP waiter therefore cannot free its reservation while validation is still
running. Publication rechecks current account enablement and size/slot quotas.

RAII cleanup removes unfinished stages on parse/validation failure or disconnect.
A successful rename does not cause cleanup to remove the published ZIP. On
startup, after taking the exclusive data-directory lock and before accepting
HTTP, the server reclaims regular `upload-<64 lowercase hex digits>.tmp` files
directly in the data directory. It neither follows symlinks nor descends into
directories or removes unrelated filenames. Failure to complete recovery stops
startup instead of admitting uploads over unknown abandoned staging usage.

These are bounded admission/reclamation rules, not a filesystem quota, backup
strategy, cgroup/native-decoder memory isolation, or complete aggregate resource
budget. Filesystem errors and retiring I/O can delay reclamation.
