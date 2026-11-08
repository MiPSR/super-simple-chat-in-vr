# Client asset cache

The official client maintains completed world/avatar ZIPs under `user://cache`.
Completed-cache management uses lowercase SHA-256 filenames ending in `.zip`. Package hashes
are checked before cache paths are formed, and cached bytes are still hashed
before import. Pruning never follows symlinks or descends into directories, and
does not remove unrelated filenames.

The default completed-cache limits are **1 GiB and 128 ZIPs**. Pruning runs before
import and after loaded listeners have synchronously consumed scripts/audio.
Oldest unprotected entries are evicted first: accesses are tracked in memory for
the current process; untouched entries use their download modification time and
filename as deterministic tie-breakers. Recency is not written to disk on each
access. The eviction candidate list stays bounded even for a large legacy cache.

Import pins are refcounted and cover hashing, worker import, scene generation and
loaded callbacks. Import completion, worker-start failure and loader teardown
release pins. Already loaded scenes, prefabs, Lua and packaged Opus retain their
resources in memory; evicting a completed ZIP does not stop current gameplay.
Selecting an evicted version downloads it again. Normal loader teardown cancels
its HTTP request and removes its own unfinished download. New partials use
`<sha256>.zip.part-<pid>-<128-bit-random-hex>` names, preventing simultaneous
loaders/processes from sharing a download path. Startup and ordinary pruning
remove only strictly matching partials whose process no longer exists in Linux
procfs. A live owner is preserved; PID reuse can conservatively retain an orphan.
Without usable procfs, partial cleanup is skipped. Shared caches across different
PID namespaces are not supported. Legacy partials without owner/nonces remain
untouched because their owner cannot be determined safely.

These are best-effort completed-cache limits, **not a filesystem quota, decoded
memory budget or process sandbox**. Active imports can temporarily exceed the
limit; filesystem failures are surfaced as a cache notice rather than breaking
gameplay. Downloads add temporary disk usage before publication. Cross-process
completed-ZIP pin coordination and legacy partial cleanup remain unfinished. Server-side
immutable asset-version garbage collection is separate and still required.

Headless checks cover byte/file eviction, recency, refcounted pins, malformed
hash/path rejection, dead-process partial cleanup, live-owner preservation,
symlink/unrelated-file preservation, and a real package
import whose pin remains active through its loaded callback.
