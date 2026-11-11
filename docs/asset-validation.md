# Asset validation boundaries

The scene/streaming hardening now passes local workspace and packaged baseline
checks. The latest material-shape/numeric and composed-transform additions also
pass the full local workspace regression and refreshed client/server package checks.
These checks are not a complete importer/security audit or importer crash sandbox.

## Scene references and work limits

GLB validation now checks primitive/attribute/index references and shapes,
matching vertex counts, paired skinning attributes, morph-target cardinality,
skin joint references and inverse-bind matrix shape, node TRS/matrix exclusivity,
finite bounded transforms, quaternion lengths, texture/sampler/material references,
and animation sampler/channel types and input/output cardinality. Optional WebP
extension references are checked even when a fallback source also exists.

Per-GLB ceilings include 4,096 scene nodes/primitives, 4,000,000 geometry elements,
256 skins/textures/images/animations, 256 joints per skin, 64 morph targets per
primitive and 262,144 animation input keys. Supported node transforms and accessor
bounds have explicit finite numeric bounds. KHR punctual lights have validated
references, colors, cone intervals and bounded source intensity/range.

Node matrices must be affine. Composed world matrices are checked with bounded
double-precision intermediate arithmetic and a 100,000,000 component-magnitude
ceiling, so individually legal scales cannot multiply through a hierarchy into
engine-float overflow. The traversal is iterative, checks every disconnected
root, and rejects cycles/multiple parents; deep ordinary rotated chains remain
valid. This is not a bound on every later skinning/animation arithmetic path.

Across all GLB/VRM members in a package, encoded scene bytes and estimated
expanded accessor arrays are each capped at 256 MiB. These aggregate checks run
before consuming the next scene's binary chunk, preventing hundreds of separately
legal prefabs from multiplying an importer allocation budget. Existing package
decoded-image (256 MiB) and packaged Opus (64 MiB) limits remain separate.

VRM humanoid node maps, expression morph/material/texture-transform references,
spring joints, collider shapes and collider-group references are also checked.
Spring content is bounded to 64 colliders/groups, 256 total joints and 65,536
joint/collider pairs per asset. Duplicate humanoid mappings, joints within one
chain and repeated group references are rejected. Defaults for optional VRM
collider offsets/radii and texture transforms remain supported.

## Material inputs

Core PBR color, roughness/metallic, alpha mode/cutoff, sidedness and texture-info
shapes are checked before import. Known texture transforms require bounded finite
offset/scale pairs, rotation and supported coordinate-set indices. Material and
extension numeric data must be finite with magnitude at most 1,000,000, depth at
most 16, and at most 262,144 visited values across all materials in a GLB. Ordinary
`extras` metadata is not interpreted as engine uniforms and remains outside this
numeric policy (the enclosing GLB/package limits still apply). This prevents
JSON numbers that overflow engine floats from reaching recognized shader/importer
fields. Client normalization also caps retained emission energy at 8 and normal
scale magnitude at 4, clamps base/emission colors to the fixed `[0,1]` family and
substitutes safe defaults for nonfinite color inputs. Original source shaders/
pass chains are not retained.

## Streaming binary content

A streaming reader validates the existing BIN bytes while image checks consume
them, without another BIN-sized allocation, seeking a compressed ZIP member, or
writing a temporary binary spool:

- Mesh indices must be below their referenced primitive's vertex count.
- Skin joint indices must be below the referenced skin's joint count.
- Float accessor values must be finite and have magnitude at most 100,000,000.
- Sparse indices must be in range, strictly increasing and unique.
- Animation input times must be finite, within one day, and strictly increasing
  after sparse overlays are applied.

Scalar checks tolerate interleaved layouts, matrix-column padding, sparse values
and reads splitting scalar bytes. Reused index accessors use the strictest
referencing bound. Explicit base values are checked even if sparse data later
overwrites them. Tiny bounded time buffers (up to 262,144 samples and associated
sparse overlays) reconstruct final animation times; other float/index data is
checked without retaining decoded arrays. Descriptor count is capped at 262,144.

Regression sources cover invalid references/shapes, oversized geometry,
VRM expression/spring escapes, nonfinite values, unsigned indices, single-byte
stream reads, sparse ordering and post-overlay animation-time ordering. These pass
locally, including material and transform-composition cases. Fuzzing, representative
third-party assets, importer peaks/crash isolation and GPU behavior remain separate
acceptance work. Media decoding has a separate helper-process boundary described
in [media-security.md](media-security.md).
