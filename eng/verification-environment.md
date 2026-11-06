# Linux verification environment

The Linux main gate uses a prebuilt environment. The environment is maintained
separately from source verification. The lightweight PR gate and native package
jobs keep their own environments.

## Environment ownership

[The Dockerfile](../.github/ci/Dockerfile) owns the native dependency list and
build-tool versions. It installs LibRaw and software Vulkan. Rust tools live
outside `HOME` so GitHub Actions can override the home directory without changing
the toolchain. The image records installed packages and tool versions under
`/opt/verification`.

[The environment workflow](../.github/workflows/verification-image.yml) builds a
candidate, verifies a CPU Vulkan adapter, and runs the complete `just ci` gate
through `just verify` with an empty Cargo cache. It publishes the image to GHCR
only after these checks pass. Repository pushes that change the environment
files trigger validation; fork pull requests cannot publish images. Maintainers
can also dispatch the workflow manually.

## Promotion and rollback

1. Change the Dockerfile to update dependencies or tools. Rebuild periodically
   to incorporate distribution updates.
2. Wait for the environment workflow to pass. Read its verification artifact
   and the published image digest in the workflow summary.
3. Update the `container.image` digest in
   [Main CI](../.github/workflows/ci.yml) through a pull request.
4. Confirm that a fresh hosted runner can pull the image and complete the gate.
   Check both cold and restored Cargo caches when changing the execution model.

Keep the previous image available. Roll back by restoring its digest. An
unsuccessful environment update does not change the normal verification image.
The image digest fixes installed contents; rebuilding the same Dockerfile can
pick up newer distribution packages. Toolchain updates are explicit.

## Normal verification

Normal verification pulls the fixed digest, restores an optional Cargo cache,
runs `just verify`, and preserves the result. It does not install system packages
or build LibRaw. Cache misses cause compilation; they never skip tests or select
another environment. Use an explicit Bash shell for container job steps.

The image package must grant this repository read access before promotion.
Only the environment publisher needs package write access. Image pull failures
remain infrastructure errors; do not add an automatic host-install fallback.

Software Vulkan tests do not require host GPU passthrough. A successful image
check does not establish hardware GPU parity or package compatibility on other
operating systems. Native packaging has separate dependencies and acceptance.
