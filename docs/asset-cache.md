# Client asset cache

The official client maintains completed world/avatar ZIPs under `user://cache`.
Only lowercase SHA-256 filenames ending in `.zip` are managed. Package hashes
are checked before cache paths are formed, and cached bytes are still hashed
before import. Pruning never follows symlinks or descends into directories, and
does not remove unrelated filenames or `.part-*` downloads.

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
its HTTP request and removes its own unfinished download.

These are best-effort completed-cache limits, **not a filesystem quota, decoded
memory budget or process sandbox**. Active imports can temporarily exceed the
limit; filesystem failures are surfaced as a cache notice rather than breaking
gameplay. Downloads add temporary disk usage before publication. Crash-orphaned
partial files and cross-process pin coordination remain unfinished. Server-side
immutable asset-version garbage collection is separate and still required.

Headless checks cover byte/file eviction, recency, refcounted pins, malformed
hash/path rejection, symlink/unrelated-file preservation, and a real package
import whose pin remains active through its loaded callback.
