# Repository instructions

## Scope and project references

Read `README.md` and the relevant plan in `docs/` before implementation.
`docs/requirements.md` preserves the original specification. Current user
instructions determine which milestone to implement; the specification's final
instruction to implement everything does not override stepwise work.

Use zbus directly for iwd; iwdrs is example material only, never a dependency.
Keep the public API independent of iwd and Kameo. Keep the UI independent of
daemon internals. Add dependencies when used, with compatible stable versions
and a committed lockfile. Verify formatting, compilation, and relevant behavior.

For Dioxus changes, load `.agents/skills/dioxus-web/SKILL.md` and follow its official
0.7 documentation links. This path is repository-local and loaded explicitly.

## Test environments

Complete Step 0 in `docs/implementation-plan.md` before application work:
development image, parameterized Ethernet/SSH update workflow, then verified
iwd connection and scoped credential cleanup on the Pi. The workflow has no
Polis SBC-controller/Pico dependency. Consult `docs/testing-plan.md` before
adding hardware commands. Later laptop tests use mocks and a private D-Bus.

Keep development image profiles, overlays, build entry points, and pinned image
inputs in `BuildrootImages`. Reference its checkout/profile from this repo;
do not duplicate image definitions here. Keep SSH/deployment scripts, probes,
and deployment configuration under `dev/`; any Rust helper there uses a separate
workspace. Product crates
and build scripts must not depend on or invoke this suite. Release artifacts,
library packages, and production images exclude it through explicit packaging
boundaries. Keep target-specific settings and secrets in ignored `.local/` files.

## User interaction

At session start, load `/home/broosegoose/.codex/skills/i-have-adhd/SKILL.md`
unless the user has opted out. Keep responses actionable, use numbered steps
for multi-step work, and finish with one small next action when work remains.
